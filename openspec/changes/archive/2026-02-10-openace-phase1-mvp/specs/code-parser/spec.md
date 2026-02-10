## ADDED Requirements

### Requirement: Multi-language AST parsing via tree-sitter
The system SHALL parse source files using tree-sitter grammars for 5 languages: Python, TypeScript, JavaScript, Rust, Go, and Java. TypeScript and TSX SHALL use the `tree-sitter-typescript` crate which provides both grammars.

The parser SHALL use a `ParserRegistry` that maps file extensions to language grammars:
- `.py` → Python
- `.ts` → TypeScript, `.tsx` → TSX
- `.js`, `.jsx` → JavaScript
- `.rs` → Rust
- `.go` → Go
- `.java` → Java

#### Scenario: Parse Python file with classes and functions
- **WHEN** a Python file containing classes, methods, and functions is parsed
- **THEN** all symbols are extracted with correct `kind`, `qualified_name`, `line_range`, `signature`, `doc_comment`, and `body_hash`

#### Scenario: Parse TypeScript file with interfaces and types
- **WHEN** a TypeScript file containing interfaces, type aliases, and arrow functions is parsed
- **THEN** all named symbols are extracted; arrow functions assigned to variables are extracted as Functions

#### Scenario: Unsupported file extension
- **WHEN** a file with extension `.txt` is submitted for parsing
- **THEN** the parser returns a `ParserError::UnsupportedLanguage` error

### Requirement: Symbol extraction with qualified names
The system SHALL extract `CodeSymbol` instances from parsed ASTs. Each symbol SHALL have a `qualified_name` computed by traversing the AST scope chain and joining names with dot separators.

For each language, the following symbol kinds SHALL be extracted:
- **Python**: Module, Class, Function, Method, Variable (module-level), Constant
- **TypeScript/JavaScript**: Module, Class, Function, Method, Interface, TypeAlias, Variable (module-level), Constant, Enum
- **Rust**: Module, Struct, Enum, Trait, Function, Method, Constant, TypeAlias
- **Go**: Package, Struct, Interface, Function, Method, Constant, Variable
- **Java**: Package, Class, Interface, Enum, Method, Constant

Anonymous functions, lambdas, and closures SHALL be skipped (not extracted as symbols).

#### Scenario: Qualified name computation for nested class
- **WHEN** a Python file `auth.py` contains `class AuthService` with method `login`
- **THEN** the method's qualified name is `auth.AuthService.login`

#### Scenario: Anonymous function skipping
- **WHEN** a Python file contains `lambda x: x + 1` or JavaScript contains `() => {}`
- **THEN** no symbol is created for the anonymous function

#### Scenario: Rust qualified name normalization
- **WHEN** a Rust file in module `collections` contains `struct HashMap`
- **THEN** the qualified name is stored internally as `collections.HashMap` (dot-separated)

### Requirement: Static relation extraction
The system SHALL extract `CodeRelation` instances from ASTs representing static code relationships. The following relation kinds SHALL be extracted:

- **Calls**: Function/method invocations (confidence: 0.8)
- **Imports**: Import/use/require statements (confidence: 0.9)
- **Inherits**: Class inheritance / struct embedding (confidence: 0.85)
- **Implements**: Interface/trait implementation (confidence: 0.85)
- **Uses**: Type references in signatures/annotations (confidence: 0.7)
- **Contains**: Parent-child scope containment (confidence: 0.95)

Re-exports and aliased imports SHALL be represented as `Imports` relations pointing to the original symbol. No alias symbols SHALL be created.

#### Scenario: Call relation extraction
- **WHEN** function `A` contains a call to function `B`
- **THEN** a `CodeRelation { source: A.id, target: B.id, kind: Calls, confidence: 0.8 }` is created

#### Scenario: Import relation for re-export
- **WHEN** module `foo` re-exports `bar.Baz`
- **THEN** an `Imports` relation from `foo` to `bar.Baz` is created; no alias symbol is created

#### Scenario: Contains relation for class method
- **WHEN** class `MyClass` contains method `my_method`
- **THEN** a `Contains` relation from `MyClass` to `my_method` with confidence 0.95 is created

### Requirement: File size limit enforcement
The system SHALL skip files larger than 1MB (1,048,576 bytes). The file size SHALL be checked before reading the file contents. Skipped files SHALL be reported via a `ParserError::FileTooLarge` with the file path and actual size.

#### Scenario: File at size limit
- **WHEN** a source file is exactly 1,048,576 bytes
- **THEN** the file is parsed normally

#### Scenario: File exceeding size limit
- **WHEN** a source file is 1,048,577 bytes
- **THEN** the file is skipped and a `FileTooLarge` error is returned

### Requirement: Parser robustness against invalid input
The system SHALL never panic on any input. Files with invalid UTF-8, syntax errors, or corrupted content SHALL produce a best-effort parse result or an error, but SHALL NOT crash the process. Each file parse failure SHALL be isolated — a failure on one file SHALL NOT prevent other files from being parsed.

#### Scenario: Invalid UTF-8 file
- **WHEN** a `.py` file contains invalid UTF-8 byte sequences
- **THEN** the parser returns an error for that file but does not panic

#### Scenario: Syntax error in source file
- **WHEN** a Python file contains syntax errors
- **THEN** tree-sitter performs error-recovery parsing and extracts whatever symbols it can; incomplete symbols are omitted

### Requirement: Parsing throughput target
The system SHALL achieve a parsing throughput of >50,000 symbols per second when processing Python and TypeScript files on a single thread. Multi-threaded parsing via rayon SHALL scale linearly with available cores.

#### Scenario: Single-thread throughput benchmark
- **WHEN** 10,000 Python files averaging 20 symbols each are parsed on a single thread
- **THEN** total parsing time is under 4 seconds (>50K symbols/sec)
