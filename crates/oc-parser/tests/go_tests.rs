#[cfg(test)]
mod go_tests {
    use oc_core::{RelationKind, SymbolKind};
    use oc_parser::parse_file;

    fn parse(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "main.go", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    #[test]
    fn extracts_package() {
        let source = "package main\n\nfunc main() {}\n";
        let out = parse(source);
        let pkg = out.symbols.iter().find(|s| s.kind == SymbolKind::Package);
        assert!(pkg.is_some());
        assert_eq!(pkg.unwrap().name, "main");
    }

    #[test]
    fn extracts_function() {
        let source = "package main\n\nfunc Hello(name string) string {\n\treturn \"Hello \" + name\n}\n";
        let out = parse(source);
        let f = out.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "Hello");
        assert!(f.is_some(), "should extract function");
        assert_eq!(f.unwrap().qualified_name, "main.Hello");
    }

    #[test]
    fn extracts_struct() {
        let source = "package models\n\ntype User struct {\n\tName string\n\tAge  int\n}\n";
        let out = parse_file("test-repo", "models/user.go", source.as_bytes(), source.len() as u64).unwrap();
        let s = out.symbols.iter().find(|s| s.kind == SymbolKind::Struct && s.name == "User");
        assert!(s.is_some(), "should extract struct");
    }

    #[test]
    fn extracts_interface() {
        let source = "package io\n\ntype Reader interface {\n\tRead(p []byte) (n int, err error)\n}\n";
        let out = parse_file("test-repo", "io/reader.go", source.as_bytes(), source.len() as u64).unwrap();
        let i = out.symbols.iter().find(|s| s.kind == SymbolKind::Interface && s.name == "Reader");
        assert!(i.is_some(), "should extract interface");
    }

    #[test]
    fn extracts_method() {
        let source = "package main\n\ntype Foo struct{}\n\nfunc (f *Foo) Bar() {}\n";
        let out = parse(source);
        let m = out.symbols.iter().find(|s| s.kind == SymbolKind::Method && s.name == "Bar");
        assert!(m.is_some(), "should extract method");
        assert!(m.unwrap().qualified_name.contains("Foo.Bar"), "method should be scoped to receiver type");
    }

    #[test]
    fn extracts_const() {
        let source = "package main\n\nconst MaxSize = 100\n";
        let out = parse(source);
        let c = out.symbols.iter().find(|s| s.kind == SymbolKind::Constant && s.name == "MaxSize");
        assert!(c.is_some(), "should extract const");
    }

    #[test]
    fn extracts_var() {
        let source = "package main\n\nvar count int\n";
        let out = parse(source);
        let v = out.symbols.iter().find(|s| s.kind == SymbolKind::Variable && s.name == "count");
        assert!(v.is_some(), "should extract var");
    }

    #[test]
    fn extracts_import_relations() {
        let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
        let out = parse(source);
        let imports: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Imports).collect();
        assert!(imports.len() >= 2, "should have at least 2 import relations, got {}", imports.len());
    }

    #[test]
    fn extracts_calls() {
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
        let out = parse(source);
        let calls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Calls).collect();
        assert!(!calls.is_empty(), "should extract call relations");
    }

    #[test]
    fn extracts_contains_relations() {
        let source = "package main\n\nfunc foo() {}\nfunc bar() {}\n";
        let out = parse(source);
        let contains: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Contains).collect();
        assert!(contains.len() >= 2, "should have Contains relations");
    }
}
