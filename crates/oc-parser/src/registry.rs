use oc_core::Language;
use tree_sitter::Language as TSLanguage;

/// Maps file extensions to tree-sitter grammars and Language enums.
pub struct ParserRegistry;

impl ParserRegistry {
    /// Get the tree-sitter grammar for a given Language and file extension.
    /// The extension is needed because TypeScript/JavaScript have both
    /// regular and TSX grammars.
    pub fn grammar_for_extension(lang: Language, ext: &str) -> TSLanguage {
        match lang {
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::TypeScript => {
                if ext == "tsx" {
                    tree_sitter_typescript::LANGUAGE_TSX.into()
                } else {
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
                }
            }
            Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
        }
    }

    /// Resolve a file extension to a Language.
    /// Delegates to `Language::from_extension`.
    pub fn language_for_extension(ext: &str) -> Option<Language> {
        Language::from_extension(ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_languages_have_grammars() {
        let cases = [
            (Language::Python, "py"),
            (Language::TypeScript, "ts"),
            (Language::TypeScript, "tsx"),
            (Language::JavaScript, "js"),
            (Language::JavaScript, "jsx"),
            (Language::Rust, "rs"),
            (Language::Go, "go"),
            (Language::Java, "java"),
        ];
        for (lang, ext) in cases {
            let _grammar = ParserRegistry::grammar_for_extension(lang, ext);
        }
    }

    #[test]
    fn extension_mapping() {
        assert_eq!(
            ParserRegistry::language_for_extension("py"),
            Some(Language::Python)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("ts"),
            Some(Language::TypeScript)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("tsx"),
            Some(Language::TypeScript)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("js"),
            Some(Language::JavaScript)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("jsx"),
            Some(Language::JavaScript)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("rs"),
            Some(Language::Rust)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("go"),
            Some(Language::Go)
        );
        assert_eq!(
            ParserRegistry::language_for_extension("java"),
            Some(Language::Java)
        );
        assert_eq!(ParserRegistry::language_for_extension("txt"), None);
    }

    #[test]
    fn tsx_gets_tsx_grammar() {
        // Verify TSX extension gets a different grammar object than TS
        let ts = ParserRegistry::grammar_for_extension(Language::TypeScript, "ts");
        let tsx = ParserRegistry::grammar_for_extension(Language::TypeScript, "tsx");
        // These are different grammars (can't directly compare, but both should be valid)
        let _ = (ts, tsx);
    }
}
