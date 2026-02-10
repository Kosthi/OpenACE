#[cfg(test)]
mod typescript_tests {
    use oc_core::{RelationKind, SymbolKind};
    use oc_parser::parse_file;

    fn parse_ts(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "src/app.ts", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    fn parse_tsx(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "src/App.tsx", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    #[test]
    fn extracts_module() {
        let out = parse_ts("const x = 1;\n");
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(module.is_some());
        assert_eq!(module.unwrap().name, "app");
    }

    #[test]
    fn extracts_interface() {
        let source = "interface Serializable {\n  serialize(): string;\n}\n";
        let out = parse_ts(source);
        let iface = out.symbols.iter().find(|s| s.kind == SymbolKind::Interface && s.name == "Serializable");
        assert!(iface.is_some(), "should extract interface");
        assert_eq!(iface.unwrap().qualified_name, "app.Serializable");
    }

    #[test]
    fn extracts_type_alias() {
        let source = "type StringMap = Record<string, string>;\n";
        let out = parse_ts(source);
        let alias = out.symbols.iter().find(|s| s.kind == SymbolKind::TypeAlias && s.name == "StringMap");
        assert!(alias.is_some(), "should extract type alias");
    }

    #[test]
    fn extracts_enum() {
        let source = "enum Color {\n  Red,\n  Green,\n  Blue,\n}\n";
        let out = parse_ts(source);
        let en = out.symbols.iter().find(|s| s.kind == SymbolKind::Enum && s.name == "Color");
        assert!(en.is_some(), "should extract enum");
    }

    #[test]
    fn extracts_class_with_methods() {
        let source = r#"
class Animal {
    name: string;
    constructor(name: string) {
        this.name = name;
    }
    greet(): string {
        return this.name;
    }
}
"#;
        let out = parse_ts(source);
        let cls = out.symbols.iter().find(|s| s.kind == SymbolKind::Class && s.name == "Animal");
        assert!(cls.is_some(), "should extract class");

        let methods: Vec<_> = out.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty(), "should extract methods");
    }

    #[test]
    fn extracts_function() {
        let source = "function greet(name: string): string {\n  return `Hello ${name}`;\n}\n";
        let out = parse_ts(source);
        let func = out.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "greet");
        assert!(func.is_some(), "should extract function");
    }

    #[test]
    fn extracts_const_variable() {
        let source = "export const MAX_SIZE: number = 100;\nlet count = 0;\n";
        let out = parse_ts(source);
        let max = out.symbols.iter().find(|s| s.name == "MAX_SIZE");
        assert!(max.is_some(), "should extract MAX_SIZE");
        let count = out.symbols.iter().find(|s| s.name == "count");
        assert!(count.is_some(), "should extract count");
    }

    #[test]
    fn extracts_import_relations() {
        let source = "import { useState } from 'react';\nimport * as path from 'path';\n";
        let out = parse_ts(source);
        let imports: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Imports).collect();
        assert!(imports.len() >= 2, "should have at least 2 import relations, got {}", imports.len());
    }

    #[test]
    fn extracts_inherits_relation() {
        let source = "class Dog extends Animal {\n  bark() {}\n}\n";
        let out = parse_ts(source);
        let inherits: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Inherits).collect();
        assert!(!inherits.is_empty(), "should have Inherits relation");
    }

    #[test]
    fn extracts_implements_relation() {
        let source = "class MyClass implements Serializable {\n  serialize() { return ''; }\n}\n";
        let out = parse_ts(source);
        let impls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Implements).collect();
        assert!(!impls.is_empty(), "should have Implements relation");
    }

    #[test]
    fn tsx_parses_without_error() {
        let source = r#"
function App(): JSX.Element {
    return <div>Hello</div>;
}
"#;
        let out = parse_tsx(source);
        let func = out.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "App");
        assert!(func.is_some(), "should parse TSX and extract function");
    }

    #[test]
    fn extracts_calls() {
        let source = "function main() {\n  console.log('hi');\n  greet('world');\n}\n";
        let out = parse_ts(source);
        let calls: Vec<_> = out.relations.iter().filter(|r| r.kind == RelationKind::Calls).collect();
        assert!(!calls.is_empty(), "should extract call relations");
    }

    #[test]
    fn index_file_uses_directory_name() {
        let source = "export const x = 1;\n";
        let out = parse_file("repo", "components/index.ts", source.as_bytes(), source.len() as u64).unwrap();
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module).unwrap();
        assert_eq!(module.name, "components");
    }

    #[test]
    fn exported_function() {
        let source = "export function doStuff(): void {}\n";
        let out = parse_ts(source);
        let func = out.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "doStuff");
        assert!(func.is_some(), "should extract exported function");
    }
}
