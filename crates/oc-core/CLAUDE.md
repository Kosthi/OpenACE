[Root](../../CLAUDE.md) > [crates](./) > **oc-core**

# oc-core

## Module Responsibility

Shared types and utilities for the entire OpenACE Rust workspace. Defines the fundamental data model that all other crates depend on.

## Entry Point

- `src/lib.rs` -- re-exports all public types and the `truncate_utf8_bytes` utility

## Public API

- **`SymbolId`** (`src/symbol.rs`) -- Deterministic 128-bit identifier computed via XXH3-128 hash of `repo_id|relative_path|qualified_name|byte_start|byte_end`. Provides `as_bytes()` / `from_bytes()` for storage serialization.
- **`CodeSymbol`** (`src/symbol.rs`) -- Full symbol record: id, name, qualified_name, kind, language, file_path, byte_range, line_range, signature, doc_comment, body_hash.
- **`SymbolKind`** (`src/symbol.rs`) -- Enum: Function, Method, Class, Struct, Interface, Trait, Module, Package, Variable, Constant, Enum, TypeAlias.
- **`CodeRelation`** (`src/relation.rs`) -- Directed relationship between two symbols: source_id, target_id, kind, file_path, line, confidence.
- **`RelationKind`** (`src/relation.rs`) -- Enum: Calls, Imports, Inherits, Implements, Uses, Contains. Each has a default confidence score.
- **`Language`** (`src/language.rs`) -- Enum: Python, TypeScript, JavaScript, Rust, Go, Java. Maps file extensions and provides native separator info.
- **`QualifiedName`** (`src/qualified_name.rs`) -- Utilities for normalizing qualified names (e.g., Rust `::` -> `.`) and rendering in native form.
- **`CoreError`** (`src/error.rs`) -- Error type for hash failures, invalid ordinals, conversion failures.
- **`truncate_utf8_bytes()`** (`src/lib.rs`) -- Safe UTF-8 truncation on byte boundaries.

## Key Dependencies

- `xxhash-rust` (XXH3 hashing)
- `thiserror` (error derive)
- `serde` (serialization)

## Tests

All source files contain inline `#[cfg(test)] mod tests` with unit tests covering round-trip serialization, deterministic ID generation, ordinal mappings, and edge cases.

## Related Files

- `Cargo.toml`
- `src/lib.rs`
- `src/symbol.rs`
- `src/relation.rs`
- `src/language.rs`
- `src/qualified_name.rs`
- `src/error.rs`

## Changelog

- 2026-02-11: Initial module CLAUDE.md created.
