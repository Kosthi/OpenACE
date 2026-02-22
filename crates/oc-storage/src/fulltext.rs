use std::path::Path;
use std::time::{Duration, Instant};

use oc_core::{ChunkId, CodeChunk, CodeSymbol, Language, SymbolId};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, QueryParser, TermQuery};
use tantivy::schema::*;
use tantivy::tokenizer::{LowerCaser, TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

use crate::error::StorageError;

const CODE_TOKENIZER: &str = "code";
const CONTENT_MAX_BYTES: usize = 10_240;
const BATCH_COUNT_THRESHOLD: usize = 500;
const BATCH_TIME_THRESHOLD: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Code-aware tokenizer
// ---------------------------------------------------------------------------

/// Check if a character is a CJK Unified Ideograph.
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
    )
}

/// Decode a CJK character at the given byte position.
/// Returns the char and its UTF-8 byte length, or None if not CJK.
fn decode_cjk_at(text: &[u8], pos: usize) -> Option<(char, usize)> {
    if pos >= text.len() {
        return None;
    }
    let s = std::str::from_utf8(&text[pos..]).ok()?;
    let ch = s.chars().next()?;
    if is_cjk(ch) {
        Some((ch, ch.len_utf8()))
    } else {
        None
    }
}

/// Return the byte length of a UTF-8 character from its first byte.
fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1, // invalid, advance 1 byte
    }
}

/// Splits identifiers on camelCase, PascalCase, snake_case, and digit boundaries.
///
/// Equivalent to the regex `[A-Z]?[a-z]+|[A-Z]+(?=[A-Z][a-z]|\d|\b)|[A-Z]+|[0-9]+`
/// but implemented as a state machine since Rust's `regex` crate lacks lookahead.
///
/// Examples:
/// - `HTMLParser`      → `HTML`, `Parser`
/// - `parseXMLStream`  → `parse`, `XML`, `Stream`
/// - `user_service`    → `user`, `service`
/// - `__init__`        → `init`
/// - `i18n`            → `i`, `18`, `n`
#[derive(Clone)]
struct CodeTokenizer {
    token: Token,
}

impl CodeTokenizer {
    fn new() -> Self {
        Self {
            token: Token::default(),
        }
    }
}

impl Tokenizer for CodeTokenizer {
    type TokenStream<'a> = CodeTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        self.token.reset();
        CodeTokenStream {
            text: text.as_bytes(),
            pos: 0,
            token: &mut self.token,
            pending_cjk_offset: None,
        }
    }
}

struct CodeTokenStream<'a> {
    text: &'a [u8],
    pos: usize,
    token: &'a mut Token,
    /// When inside a CJK run, stores the byte offset of the second char
    /// of the current bigram (so the next call re-uses it as the first char).
    pending_cjk_offset: Option<usize>,
}

