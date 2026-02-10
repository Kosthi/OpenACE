use serde::{Deserialize, Serialize};

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Language {
    Python = 0,
    TypeScript = 1,
    JavaScript = 2,
    Rust = 3,
    Go = 4,
    Java = 5,
}

impl Language {
    /// Map a file extension to a Language.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Self::Python),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::TypeScript),
            "js" => Some(Self::JavaScript),
            "jsx" => Some(Self::JavaScript),
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            _ => None,
        }
    }

    /// The separator used in qualified names for this language.
    pub fn native_separator(self) -> &'static str {
        match self {
            Self::Rust => "::",
            Self::Go => ".",
            Self::Python | Self::TypeScript | Self::JavaScript | Self::Java => ".",
        }
    }

    pub fn from_ordinal(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Python),
            1 => Some(Self::TypeScript),
            2 => Some(Self::JavaScript),
            3 => Some(Self::Rust),
            4 => Some(Self::Go),
            5 => Some(Self::Java),
            _ => None,
        }
    }

    pub fn ordinal(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Java => "java",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("java"), Some(Language::Java));
        assert_eq!(Language::from_extension("txt"), None);
    }

    #[test]
    fn ordinal_round_trip() {
        for n in 0..=5u8 {
            let lang = Language::from_ordinal(n).unwrap();
            assert_eq!(lang.ordinal(), n);
        }
        assert!(Language::from_ordinal(6).is_none());
    }
}
