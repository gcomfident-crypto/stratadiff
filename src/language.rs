use std::path::Path;

use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tree_sitter::Language as TsLanguage;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    Javascript,
    Typescript,
    Tsx,
    Rust,
    Java,
    Json,
}

impl Language {
    pub fn detect(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow::anyhow!("cannot detect a language for {}", path.display()))?;

        match extension.to_ascii_lowercase().as_str() {
            "py" | "pyi" => Ok(Self::Python),
            "js" | "jsx" | "mjs" | "cjs" => Ok(Self::Javascript),
            "ts" | "mts" | "cts" => Ok(Self::Typescript),
            "tsx" => Ok(Self::Tsx),
            "rs" => Ok(Self::Rust),
            "java" => Ok(Self::Java),
            "json" => Ok(Self::Json),
            _ => {
                bail!("unsupported extension .{extension}; pass --language or use a supported file")
            }
        }
    }

    pub fn parser_language(self) -> TsLanguage {
        match self {
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Javascript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
        }
    }

    pub fn grammar_name(self) -> &'static str {
        match self {
            Self::Python => "tree-sitter-python",
            Self::Javascript => "tree-sitter-javascript",
            Self::Typescript => "tree-sitter-typescript",
            Self::Tsx => "tree-sitter-tsx",
            Self::Rust => "tree-sitter-rust",
            Self::Java => "tree-sitter-java",
            Self::Json => "tree-sitter-json",
        }
    }

    pub fn grammar_version(self) -> &'static str {
        match self {
            Self::Python => "0.25.0",
            Self::Javascript => "0.25.0",
            Self::Typescript | Self::Tsx => "0.23.2",
            Self::Rust => "0.24.2",
            Self::Java => "0.23.5",
            Self::Json => "0.24.8",
        }
    }

    pub fn node_types(self) -> &'static str {
        match self {
            Self::Python => tree_sitter_python::NODE_TYPES,
            Self::Javascript => tree_sitter_javascript::NODE_TYPES,
            Self::Typescript => tree_sitter_typescript::TYPESCRIPT_NODE_TYPES,
            Self::Tsx => tree_sitter_typescript::TSX_NODE_TYPES,
            Self::Rust => tree_sitter_rust::NODE_TYPES,
            Self::Java => tree_sitter_java::NODE_TYPES,
            Self::Json => tree_sitter_json::NODE_TYPES,
        }
    }
}