impl<'a> TokenStream for CodeTokenStream<'a> {
    fn advance(&mut self) -> bool {
        let len = self.text.len();

        // If we have a pending CJK bigram continuation, resume from the second
        // char of the previous bigram (it becomes the first char of the next).
        if let Some(second_offset) = self.pending_cjk_offset.take() {
            self.pos = second_offset;

            // Try to form next bigram starting from second_offset
            if let Some((ch1, len1)) = decode_cjk_at(self.text, self.pos) {
                let next_pos = self.pos + len1;
                if let Some((ch2, len2)) = decode_cjk_at(self.text, next_pos) {
                    // Emit bigram ch1+ch2, set pending to ch2's offset
                    let start = self.pos;
                    self.pending_cjk_offset = Some(next_pos);
                    self.pos = next_pos + len2;

                    self.token.text.clear();
                    self.token.text.push(ch1);
                    self.token.text.push(ch2);
                    self.token.offset_from = start;
                    self.token.offset_to = self.pos;
                    self.token.position = self.token.position.wrapping_add(1);
                    return true;
                } else {
                    // Single trailing CJK char — no bigram possible.
                    // Don't emit it as unigram since it was already part of
                    // the previous bigram. Fall through to normal scanning.
                    self.pos = next_pos;
                }
            }
            // pending offset pointed to non-CJK; fall through to normal scan
        }

        // Skip non-alphanumeric, non-CJK characters
        while self.pos < len {
            let b = self.text[self.pos];
            if b.is_ascii_alphanumeric() {
                break; // start ASCII token
            }
            if !b.is_ascii() {
                if decode_cjk_at(self.text, self.pos).is_some() {
                    break; // start CJK bigram
                }
                // Non-CJK non-ASCII: skip the full UTF-8 character
                self.pos += utf8_char_len(b);
                continue;
            }
            self.pos += 1; // ASCII non-alphanumeric (underscore, punctuation, etc.)
        }

        if self.pos >= len {
            return false;
        }

        // --- CJK bigram branch ---
        if !self.text[self.pos].is_ascii() {
            if let Some((ch1, len1)) = decode_cjk_at(self.text, self.pos) {
                let start = self.pos;
                let next_pos = self.pos + len1;
                if let Some((ch2, len2)) = decode_cjk_at(self.text, next_pos) {
                    // Emit bigram ch1+ch2
                    self.pending_cjk_offset = Some(next_pos);
                    self.pos = next_pos + len2;

                    self.token.text.clear();
                    self.token.text.push(ch1);
                    self.token.text.push(ch2);
                    self.token.offset_from = start;
                    self.token.offset_to = self.pos;
                    self.token.position = self.token.position.wrapping_add(1);
                    return true;
                } else {
                    // Single CJK char at end or before non-CJK: emit as unigram
                    self.pos = next_pos;

                    self.token.text.clear();
                    self.token.text.push(ch1);
                    self.token.offset_from = start;
                    self.token.offset_to = self.pos;
                    self.token.position = self.token.position.wrapping_add(1);
                    return true;
                }
            }
        }

        // --- ASCII token branches (unchanged) ---
        let start = self.pos;
        let first = self.text[start];
        self.pos += 1;

        if first.is_ascii_uppercase() {
            if self.pos < len && self.text[self.pos].is_ascii_lowercase() {
                // Uppercase + lowercase: PascalCase word like "Parser", "My"
                while self.pos < len && self.text[self.pos].is_ascii_lowercase() {
                    self.pos += 1;
                }
            } else {
                // Uppercase run like "HTML" in "HTMLParser" or standalone "HTTP"
                while self.pos < len && self.text[self.pos].is_ascii_uppercase() {
                    // If next char after this uppercase is lowercase, stop here
                    // so the current uppercase starts the next PascalCase word.
                    if self.pos + 1 < len && self.text[self.pos + 1].is_ascii_lowercase() {
                        break;
                    }
                    self.pos += 1;
                }
            }
        } else if first.is_ascii_lowercase() {
            while self.pos < len && self.text[self.pos].is_ascii_lowercase() {
                self.pos += 1;
            }
        } else if first.is_ascii_digit() {
            while self.pos < len && self.text[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }

        // Safety: input is ASCII alphanumeric so valid UTF-8
        let text = std::str::from_utf8(&self.text[start..self.pos]).unwrap_or("");
        self.token.text.clear();
        self.token.text.push_str(text);
        self.token.offset_from = start;
        self.token.offset_to = self.pos;
        self.token.position = self.token.position.wrapping_add(1);

        true
    }

    fn token(&self) -> &Token {
        self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        self.token
    }
}

// ---------------------------------------------------------------------------
// Full-text store
// ---------------------------------------------------------------------------

/// A single BM25 search hit.
#[derive(Debug, Clone)]
pub struct FullTextHit {
    pub symbol_id: SymbolId,
    pub score: f32,
}

/// A single BM25 chunk search hit.
#[derive(Debug, Clone)]
pub struct FullTextChunkHit {
    pub chunk_id: ChunkId,
    pub file_path: String,
    pub score: f32,
}

/// Full-text search index backed by Tantivy.
///
/// Uses a code-aware tokenizer that splits camelCase, PascalCase, snake_case,
/// and UPPER_CASE identifiers into individual tokens with lowercasing.
///
/// Batched commit strategy: commits on 500 documents or 500ms elapsed,
/// whichever comes first. Forced commit on drop.
pub struct FullTextStore {
    index: Index,
    reader: IndexReader,
    writer: IndexWriter,
    f_symbol_id: Field,
    f_name: Field,
    f_qualified_name: Field,
    f_content: Field,
    f_file_path: Field,
    f_language: Field,
    f_doc_type: Field,
    pending_count: usize,
    last_commit: Instant,
}

fn build_schema() -> (Schema, Field, Field, Field, Field, Field, Field, Field) {
    let mut builder = Schema::builder();

    let symbol_id = builder.add_text_field("symbol_id", STRING | STORED);

    let code_text = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(CODE_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let code_text_unstored = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(CODE_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );

    let name = builder.add_text_field("name", code_text.clone());
    let qualified_name = builder.add_text_field("qualified_name", code_text);
    let content = builder.add_text_field("content", code_text_unstored);
    let file_path = builder.add_text_field("file_path", STRING | STORED);
    let language = builder.add_text_field("language", STRING | STORED);
    let doc_type = builder.add_text_field("doc_type", STRING);

    let schema = builder.build();
    (schema, symbol_id, name, qualified_name, content, file_path, language, doc_type)
}

fn register_code_tokenizer(index: &Index) {
    let analyzer = TextAnalyzer::builder(CodeTokenizer::new())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register(CODE_TOKENIZER, analyzer);
}

impl FullTextStore {
    /// Open or create a full-text index at the given directory path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(path)?;

        let (schema, f_symbol_id, f_name, f_qualified_name, f_content, f_file_path, f_language, f_doc_type) =
            build_schema();

        let index = Index::open_in_dir(path)
            .or_else(|_| Index::create_in_dir(path, schema.clone()))?;

        register_code_tokenizer(&index);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        let writer = index.writer(50_000_000)?;

        Ok(Self {
            index,
            reader,
            writer,
            f_symbol_id,
            f_name,
            f_qualified_name,
            f_content,
            f_file_path,
            f_language,
            f_doc_type,
            pending_count: 0,
            last_commit: Instant::now(),
        })
    }

    /// Create an in-memory full-text index (for testing).
    pub fn create_in_ram() -> Result<Self, StorageError> {
        let (schema, f_symbol_id, f_name, f_qualified_name, f_content, f_file_path, f_language, f_doc_type) =
            build_schema();

        let index = Index::create_in_ram(schema);
        register_code_tokenizer(&index);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        let writer = index.writer(15_000_000)?;

        Ok(Self {
            index,
            reader,
            writer,
            f_symbol_id,
            f_name,
            f_qualified_name,
            f_content,
            f_file_path,
            f_language,
            f_doc_type,
            pending_count: 0,
            last_commit: Instant::now(),
        })
    }

    /// Delete all documents from the index.
    ///
    /// Used before a full reindex to ensure no stale documents from deleted
    /// files persist. Commits immediately and refreshes the reader.
    pub fn clear(&mut self) -> Result<(), StorageError> {
        self.writer.delete_all_documents()?;
        self.writer.commit()?;
        self.reader.reload()?;
        self.pending_count = 0;
        self.last_commit = Instant::now();
        Ok(())
    }

    /// Add a symbol document to the index.
    ///
    /// `body` is the optional source text of the symbol body, truncated to 10KB.
    pub fn add_document(
        &mut self,
        symbol: &CodeSymbol,
        body: Option<&str>,
    ) -> Result<(), StorageError> {
        let id_hex = format!("{}", symbol.id);
        let body_content = body.map(|b| oc_core::truncate_utf8_bytes(b, CONTENT_MAX_BYTES)).unwrap_or("");

        // Tokenize file path segments so BM25 can match on directory/file names
        let path_tokens: String = symbol
            .file_path
            .to_string_lossy()
            .split(|c: char| c == '/' || c == '\\' || c == '.' || c == '_' || c == '-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        // Prepend doc_comment so BM25 can match on documentation terms
        let doc_comment = symbol.doc_comment.as_deref().unwrap_or("");
        let content = format!("{} {} {}", path_tokens, doc_comment, body_content);

        self.writer.add_document(doc!(
            self.f_symbol_id => id_hex,
            self.f_name => symbol.name.as_str(),
            self.f_qualified_name => symbol.qualified_name.as_str(),
            self.f_content => content.as_str(),
            self.f_file_path => symbol.file_path.to_string_lossy().as_ref(),
            self.f_language => symbol.language.name(),
            self.f_doc_type => "symbol",
        ))?;

        self.pending_count += 1;
        self.maybe_commit()?;
        Ok(())
    }

    /// Delete all documents matching the given symbol ID.
    pub fn delete_document(&mut self, symbol_id: SymbolId) -> Result<(), StorageError> {
        let hex = format!("{}", symbol_id);
        self.writer
            .delete_term(Term::from_field_text(self.f_symbol_id, &hex));
        self.pending_count += 1;
        self.maybe_commit()?;
        Ok(())
    }

    /// Search for symbols using BM25 ranking.
    ///
    /// The query is tokenized with the code-aware tokenizer and matched against
    /// name, qualified_name, and content fields. Optional filters narrow results
    /// by file path and/or language.
    #[tracing::instrument(skip(self), fields(result_count))]
    pub fn search_bm25(
        &self,
        query: &str,
        limit: usize,
        file_path_filter: Option<&str>,
        language_filter: Option<Language>,
    ) -> Result<Vec<FullTextHit>, StorageError> {
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_name, self.f_qualified_name, self.f_content],
        );

        let (text_query, _errors) = query_parser.parse_query_lenient(query);

        let final_query: Box<dyn tantivy::query::Query> = {
            let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> =
                vec![(Occur::Must, text_query)];

            // Always filter to symbol documents
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.f_doc_type, "symbol"),
                    IndexRecordOption::Basic,
                )),
            ));

            if let Some(fp) = file_path_filter {
                clauses.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.f_file_path, fp),
                        IndexRecordOption::Basic,
                    )),
                ));
            }

            if let Some(lang) = language_filter {
                clauses.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.f_language, lang.name()),
                        IndexRecordOption::Basic,
                    )),
                ));
            }

            Box::new(BooleanQuery::new(clauses))
        };

        let searcher = self.reader.searcher();
        let top_docs = searcher.search(&*final_query, &TopDocs::with_limit(limit))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let retrieved: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(OwnedValue::Str(hex)) = retrieved.get_first(self.f_symbol_id) {
                if let Some(id) = parse_symbol_id_hex(hex) {
                    hits.push(FullTextHit {
                        symbol_id: id,
                        score,
                    });
                }
            }
        }

        tracing::Span::current().record("result_count", hits.len());
        Ok(hits)
    }

    /// Add a chunk document to the index.
    ///
    /// The chunk ID is stored with a `c:` prefix to avoid collision with symbol IDs.
    pub fn add_chunk_document(&mut self, chunk: &CodeChunk) -> Result<(), StorageError> {
        let id_hex = format!("c:{}", chunk.id);
        let content_text = oc_core::truncate_utf8_bytes(&chunk.content, CONTENT_MAX_BYTES);

        // Tokenize file path segments for BM25 matching
        let path_tokens: String = chunk
            .file_path
            .to_string_lossy()
            .split(|c: char| c == '/' || c == '\\' || c == '.' || c == '_' || c == '-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        let content = format!("{} {} {}", path_tokens, chunk.context_path, content_text);

        self.writer.add_document(doc!(
            self.f_symbol_id => id_hex,
            self.f_name => chunk.context_path.as_str(),
            self.f_qualified_name => chunk.context_path.as_str(),
            self.f_content => content.as_str(),
            self.f_file_path => chunk.file_path.to_string_lossy().as_ref(),
            self.f_language => chunk.language.name(),
            self.f_doc_type => "chunk",
        ))?;

        self.pending_count += 1;
        self.maybe_commit()?;
        Ok(())
    }

    /// Delete a chunk document by its chunk ID.
    pub fn delete_chunk_document(&mut self, chunk_id: ChunkId) -> Result<(), StorageError> {
        let hex = format!("c:{}", chunk_id);
        self.writer
            .delete_term(Term::from_field_text(self.f_symbol_id, &hex));
        self.pending_count += 1;
        self.maybe_commit()?;
        Ok(())
    }

    /// Search for chunks using BM25 ranking.
    ///
    /// Same query logic as `search_bm25()` but filters on `doc_type = "chunk"`.
    pub fn search_bm25_chunks(
        &self,
        query: &str,
        limit: usize,
        file_path_filter: Option<&str>,
        language_filter: Option<Language>,
    ) -> Result<Vec<FullTextChunkHit>, StorageError> {
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_name, self.f_qualified_name, self.f_content],
        );

        let (text_query, _errors) = query_parser.parse_query_lenient(query);

        let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> =
            vec![(Occur::Must, text_query)];

        // Filter to chunk documents only
        clauses.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(self.f_doc_type, "chunk"),
                IndexRecordOption::Basic,
            )),
        ));

        if let Some(fp) = file_path_filter {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.f_file_path, fp),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if let Some(lang) = language_filter {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.f_language, lang.name()),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        let final_query = BooleanQuery::new(clauses);
        let searcher = self.reader.searcher();
        let top_docs = searcher.search(&final_query, &TopDocs::with_limit(limit))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let retrieved: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(OwnedValue::Str(hex)) = retrieved.get_first(self.f_symbol_id) {
                if let Some(chunk_id) = parse_chunk_id_hex(hex) {
                    let file_path = retrieved
                        .get_first(self.f_file_path)
                        .and_then(|v| match v {
                            OwnedValue::Str(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    hits.push(FullTextChunkHit {
                        chunk_id,
                        file_path,
                        score,
                    });
                }
            }
        }

        Ok(hits)
    }

    /// Force a commit of all pending documents and refresh the reader.
    pub fn commit(&mut self) -> Result<(), StorageError> {
        if self.pending_count > 0 {
            self.writer.commit()?;
            self.reader.reload()?;
            self.pending_count = 0;
            self.last_commit = Instant::now();
        }
        Ok(())
    }

    /// Number of documents pending commit.
    pub fn pending_count(&self) -> usize {
        self.pending_count
    }

    fn maybe_commit(&mut self) -> Result<(), StorageError> {
        if self.pending_count >= BATCH_COUNT_THRESHOLD
            || self.last_commit.elapsed() >= BATCH_TIME_THRESHOLD
        {
            self.commit()?;
        }
        Ok(())
    }
}

