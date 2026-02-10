#[cfg(test)]
mod java_tests {
    use oc_core::{RelationKind, SymbolKind};
    use oc_parser::parse_file;

    fn parse(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "src/Main.java", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    #[test]
    fn extracts_package() {
        let source = "package com.example;\n\npublic class Main {}\n";
        let out = parse(source);
        let pkg = out.symbols.iter().find(|s| s.kind == SymbolKind::Package);
        assert!(pkg.is_some());
    }

    #[test]
    fn extracts_class() {
        let source = "package test;\n\npublic class Animal {\n    public void speak() {}\n}\n";
        let out = parse(source);
        let cls = out.symbols.iter().find(|s| s.kind == SymbolKind::Class && s.name == "Animal");
        assert!(cls.is_some(), "should extract class");
    }

    #[test]
    fn extracts_interface() {
        let source = "package test;\n\npublic interface Serializable {\n    String serialize();\n}\n";
        let out = parse(source);
        let iface = out.symbols.iter().find(|s| s.kind == SymbolKind::Interface && s.name == "Serializable");
        assert!(iface.is_some(), "should extract interface");
    }

    #[test]
    fn extracts_enum() {
        let source = "package test;\n\npublic enum Color {\n    RED, GREEN, BLUE\n}\n";
        let out = parse(source);
        let en = out.symbols.iter().find(|s| s.kind == SymbolKind::Enum && s.name == "Color");
        assert!(en.is_some(), "should extract enum");
    }

    #[test]
    fn extracts_methods() {
        let source = r#"
package test;

public class Calculator {
    public int add(int a, int b) {
        return a + b;
    }
    public int subtract(int a, int b) {
        return a - b;
    }
}
"#;
        let out = parse(source);
        let methods: Vec<_> = out.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(methods.len() >= 2, "should extract at least 2 methods, got {}", methods.len());

        let add = out.symbols.iter().find(|s| s.name == "add" && s.kind == SymbolKind::Method);
        assert!(add.is_some());
        assert!(add.unwrap().qualified_name.contains("Calculator.add"));
    }

    #[test]
    fn extracts_constant_field() {
        let source = r#"
package test;

public class Config {
    public static final int MAX_SIZE = 100;
}
"#;
        let out = parse(source);
        let c = out.symbols.iter().find(|s| s.kind == SymbolKind::Constant && s.name == "MAX_SIZE");
        assert!(c.is_some(), "should extract static final field as constant");
    }

    #[test]
    fn extracts_import_relations() {
        let source = "package test;\n\nimport java.util.List;\nimport java.util.Map;\n\npublic class Main {}\n";
        let out = parse(source);
        let imports: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Imports).collect();
        assert!(imports.len() >= 2, "should have at least 2 import relations, got {}", imports.len());
    }

    #[test]
    fn extracts_inherits_relation() {
        let source = "package test;\n\npublic class Dog extends Animal {\n}\n";
        let out = parse(source);
        let inherits: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Inherits).collect();
        assert!(!inherits.is_empty(), "should have Inherits relation");
    }

    #[test]
    fn extracts_implements_relation() {
        let source = "package test;\n\npublic class MyClass implements Runnable {\n    public void run() {}\n}\n";
        let out = parse(source);
        let impls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Implements).collect();
        assert!(!impls.is_empty(), "should have Implements relation");
    }

    #[test]
    fn extracts_calls() {
        let source = r#"
package test;

public class Main {
    public void run() {
        System.out.println("hello");
        helper();
    }
    private void helper() {}
}
"#;
        let out = parse(source);
        let calls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Calls).collect();
        assert!(!calls.is_empty(), "should extract call relations");
    }

    #[test]
    fn extracts_nested_class() {
        let source = r#"
package test;

public class Outer {
    public class Inner {
        public void method() {}
    }
}
"#;
        let out = parse(source);
        let inner = out.symbols.iter().find(|s| s.kind == SymbolKind::Class && s.name == "Inner");
        assert!(inner.is_some(), "should extract nested class");
        assert!(inner.unwrap().qualified_name.contains("Outer.Inner"));
    }

    #[test]
    fn extracts_enum_constants() {
        let source = "package test;\n\npublic enum Direction {\n    NORTH, SOUTH, EAST, WEST\n}\n";
        let out = parse(source);
        let constants: Vec<_> = out.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert!(constants.len() >= 4, "should extract 4 enum constants, got {}", constants.len());
    }
}
