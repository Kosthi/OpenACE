use std::path::Path;
use std::time::{Duration, Instant};

use oc_core::{CodeSymbol, Language, SymbolId};
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
        }
    }
}

struct CodeTokenStream<'a> {
    text: &'a [u8],
    pos: usize,
    token: &'a mut Token,
}

impl<'a> TokenStream for CodeTokenStream<'a> {
    fn advance(&mut self) -> bool {
        let len = self.text.len();

        // Skip non-alphanumeric characters (underscores, dots, punctuation, etc.)
        while self.pos < len && !self.text[self.pos].is_ascii_alphanumeric() {
            self.pos += 1;
        }

        if self.pos >= len {
            return false;
        }

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
    pending_count: usize,
    last_commit: Instant,
}

fn build_schema() -> (Schema, Field, Field, Field, Field, Field, Field) {
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

    let schema = builder.build();
    (schema, symbol_id, name, qualified_name, content, file_path, language)
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

        let (schema, f_symbol_id, f_name, f_qualified_name, f_content, f_file_path, f_language) =
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
            pending_count: 0,
            last_commit: Instant::now(),
        })
    }

    /// Create an in-memory full-text index (for testing).
    pub fn create_in_ram() -> Result<Self, StorageError> {
        let (schema, f_symbol_id, f_name, f_qualified_name, f_content, f_file_path, f_language) =
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

        let text_query =
            query_parser
                .parse_query(query)
                .map_err(|e| StorageError::FullTextIndexUnavailable {
                    reason: format!("query parse error: {e}"),
                })?;

        let final_query: Box<dyn tantivy::query::Query> =
            if file_path_filter.is_some() || language_filter.is_some() {
                let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> =
                    vec![(Occur::Must, text_query)];

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
            } else {
                text_query
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
}