impl Drop for FullTextStore {
    fn drop(&mut self) {
        let _ = self.commit();
    }
}

/// Parse a 32-hex-char SymbolId.
fn parse_symbol_id_hex(hex: &str) -> Option<SymbolId> {
    u128::from_str_radix(hex, 16).ok().map(SymbolId)
}

/// Parse a chunk ID hex string with `c:` prefix.
fn parse_chunk_id_hex(hex: &str) -> Option<ChunkId> {
    let stripped = hex.strip_prefix("c:")?;
    u128::from_str_radix(stripped, 16).ok().map(ChunkId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_symbol(id_val: u128, name: &str, qname: &str, lang: Language) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId(id_val),
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: oc_core::SymbolKind::Function,
            language: lang,
            file_path: PathBuf::from("src/main.py"),
            byte_range: 0..100,
            line_range: 0..5,
            signature: None,
            doc_comment: None,
            body_hash: 0,
            body_text: None,
        }
    }

    // --- Task 5.7: Tokenizer edge cases ---

    fn tokenize(text: &str) -> Vec<String> {
        let mut tok = CodeTokenizer::new();
        let mut stream = tok.token_stream(text);
        let mut tokens = Vec::new();
        let mut collect = |t: &Token| tokens.push(t.text.to_lowercase());
        stream.process(&mut collect);
        tokens
    }

    #[test]
    fn tokenizer_html_parser() {
        assert_eq!(tokenize("HTMLParser"), vec!["html", "parser"]);
    }

    #[test]
    fn tokenizer_parse_xml_stream() {
        assert_eq!(tokenize("parseXMLStream"), vec!["parse", "xml", "stream"]);
    }

    #[test]
    fn tokenizer_dunder_init() {
        assert_eq!(tokenize("__init__"), vec!["init"]);
    }

    #[test]
    fn tokenizer_i18n() {
        assert_eq!(tokenize("i18n"), vec!["i", "18", "n"]);
    }

    #[test]
    fn tokenizer_snake_case() {
        assert_eq!(tokenize("user_service"), vec!["user", "service"]);
    }

    #[test]
    fn tokenizer_pascal_case() {
        assert_eq!(tokenize("MyClassName"), vec!["my", "class", "name"]);
    }

    #[test]
    fn tokenizer_all_upper() {
        assert_eq!(tokenize("HTTP"), vec!["http"]);
    }

    #[test]
    fn tokenizer_mixed_numbers() {
        assert_eq!(tokenize("base64Decode"), vec!["base", "64", "decode"]);
    }

    // --- Task 5.3/5.5: Add/search/delete round-trip ---

    #[test]
    fn add_and_search_round_trip() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym =
            make_symbol(1, "parseXMLStream", "http.client.parseXMLStream", Language::Python);
        store
            .add_document(&sym, Some("def parseXMLStream(data): pass"))
            .unwrap();
        store.commit().unwrap();

        let hits = store.search_bm25("parse", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol_id, SymbolId(1));
    }

    #[test]
    fn cross_case_matching() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym = make_symbol(10, "HTMLParser", "html.HTMLParser", Language::Python);
        store.add_document(&sym, None).unwrap();
        store.commit().unwrap();

        // Searching lowercase matches PascalCase/UPPER symbol names
        let hits = store.search_bm25("html", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol_id, SymbolId(10));

        let hits = store.search_bm25("parser", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);

        // Upper-case query also matches (tokenizer lowercases)
        let hits = store.search_bm25("HTML", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn delete_document_removes_from_search() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym = make_symbol(20, "myFunc", "mod.myFunc", Language::Rust);
        store.add_document(&sym, None).unwrap();
        store.commit().unwrap();

        assert_eq!(
            store.search_bm25("myFunc", 10, None, None).unwrap().len(),
            1
        );

        store.delete_document(SymbolId(20)).unwrap();
        store.commit().unwrap();

        assert!(store
            .search_bm25("myFunc", 10, None, None)
            .unwrap()
            .is_empty());
    }

    // --- Task 5.5: Filters ---

    #[test]
    fn language_filter() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let py = make_symbol(30, "process", "app.process", Language::Python);
        let mut rs = make_symbol(31, "process", "app.process", Language::Rust);
        rs.file_path = PathBuf::from("src/main.rs");

        store.add_document(&py, None).unwrap();
        store.add_document(&rs, None).unwrap();
        store.commit().unwrap();

        let all = store.search_bm25("process", 10, None, None).unwrap();
        assert_eq!(all.len(), 2);

        let py_only = store
            .search_bm25("process", 10, None, Some(Language::Python))
            .unwrap();
        assert_eq!(py_only.len(), 1);
        assert_eq!(py_only[0].symbol_id, SymbolId(30));

        let rs_only = store
            .search_bm25("process", 10, None, Some(Language::Rust))
            .unwrap();
        assert_eq!(rs_only.len(), 1);
        assert_eq!(rs_only[0].symbol_id, SymbolId(31));
    }

    #[test]
    fn file_path_filter() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let mut s1 = make_symbol(40, "handler", "web.handler", Language::Python);
        s1.file_path = PathBuf::from("src/web.py");
        let mut s2 = make_symbol(41, "handler", "api.handler", Language::Python);
        s2.file_path = PathBuf::from("src/api.py");

        store.add_document(&s1, None).unwrap();
        store.add_document(&s2, None).unwrap();
        store.commit().unwrap();

        let filtered = store
            .search_bm25("handler", 10, Some("src/web.py"), None)
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].symbol_id, SymbolId(40));
    }

    // --- Task 5.5: Body content truncation ---

    #[test]
    fn body_truncation_at_10kb() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym = make_symbol(50, "bigFunc", "mod.bigFunc", Language::Python);

        // Create body larger than 10KB with a unique token at the end
        let mut body = "x ".repeat(6000); // 12000 bytes
        body.push_str("uniqueEndToken");

        store.add_document(&sym, Some(&body)).unwrap();
        store.commit().unwrap();

        // The symbol name is always indexed
        let hits = store.search_bm25("bigFunc", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);

        // The unique token past 10KB should NOT be findable
        let hits = store
            .search_bm25("uniqueEndToken", 10, None, None)
            .unwrap();
        assert!(hits.is_empty());
    }

    // --- Task 5.4: Batched commit triggers ---

    #[test]
    fn batch_count_triggers_commit() {
        let mut store = FullTextStore::create_in_ram().unwrap();

        for i in 0..500u128 {
            let sym =
                make_symbol(i + 1000, &format!("fn{i}"), &format!("mod.fn{i}"), Language::Rust);
            store.add_document(&sym, None).unwrap();
        }

        // After 500 docs, an automatic commit should have happened
        assert_eq!(store.pending_count(), 0);

        let hits = store.search_bm25("fn0", 10, None, None).unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    fn time_threshold_triggers_commit() {
        let mut store = FullTextStore::create_in_ram().unwrap();

        // Force last_commit to be in the past
        store.last_commit = Instant::now() - Duration::from_secs(1);

        let sym = make_symbol(2000, "timedFunc", "mod.timedFunc", Language::Go);
        store.add_document(&sym, None).unwrap();

        // The 500ms threshold should have triggered a commit
        assert_eq!(store.pending_count(), 0);
    }

    // --- Task 5.6: Graceful degradation ---

    #[test]
    fn search_empty_index_returns_empty() {
        let store = FullTextStore::create_in_ram().unwrap();
        let hits = store.search_bm25("anything", 10, None, None).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn corrupted_dir_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tantivy");
        std::fs::create_dir_all(&path).unwrap();
        // Write a corrupt meta.json
        std::fs::write(path.join("meta.json"), b"not valid json").unwrap();

        let result = FullTextStore::open(&path);
        assert!(result.is_err());
    }

    // --- Task 5.2: SymbolId round-trip via hex ---

    #[test]
    fn symbol_id_hex_round_trip() {
        let id = SymbolId(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0);
        let hex = format!("{id}");
        let parsed = parse_symbol_id_hex(&hex).unwrap();
        assert_eq!(parsed, id);
    }

    // --- UTF-8 truncation ---

    #[test]
    fn truncate_utf8_on_boundary() {
        use oc_core::truncate_utf8_bytes;
        assert_eq!(truncate_utf8_bytes("hello", 3), "hel");
        assert_eq!(truncate_utf8_bytes("hello", 100), "hello");
        // Multi-byte: 'é' is 2 bytes
        assert_eq!(truncate_utf8_bytes("café", 4), "caf");
        assert_eq!(truncate_utf8_bytes("café", 5), "café");
    }

    // --- Persistence ---

    #[test]
    fn persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tantivy");

        let sym = make_symbol(3000, "persist", "mod.persist", Language::Java);

        {
            let mut store = FullTextStore::open(&path).unwrap();
            store.add_document(&sym, None).unwrap();
            store.commit().unwrap();
        }

        {
            let store = FullTextStore::open(&path).unwrap();
            let hits = store.search_bm25("persist", 10, None, None).unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].symbol_id, SymbolId(3000));
        }
    }

    // --- Path token searchability ---

    #[test]
    fn path_tokens_searchable() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let mut sym = make_symbol(4000, "detect_formula", "model.mfd.detect_formula", Language::Python);
        sym.file_path = PathBuf::from("model/mfd/detect_formula.py");
        store.add_document(&sym, Some("def detect_formula(image, model): pass")).unwrap();
        store.commit().unwrap();

        // Search by directory name from path
        let hits = store.search_bm25("mfd", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol_id, SymbolId(4000));

        // Search by "model" from path
        let hits = store.search_bm25("model", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);

        // Search by "formula" matches both path token and function name
        let hits = store.search_bm25("formula", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    // --- Chunk document tests ---

    fn make_chunk(id_val: u128, file: &str, context: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: ChunkId(id_val),
            language: Language::Python,
            file_path: PathBuf::from(file),
            byte_range: 0..100,
            line_range: 0..5,
            chunk_index: 0,
            total_chunks: 1,
            context_path: context.to_string(),
            content: content.to_string(),
            content_hash: 0,
        }
    }

    #[test]
    fn chunk_add_and_search() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let chunk = make_chunk(100, "src/parser.py", "MyParser.parse", "def parse(self, data): return data.strip()");
        store.add_chunk_document(&chunk).unwrap();
        store.commit().unwrap();

        let hits = store.search_bm25_chunks("parse", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, ChunkId(100));
        assert_eq!(hits[0].file_path, "src/parser.py");
    }

    #[test]
    fn chunk_isolation_from_symbol_search() {
        let mut store = FullTextStore::create_in_ram().unwrap();

        // Add a symbol and a chunk with the same content
        let sym = make_symbol(1, "parse", "parser.parse", Language::Python);
        store.add_document(&sym, Some("def parse(): pass")).unwrap();

        let chunk = make_chunk(200, "src/parser.py", "Parser", "def parse(): pass");
        store.add_chunk_document(&chunk).unwrap();
        store.commit().unwrap();

        // Symbol search should only find the symbol, not the chunk
        let sym_hits = store.search_bm25("parse", 10, None, None).unwrap();
        assert_eq!(sym_hits.len(), 1);
        assert_eq!(sym_hits[0].symbol_id, SymbolId(1));

        // Chunk search should only find the chunk, not the symbol
        let chunk_hits = store.search_bm25_chunks("parse", 10, None, None).unwrap();
        assert_eq!(chunk_hits.len(), 1);
        assert_eq!(chunk_hits[0].chunk_id, ChunkId(200));
    }

    #[test]
    fn chunk_delete() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let chunk = make_chunk(300, "src/utils.py", "", "def helper(): pass");
        store.add_chunk_document(&chunk).unwrap();
        store.commit().unwrap();

        assert_eq!(store.search_bm25_chunks("helper", 10, None, None).unwrap().len(), 1);

        store.delete_chunk_document(ChunkId(300)).unwrap();
        store.commit().unwrap();

        assert!(store.search_bm25_chunks("helper", 10, None, None).unwrap().is_empty());
    }

    #[test]
    fn chunk_id_hex_round_trip() {
        let id = ChunkId(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0);
        let hex = format!("c:{id}");
        let parsed = parse_chunk_id_hex(&hex).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn chunk_clear_removes_chunks() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let chunk = make_chunk(400, "src/a.py", "", "def foo(): pass");
        store.add_chunk_document(&chunk).unwrap();
        store.commit().unwrap();
        assert_eq!(store.search_bm25_chunks("foo", 10, None, None).unwrap().len(), 1);

        store.clear().unwrap();
        assert!(store.search_bm25_chunks("foo", 10, None, None).unwrap().is_empty());
    }

    // --- CJK bigram tokenization ---

    #[test]
    fn tokenizer_cjk_bigram() {
        // "识别框" → ["识别", "别框"]
        assert_eq!(tokenize("识别框"), vec!["识别", "别框"]);
    }

    #[test]
    fn tokenizer_cjk_single_char() {
        // Single CJK char → unigram
        assert_eq!(tokenize("框"), vec!["框"]);
    }

    #[test]
    fn tokenizer_cjk_two_chars() {
        assert_eq!(tokenize("识别"), vec!["识别"]);
    }

    #[test]
    fn tokenizer_mixed_ascii_cjk() {
        // "box识别框detect" → ["box", "识别", "别框", "detect"]
        assert_eq!(tokenize("box识别框detect"), vec!["box", "识别", "别框", "detect"]);
    }

    #[test]
    fn tokenizer_cjk_in_comment() {
        // Simulates a Python comment with CJK
        assert_eq!(
            tokenize("# 不相交直接退出检测"),
            vec!["不相", "相交", "交直", "直接", "接退", "退出", "出检", "检测"]
        );
    }

    #[test]
    fn tokenizer_cjk_mixed_with_snake_case() {
        assert_eq!(
            tokenize("calculate_iou # 计算交集"),
            vec!["calculate", "iou", "计算", "算交", "交集"]
        );
    }

    #[test]
    fn cjk_bm25_search() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym = make_symbol(5000, "merge_det_boxes", "ocr_utils.merge_det_boxes", Language::Python);
        store.add_document(&sym, Some("def merge_det_boxes(dt_boxes):\n    # 不相交直接退出检测\n    pass")).unwrap();
        store.commit().unwrap();

        // Chinese query should find via CJK bigram matching on comment
        let hits = store.search_bm25("检测", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol_id, SymbolId(5000));
    }

    // --- Lenient query parsing (natural language with special chars) ---

    #[test]
    fn natural_language_query_does_not_crash() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let sym = make_symbol(6000, "validate_input", "app.validate_input", Language::Python);
        store
            .add_document(&sym, Some("def validate_input(data): return bool(data)"))
            .unwrap();
        store.commit().unwrap();

        // Queries containing Tantivy DSL special characters should not error
        let tricky_queries = [
            "validate (input) data",
            "how does validate_input work?",
            r#"fix the "bug" in validate"#,
            "validate: input -> output",
            "validate() returns bool(data)",
            "path/to/file.py:123",
            "error in `validate_input` function",
            "validate + input - other",
            "field~2 boost^3",
        ];

        for q in &tricky_queries {
            let result = store.search_bm25(q, 10, None, None);
            assert!(result.is_ok(), "query {:?} should not error: {:?}", q, result.err());
        }
    }

    #[test]
    fn natural_language_chunk_query_does_not_crash() {
        let mut store = FullTextStore::create_in_ram().unwrap();
        let chunk = make_chunk(600, "src/app.py", "App.run", "def run(self): pass");
        store.add_chunk_document(&chunk).unwrap();
        store.commit().unwrap();

        let tricky_queries = [
            "how does App.run() work?",
            "fix the (bug) in run",
            r#"the "run" method"#,
        ];

        for q in &tricky_queries {
            let result = store.search_bm25_chunks(q, 10, None, None);
            assert!(result.is_ok(), "chunk query {:?} should not error: {:?}", q, result.err());
        }
    }
}
