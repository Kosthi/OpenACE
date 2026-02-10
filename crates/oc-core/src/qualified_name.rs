use crate::language::Language;

/// Utilities for qualified name normalization and rendering.
pub struct QualifiedName;

impl QualifiedName {
    /// Normalize a language-native qualified name to dot-separated canonical form.
    ///
    /// - Rust: `std::collections::HashMap` → `std.collections.HashMap`
    /// - Go: `net/http.Client.Do` → `net.http.Client.Do`
    /// - Python/TS/JS/Java: identity (already dot-separated)
    pub fn normalize(name: &str, language: Language) -> String {
        match language {
            Language::Rust => name.replace("::", "."),
            Language::Go => name.replace('/', "."),
            Language::Python | Language::TypeScript | Language::JavaScript | Language::Java => {
                name.to_string()
            }
        }
    }

    /// Render a canonical dot-separated qualified name in language-native form.
    pub fn to_native(canonical: &str, language: Language) -> String {
        match language {
            Language::Rust => canonical.replace('.', "::"),
            // Go uses `/` for package paths but `.` for member access.
            // Without package boundary info, we keep dots (safe display form).
            Language::Go => canonical.to_string(),
            Language::Python | Language::TypeScript | Language::JavaScript | Language::Java => {
                canonical.to_string()
            }
        }
    }

    /// Join scope segments into a canonical dot-separated qualified name.
    pub fn join(segments: &[&str]) -> String {
        segments.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rust() {
        assert_eq!(
            QualifiedName::normalize("std::collections::HashMap", Language::Rust),
            "std.collections.HashMap"
        );
    }

    #[test]
    fn normalize_go() {
        assert_eq!(
            QualifiedName::normalize("net/http.Client.Do", Language::Go),
            "net.http.Client.Do"
        );
    }

    #[test]
    fn normalize_python_identity() {
        assert_eq!(
            QualifiedName::normalize("module.Class.method", Language::Python),
            "module.Class.method"
        );
    }

    #[test]
    fn to_native_rust() {
        assert_eq!(
            QualifiedName::to_native("std.collections.HashMap", Language::Rust),
            "std::collections::HashMap"
        );
    }

    #[test]
    fn round_trip_rust() {
        let native = "std::collections::HashMap";
        let canonical = QualifiedName::normalize(native, Language::Rust);
        let back = QualifiedName::to_native(&canonical, Language::Rust);
        assert_eq!(back, native);
    }

    #[test]
    fn join_segments() {
        assert_eq!(QualifiedName::join(&["module", "Class", "method"]), "module.Class.method");
    }
}
