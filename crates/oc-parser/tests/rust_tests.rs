#[cfg(test)]
mod rust_tests {
    use oc_core::{RelationKind, SymbolKind};
    use oc_parser::parse_file;

    fn parse(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "src/lib.rs", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    #[test]
    fn extracts_module() {
        let out = parse("fn main() {}\n");
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(module.is_some());
        // lib.rs uses parent dir name
        assert_eq!(module.unwrap().name, "src");
    }

    #[test]
    fn extracts_struct() {
        let source = "pub struct Point {\n    x: f64,\n    y: f64,\n}\n";
        let out = parse(source);
        let s = out.symbols.iter().find(|s| s.kind == SymbolKind::Struct && s.name == "Point");
        assert!(s.is_some(), "should extract struct");
    }

    #[test]
    fn extracts_enum() {
        let source = "enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
        let out = parse(source);
        let e = out.symbols.iter().find(|s| s.kind == SymbolKind::Enum && s.name == "Color");
        assert!(e.is_some(), "should extract enum");
    }

    #[test]
    fn extracts_trait() {
        let source = "trait Drawable {\n    fn draw(&self);\n}\n";
        let out = parse(source);
        let t = out.symbols.iter().find(|s| s.kind == SymbolKind::Trait && s.name == "Drawable");
        assert!(t.is_some(), "should extract trait");
        let m = out.symbols.iter().find(|s| s.kind == SymbolKind::Method && s.name == "draw");
        assert!(m.is_some(), "should extract trait method");
    }

    #[test]
    fn extracts_function() {
        let source = "fn greet(name: &str) -> String {\n    format!(\"Hello {}\", name)\n}\n";
        let out = parse(source);
        let f = out.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "greet");
        assert!(f.is_some(), "should extract function");
        let sig = f.unwrap().signature.as_deref().unwrap();
        assert!(sig.contains("fn greet"));
    }

    #[test]
    fn extracts_impl_methods() {
        let source = r#"
struct Foo;
impl Foo {
    fn new() -> Self { Foo }
    fn bar(&self) {}
}
"#;
        let out = parse(source);
        let methods: Vec<_> = out.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(methods.len() >= 2, "should extract impl methods, got {}", methods.len());

        let new_method = out.symbols.iter().find(|s| s.name == "new" && s.kind == SymbolKind::Method);
        assert!(new_method.is_some());
        assert!(new_method.unwrap().qualified_name.contains("Foo.new"));
    }

    #[test]
    fn extracts_const() {
        let source = "const MAX: u32 = 100;\n";
        let out = parse(source);
        let c = out.symbols.iter().find(|s| s.kind == SymbolKind::Constant && s.name == "MAX");
        assert!(c.is_some(), "should extract const");
    }

    #[test]
    fn extracts_type_alias() {
        let source = "type Result<T> = std::result::Result<T, Error>;\n";
        let out = parse(source);
        let t = out.symbols.iter().find(|s| s.kind == SymbolKind::TypeAlias && s.name == "Result");
        assert!(t.is_some(), "should extract type alias");
    }

    #[test]
    fn extracts_use_imports() {
        let source = "use std::collections::HashMap;\nuse crate::foo::Bar;\n";
        let out = parse(source);
        let imports: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Imports).collect();
        assert!(imports.len() >= 2, "should have at least 2 import relations, got {}", imports.len());
    }

    #[test]
    fn extracts_mod_item() {
        let source = "mod inner {\n    fn helper() {}\n}\n";
        let out = parse(source);
        let m = out.symbols.iter().find(|s| s.kind == SymbolKind::Module && s.name == "inner");
        assert!(m.is_some(), "should extract mod");
        let f = out.symbols.iter().find(|s| s.name == "helper");
        assert!(f.is_some(), "should extract nested function in mod");
        assert!(f.unwrap().qualified_name.contains("inner.helper"));
    }

    #[test]
    fn extracts_impl_trait_implements() {
        let source = r#"
struct Foo;
trait Bar {}
impl Bar for Foo {}
"#;
        let out = parse(source);
        let impls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Implements).collect();
        assert!(!impls.is_empty(), "should have Implements relation for impl Trait for Type");
    }

    #[test]
    fn extracts_calls() {
        let source = "fn main() {\n    println!(\"hello\");\n    foo();\n}\n";
        let out = parse(source);
        let calls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Calls).collect();
        assert!(!calls.is_empty(), "should extract call relations");
    }
}
