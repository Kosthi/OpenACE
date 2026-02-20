[Root](../../CLAUDE.md) > [crates](./) > **oc-parser**

# oc-parser

## Module Responsibility

Multi-language AST parser that uses tree-sitter grammars to extract code symbols and relationships from source files. Supports Python, TypeScript/JavaScript, Rust, Go, and Java.

## Entry Point

- `src/lib.rs` -- re-exports `parse_file()`, `ParseOutput`, `ParserRegistry`, `check_file_size`, `is_binary`
- `src/visitor.rs` -- `parse_file()` is the main entry point; dispatches to language-specific visitors

## Public API

- **`parse_file(repo_id, file_path, content, file_size)`** -- Parse a source file, returning `ParseOutput { symbols, relations }`. Validates size (1 MB limit), encoding (rejects binary), and language support.
- **`ParseOutput`** -- Contains `Vec<CodeSymbol>` and `Vec<CodeRelation>`.
- **`ParserRegistry`** -- Static methods for language/grammar lookup from file extensions.
- **`check_file_size()` / `is_binary()`** -- Pre-parse validation utilities.

## Language Visitors

Each visitor (`src/visitor/<lang>.rs`) walks the tree-sitter AST and extracts:
- Symbol definitions (functions, classes, methods, structs, interfaces, etc.)
- Relationships (calls, imports, inheritance, containment)
- Signatures and doc comments
- Body hashes for incremental change detection

| File | Language(s) |
|------|-------------|
| `src/visitor/python.rs` | Python |
| `src/visitor/typescript.rs` | TypeScript, JavaScript |
| `src/visitor/rust_lang.rs` | Rust |
| `src/visitor/go_lang.rs` | Go |
| `src/visitor/java.rs` | Java |

## Key Dependencies

- `oc-core` (shared types)
- `tree-sitter` + per-language grammars (python, typescript, rust, go, java)
- `xxhash-rust` (body hashing)

## Tests

- Inline unit tests in source files
- Integration tests per language: `tests/python_tests.rs`, `tests/typescript_tests.rs`, `tests/rust_tests.rs`, `tests/go_tests.rs`, `tests/java_tests.rs`

## Related Files

- `Cargo.toml`
- `src/lib.rs`, `src/visitor.rs`, `src/registry.rs`
- `src/visitor/python.rs`, `src/visitor/typescript.rs`, `src/visitor/rust_lang.rs`, `src/visitor/go_lang.rs`, `src/visitor/java.rs`
- `src/body_hash.rs`, `src/file_check.rs`, `src/error.rs`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
