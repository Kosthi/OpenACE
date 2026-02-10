#[cfg(test)]
mod python_tests {
    use oc_core::SymbolKind;
    use oc_parser::parse_file;

    fn parse(source: &str) -> oc_parser::ParseOutput {
        parse_file("test-repo", "src/example.py", source.as_bytes(), source.len() as u64)
            .expect("parse should succeed")
    }

    // --- Symbol extraction ---

    #[test]
    fn extracts_module_symbol() {
        let out = parse("x = 1\n");
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(module.is_some());
        let m = module.unwrap();
        assert_eq!(m.name, "example");
        assert_eq!(m.qualified_name, "example");
    }

    #[test]
    fn extracts_top_level_function() {
        let out = parse("def hello(name: str) -> str:\n    return f'Hello {name}'\n");
        let func = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == "hello");
        assert!(func.is_some(), "should extract function 'hello'");
        let f = func.unwrap();
        assert_eq!(f.qualified_name, "example.hello");
        assert!(f.signature.as_deref().unwrap().contains("def hello"));
    }

    #[test]
    fn extracts_class_with_methods() {
        let source = r#"
class MyClass(Base):
    """A test class."""
    def __init__(self, value):
        self.value = value

    def get_value(self):
        return self.value
"#;
        let out = parse(source);

        let class = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "MyClass");
        assert!(class.is_some(), "should extract class 'MyClass'");
        let c = class.unwrap();
        assert_eq!(c.qualified_name, "example.MyClass");
        assert_eq!(c.doc_comment.as_deref(), Some("A test class."));

        let init = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Method && s.name == "__init__");
        assert!(init.is_some(), "should extract __init__ method");
        assert_eq!(
            init.unwrap().qualified_name,
            "example.MyClass.__init__"
        );

        let get_val = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Method && s.name == "get_value");
        assert!(get_val.is_some(), "should extract get_value method");
        assert_eq!(
            get_val.unwrap().qualified_name,
            "example.MyClass.get_value"
        );
    }

    #[test]
    fn extracts_nested_class() {
        let source = r#"
class Outer:
    class Inner:
        def method(self):
            pass
"#;
        let out = parse(source);
        let inner = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Inner");
        assert!(inner.is_some());
        assert_eq!(inner.unwrap().qualified_name, "example.Outer.Inner");

        let method = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Method && s.name == "method");
        assert!(method.is_some());
        assert_eq!(
            method.unwrap().qualified_name,
            "example.Outer.Inner.method"
        );
    }

    #[test]
    fn extracts_constants_and_variables() {
        let source = "MAX_SIZE = 100\nmy_var = 'hello'\n";
        let out = parse(source);

        let constant = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Constant && s.name == "MAX_SIZE");
        assert!(constant.is_some(), "should extract constant MAX_SIZE");

        let var = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Variable && s.name == "my_var");
        assert!(var.is_some(), "should extract variable my_var");
    }

    #[test]
    fn extracts_docstring() {
        let source = r#"
class Documented:
    """This is the docstring."""
    pass
"#;
        let out = parse(source);
        let cls = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name == "Documented");
        assert!(cls.is_some());
        assert_eq!(
            cls.unwrap().doc_comment.as_deref(),
            Some("This is the docstring.")
        );
    }

    #[test]
    fn extracts_function_signature_with_return_type() {
        let source = "def greet(name: str) -> str:\n    return name\n";
        let out = parse(source);
        let func = out
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        let sig = func.signature.as_deref().unwrap();
        assert!(sig.contains("def greet"));
        assert!(sig.contains("-> str"));
    }

    // --- Relation extraction ---

    #[test]
    fn extracts_import_relations() {
        let source = "import os\nfrom pathlib import Path\n";
        let out = parse(source);

        let imports: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Imports)
            .collect();
        assert!(imports.len() >= 2, "should have at least 2 import relations");
    }

    #[test]
    fn extracts_aliased_import() {
        let source = "import numpy as np\nfrom collections import OrderedDict as OD\n";
        let out = parse(source);

        let imports: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Imports)
            .collect();
        assert!(
            imports.len() >= 2,
            "should have at least 2 import relations for aliased imports, got {}",
            imports.len()
        );
    }

    #[test]
    fn extracts_wildcard_import() {
        let source = "from os.path import *\n";
        let out = parse(source);

        let imports: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Imports)
            .collect();
        assert!(
            !imports.is_empty(),
            "wildcard import should produce at least 1 Imports relation"
        );
    }

    #[test]
    fn extracts_inherits_relation() {
        let source = "class Child(Parent):\n    pass\n";
        let out = parse(source);

        let inherits: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Inherits)
            .collect();
        assert_eq!(inherits.len(), 1, "should have exactly 1 Inherits relation");
    }

    #[test]
    fn extracts_contains_relations() {
        let source = "class Foo:\n    def bar(self):\n        pass\n";
        let out = parse(source);

        let contains: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Contains)
            .collect();
        // Module contains Foo, Foo contains bar
        assert!(
            contains.len() >= 2,
            "should have at least 2 Contains relations, got {}",
            contains.len()
        );
    }

    #[test]
    fn extracts_calls_relations() {
        let source = "def foo():\n    bar()\n    baz(1, 2)\n";
        let out = parse(source);

        let calls: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Calls)
            .collect();
        assert!(
            calls.len() >= 2,
            "should have at least 2 Calls relations, got {}",
            calls.len()
        );
    }

    // --- Edge cases ---

    #[test]
    fn handles_syntax_errors_gracefully() {
        let source = "def incomplete(:\n    pass\n";
        let out = parse(source);
        assert!(!out.symbols.is_empty());
    }

    #[test]
    fn handles_empty_file() {
        let out = parse("");
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(module.is_some());
    }

    #[test]
    fn rejects_oversized_file() {
        let content = b"x = 1";
        let err = oc_parser::parse_file("repo", "big.py", content, 2_000_000);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_oversized_content() {
        // Content length exceeds limit even if declared file_size is small
        let content = vec![b'a'; 1_048_577]; // 1 byte over limit
        let err = oc_parser::parse_file("repo", "big.py", &content, 1);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_binary_content() {
        let content = b"def foo():\x00    pass\n";
        let err = oc_parser::parse_file("repo", "test.py", content, content.len() as u64);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_unsupported_extension() {
        let err = oc_parser::parse_file("repo", "test.txt", b"hello", 5);
        assert!(err.is_err());
    }

    #[test]
    fn symbol_ids_are_deterministic() {
        let source = "def foo():\n    pass\n";
        let out1 = parse(source);
        let out2 = parse(source);
        let ids1: Vec<_> = out1.symbols.iter().map(|s| s.id).collect();
        let ids2: Vec<_> = out2.symbols.iter().map(|s| s.id).collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn body_hash_changes_with_content() {
        let src1 = "def foo():\n    return 1\n";
        let src2 = "def foo():\n    return 2\n";
        let out1 = parse(src1);
        let out2 = parse(src2);
        let h1 = out1.symbols.iter().find(|s| s.name == "foo").unwrap().body_hash;
        let h2 = out2.symbols.iter().find(|s| s.name == "foo").unwrap().body_hash;
        assert_ne!(h1, h2);
    }

    #[test]
    fn handles_decorated_functions() {
        let source = "@staticmethod\ndef helper():\n    pass\n";
        let out = parse(source);
        let func = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == "helper");
        assert!(func.is_some(), "should extract decorated function");
    }

    #[test]
    fn decorated_method_in_class() {
        let source = r#"
class Foo:
    @property
    def name(self):
        return self._name
"#;
        let out = parse(source);
        let method = out
            .symbols
            .iter()
            .find(|s| s.name == "name" && s.kind == SymbolKind::Method);
        assert!(method.is_some(), "decorated function in class should be Method");
    }

    #[test]
    fn handles_init_file() {
        let source = "VERSION = '1.0'\n";
        let out = oc_parser::parse_file(
            "test-repo",
            "mypackage/__init__.py",
            source.as_bytes(),
            source.len() as u64,
        )
        .expect("parse should succeed");
        let module = out.symbols.iter().find(|s| s.kind == SymbolKind::Module).unwrap();
        assert_eq!(module.name, "mypackage");
    }

    #[test]
    fn qualified_names_for_nested_scopes() {
        let source = r#"
class Outer:
    class Middle:
        class Inner:
            def deep_method(self):
                pass
"#;
        let out = parse(source);
        let method = out
            .symbols
            .iter()
            .find(|s| s.name == "deep_method")
            .unwrap();
        assert_eq!(
            method.qualified_name,
            "example.Outer.Middle.Inner.deep_method"
        );
    }

    #[test]
    fn multiple_base_classes() {
        let source = "class Multi(Base1, Base2, Base3):\n    pass\n";
        let out = parse(source);
        let inherits: Vec<_> = out
            .relations
            .iter()
            .filter(|r| r.kind == oc_core::RelationKind::Inherits)
            .collect();
        assert_eq!(
            inherits.len(),
            3,
            "should have 3 Inherits relations for 3 base classes"
        );
    }

    #[test]
    fn lambda_skipped_as_call_target_only() {
        let source = "f = lambda x: x + 1\n";
        let out = parse(source);
        let lambdas: Vec<_> = out
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name.contains("lambda"))
            .collect();
        assert!(
            lambdas.is_empty(),
            "lambdas should not be extracted as function symbols"
        );
    }

    #[test]
    fn function_named_underscore_is_extracted() {
        let source = "def _():\n    pass\n";
        let out = parse(source);
        let func = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == "_");
        assert!(func.is_some(), "function named '_' should be extracted");
    }

    #[test]
    fn nested_function_in_if_block() {
        let source = r#"
def outer():
    if True:
        def inner():
            pass
"#;
        let out = parse(source);
        let inner = out
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == "inner");
        assert!(
            inner.is_some(),
            "nested function inside if block should be extracted"
        );
        assert_eq!(inner.unwrap().qualified_name, "example.outer.inner");
    }

    #[test]
    fn nested_function_in_try_block() {
        let source = r#"
def main():
    try:
        def helper():
            pass
    except Exception:
        def error_handler():
            pass
"#;
        let out = parse(source);
        let helper = out
            .symbols
            .iter()
            .find(|s| s.name == "helper");
        assert!(helper.is_some(), "function in try block should be extracted");

        let handler = out
            .symbols
            .iter()
            .find(|s| s.name == "error_handler");
        assert!(handler.is_some(), "function in except block should be extracted");
    }

    #[test]
    fn nested_function_in_for_loop() {
        let source = r#"
def process():
    for item in items:
        def transform(x):
            return x
"#;
        let out = parse(source);
        let transform = out
            .symbols
            .iter()
            .find(|s| s.name == "transform");
        assert!(transform.is_some(), "function in for loop should be extracted");
    }
}
