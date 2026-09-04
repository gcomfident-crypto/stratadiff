use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tree_sitter::Language as TsLanguage;

pub const UNIVERSAL_PARSER_ENGINE: &str = "stratadiff-universal";
pub const UNIVERSAL_PARSER_RUNTIME_VERSION: &str = "1.0.0";
pub const UNIVERSAL_GRAMMAR_NAME: &str = "byte-lines-token-runs";
pub const UNIVERSAL_GRAMMAR_VERSION: &str = "1.0.0";
pub const UNIVERSAL_GRAMMAR_ABI: usize = 1;
pub const UNIVERSAL_COORDINATE_UNIT: &str = "zero_based_row_byte_column";
pub const UNIVERSAL_NODE_TYPES: &str = r#"[
  {"type":"universal_file","named":true},
  {"type":"universal_line","named":true},
  {"type":"universal_ascii_word","named":true},
  {"type":"universal_whitespace","named":true},
  {"type":"universal_line_feed","named":true},
  {"type":"universal_ascii_punctuation","named":true},
  {"type":"universal_opaque_bytes","named":true}
]"#;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Universal,
    Python,
    Javascript,
    Typescript,
    Tsx,
    Rust,
    Java,
    Json,
    C,
    Cpp,
    CSharp,
    Go,
    Ruby,
    Bash,
    Php,
    Html,
    Css,
    Yaml,
    Toml,
    Markdown,
    Kotlin,
    Swift,
    Lua,
    Scala,
    R,
    Elixir,
    Haskell,
    Ocaml,
    OcamlInterface,
    Zig,
}

impl Language {
    pub fn detect(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot detect a language from the file extension; pass --language explicitly"
                )
            })?;

        match extension.to_ascii_lowercase().as_str() {
            "py" | "pyi" => Ok(Self::Python),
            "js" | "jsx" | "mjs" | "cjs" => Ok(Self::Javascript),
            "ts" | "mts" | "cts" => Ok(Self::Typescript),
            "tsx" => Ok(Self::Tsx),
            "rs" => Ok(Self::Rust),
            "java" => Ok(Self::Java),
            "json" => Ok(Self::Json),
            "c" => Ok(Self::C),
            "h" => bail!(
                "ambiguous extension .h could be C or C++; pass --language c or --language cpp"
            ),
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Ok(Self::Cpp),
            "cs" => Ok(Self::CSharp),
            "go" => Ok(Self::Go),
            "rb" => Ok(Self::Ruby),
            "sh" | "bash" => Ok(Self::Bash),
            "php" | "phtml" => Ok(Self::Php),
            "html" | "htm" => Ok(Self::Html),
            "css" => Ok(Self::Css),
            "yaml" | "yml" => Ok(Self::Yaml),
            "toml" => Ok(Self::Toml),
            "md" | "markdown" | "mdown" | "mkd" => Ok(Self::Markdown),
            "kt" | "kts" => Ok(Self::Kotlin),
            "swift" => Ok(Self::Swift),
            "lua" => Ok(Self::Lua),
            "scala" | "sc" => Ok(Self::Scala),
            "r" => Ok(Self::R),
            "ex" | "exs" => Ok(Self::Elixir),
            "hs" => Ok(Self::Haskell),
            "ml" => Ok(Self::Ocaml),
            "mli" => Ok(Self::OcamlInterface),
            "zig" => Ok(Self::Zig),
            "m" => bail!(
                "ambiguous extension .m could be Objective-C or MATLAB; select a supported parser mode explicitly (for example --language universal)"
            ),
            _ => {
                let escaped_extension = extension.escape_default();
                bail!(
                    "unsupported extension .{escaped_extension}; pass --language or use a supported file"
                )
            }
        }
    }

    pub fn tree_sitter_language(self) -> Option<TsLanguage> {
        match self {
            Self::Universal => None,
            Self::Python => Some(tree_sitter_python::LANGUAGE.into()),
            Self::Javascript => Some(tree_sitter_javascript::LANGUAGE.into()),
            Self::Typescript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            Self::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            Self::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            Self::Java => Some(tree_sitter_java::LANGUAGE.into()),
            Self::Json => Some(tree_sitter_json::LANGUAGE.into()),
            Self::C => Some(tree_sitter_c::LANGUAGE.into()),
            Self::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
            Self::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            Self::Go => Some(tree_sitter_go::LANGUAGE.into()),
            Self::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
            Self::Bash => Some(tree_sitter_bash::LANGUAGE.into()),
            Self::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
            Self::Html => Some(tree_sitter_html::LANGUAGE.into()),
            Self::Css => Some(tree_sitter_css::LANGUAGE.into()),
            Self::Yaml => Some(tree_sitter_yaml::LANGUAGE.into()),
            Self::Toml => Some(tree_sitter_toml_ng::LANGUAGE.into()),
            Self::Markdown => Some(tree_sitter_md::LANGUAGE.into()),
            Self::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            Self::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
            Self::Lua => Some(tree_sitter_lua::LANGUAGE.into()),
            Self::Scala => Some(tree_sitter_scala::LANGUAGE.into()),
            Self::R => Some(tree_sitter_r::LANGUAGE.into()),
            Self::Elixir => Some(tree_sitter_elixir::LANGUAGE.into()),
            Self::Haskell => Some(tree_sitter_haskell::LANGUAGE.into()),
            Self::Ocaml => Some(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
            Self::OcamlInterface => Some(tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into()),
            Self::Zig => Some(tree_sitter_zig::LANGUAGE.into()),
        }
    }

    pub fn parser_engine(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_PARSER_ENGINE,
            _ => "tree-sitter",
        }
    }

    pub fn parser_runtime_version(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_PARSER_RUNTIME_VERSION,
            _ => crate::PARSER_RUNTIME_VERSION,
        }
    }

    pub fn grammar_name(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_GRAMMAR_NAME,
            Self::Python => "tree-sitter-python",
            Self::Javascript => "tree-sitter-javascript",
            Self::Typescript => "tree-sitter-typescript",
            Self::Tsx => "tree-sitter-tsx",
            Self::Rust => "tree-sitter-rust",
            Self::Java => "tree-sitter-java",
            Self::Json => "tree-sitter-json",
            Self::C => "tree-sitter-c",
            Self::Cpp => "tree-sitter-cpp",
            Self::CSharp => "tree-sitter-c-sharp",
            Self::Go => "tree-sitter-go",
            Self::Ruby => "tree-sitter-ruby",
            Self::Bash => "tree-sitter-bash",
            Self::Php => "tree-sitter-php",
            Self::Html => "tree-sitter-html",
            Self::Css => "tree-sitter-css",
            Self::Yaml => "tree-sitter-yaml",
            Self::Toml => "tree-sitter-toml-ng",
            Self::Markdown => "tree-sitter-markdown",
            Self::Kotlin => "tree-sitter-kotlin-ng",
            Self::Swift => "tree-sitter-swift",
            Self::Lua => "tree-sitter-lua",
            Self::Scala => "tree-sitter-scala",
            Self::R => "tree-sitter-r",
            Self::Elixir => "tree-sitter-elixir",
            Self::Haskell => "tree-sitter-haskell",
            Self::Ocaml => "tree-sitter-ocaml",
            Self::OcamlInterface => "tree-sitter-ocaml-interface",
            Self::Zig => "tree-sitter-zig",
        }
    }

    pub fn grammar_version(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_GRAMMAR_VERSION,
            Self::Python => "0.25.0",
            Self::Javascript => "0.25.0",
            Self::Typescript | Self::Tsx => "0.23.2",
            Self::Rust => "0.24.2",
            Self::Java => "0.23.5",
            Self::Json => "0.24.8",
            Self::C => "0.24.2",
            Self::Cpp => "0.23.4",
            Self::CSharp => "0.23.5",
            Self::Go => "0.25.0",
            Self::Ruby => "0.23.1",
            Self::Bash => "0.25.1",
            Self::Php => "0.24.2",
            Self::Html => "0.23.2",
            Self::Css => "0.25.0",
            Self::Yaml => "0.7.2",
            Self::Toml => "0.7.0",
            Self::Markdown => "0.5.3",
            Self::Kotlin => "1.1.0",
            Self::Swift => "0.7.3",
            Self::Lua => "0.5.0",
            Self::Scala => "0.26.2",
            Self::R => "1.3.0",
            Self::Elixir => "0.3.5",
            Self::Haskell => "0.23.1",
            Self::Ocaml | Self::OcamlInterface => "0.25.0",
            Self::Zig => "1.1.2",
        }
    }

    pub fn grammar_abi(self) -> usize {
        match self {
            Self::Universal => UNIVERSAL_GRAMMAR_ABI,
            _ => self
                .tree_sitter_language()
                .expect("every native language has a Tree-sitter grammar")
                .abi_version(),
        }
    }

    pub fn node_types(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_NODE_TYPES,
            Self::Python => tree_sitter_python::NODE_TYPES,
            Self::Javascript => tree_sitter_javascript::NODE_TYPES,
            Self::Typescript => tree_sitter_typescript::TYPESCRIPT_NODE_TYPES,
            Self::Tsx => tree_sitter_typescript::TSX_NODE_TYPES,
            Self::Rust => tree_sitter_rust::NODE_TYPES,
            Self::Java => tree_sitter_java::NODE_TYPES,
            Self::Json => tree_sitter_json::NODE_TYPES,
            Self::C => tree_sitter_c::NODE_TYPES,
            Self::Cpp => tree_sitter_cpp::NODE_TYPES,
            Self::CSharp => tree_sitter_c_sharp::NODE_TYPES,
            Self::Go => tree_sitter_go::NODE_TYPES,
            Self::Ruby => tree_sitter_ruby::NODE_TYPES,
            Self::Bash => tree_sitter_bash::NODE_TYPES,
            Self::Php => tree_sitter_php::PHP_NODE_TYPES,
            Self::Html => tree_sitter_html::NODE_TYPES,
            Self::Css => tree_sitter_css::NODE_TYPES,
            Self::Yaml => tree_sitter_yaml::NODE_TYPES,
            Self::Toml => tree_sitter_toml_ng::NODE_TYPES,
            Self::Markdown => tree_sitter_md::NODE_TYPES_BLOCK,
            Self::Kotlin => tree_sitter_kotlin_ng::NODE_TYPES,
            Self::Swift => tree_sitter_swift::NODE_TYPES,
            Self::Lua => tree_sitter_lua::NODE_TYPES,
            Self::Scala => tree_sitter_scala::NODE_TYPES,
            Self::R => tree_sitter_r::NODE_TYPES,
            Self::Elixir => tree_sitter_elixir::NODE_TYPES,
            Self::Haskell => tree_sitter_haskell::NODE_TYPES,
            Self::Ocaml => tree_sitter_ocaml::OCAML_NODE_TYPES,
            Self::OcamlInterface => tree_sitter_ocaml::INTERFACE_NODE_TYPES,
            Self::Zig => tree_sitter_zig::NODE_TYPES,
        }
    }

    pub fn coordinate_unit(self) -> &'static str {
        match self {
            Self::Universal => UNIVERSAL_COORDINATE_UNIT,
            _ => "zero_based_row_utf8_byte_column",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::Language;

    #[test]
    fn universal_is_explicit_only() {
        assert!(Language::detect(Path::new("before.unknown")).is_err());
        assert!(Language::detect(Path::new("Makefile")).is_err());
        assert_eq!(
            Language::detect(Path::new("before.PY")).unwrap(),
            Language::Python
        );
    }

    #[test]
    fn native_languages_are_detected_by_unambiguous_extensions() {
        let cases = [
            ("file.c", Language::C),
            ("file.cpp", Language::Cpp),
            ("file.hpp", Language::Cpp),
            ("file.cs", Language::CSharp),
            ("file.go", Language::Go),
            ("file.rb", Language::Ruby),
            ("file.sh", Language::Bash),
            ("file.php", Language::Php),
            ("file.html", Language::Html),
            ("file.css", Language::Css),
            ("file.yml", Language::Yaml),
            ("file.toml", Language::Toml),
            ("file.md", Language::Markdown),
            ("file.kt", Language::Kotlin),
            ("file.swift", Language::Swift),
            ("file.lua", Language::Lua),
            ("file.scala", Language::Scala),
            ("file.R", Language::R),
            ("file.exs", Language::Elixir),
            ("file.hs", Language::Haskell),
            ("file.ml", Language::Ocaml),
            ("file.mli", Language::OcamlInterface),
            ("file.zig", Language::Zig),
        ];

        for (path, language) in cases {
            assert_eq!(
                Language::detect(Path::new(path)).unwrap(),
                language,
                "{path}"
            );
        }
    }

    #[test]
    fn native_language_manifests_match_the_pinned_grammars() {
        let cases = [
            (Language::Python, "tree-sitter-python", "0.25.0"),
            (Language::Javascript, "tree-sitter-javascript", "0.25.0"),
            (Language::Typescript, "tree-sitter-typescript", "0.23.2"),
            (Language::Tsx, "tree-sitter-tsx", "0.23.2"),
            (Language::Rust, "tree-sitter-rust", "0.24.2"),
            (Language::Java, "tree-sitter-java", "0.23.5"),
            (Language::Json, "tree-sitter-json", "0.24.8"),
            (Language::C, "tree-sitter-c", "0.24.2"),
            (Language::Cpp, "tree-sitter-cpp", "0.23.4"),
            (Language::CSharp, "tree-sitter-c-sharp", "0.23.5"),
            (Language::Go, "tree-sitter-go", "0.25.0"),
            (Language::Ruby, "tree-sitter-ruby", "0.23.1"),
            (Language::Bash, "tree-sitter-bash", "0.25.1"),
            (Language::Php, "tree-sitter-php", "0.24.2"),
            (Language::Html, "tree-sitter-html", "0.23.2"),
            (Language::Css, "tree-sitter-css", "0.25.0"),
            (Language::Yaml, "tree-sitter-yaml", "0.7.2"),
            (Language::Toml, "tree-sitter-toml-ng", "0.7.0"),
            (Language::Markdown, "tree-sitter-markdown", "0.5.3"),
            (Language::Kotlin, "tree-sitter-kotlin-ng", "1.1.0"),
            (Language::Swift, "tree-sitter-swift", "0.7.3"),
            (Language::Lua, "tree-sitter-lua", "0.5.0"),
            (Language::Scala, "tree-sitter-scala", "0.26.2"),
            (Language::R, "tree-sitter-r", "1.3.0"),
            (Language::Elixir, "tree-sitter-elixir", "0.3.5"),
            (Language::Haskell, "tree-sitter-haskell", "0.23.1"),
            (Language::Ocaml, "tree-sitter-ocaml", "0.25.0"),
            (
                Language::OcamlInterface,
                "tree-sitter-ocaml-interface",
                "0.25.0",
            ),
            (Language::Zig, "tree-sitter-zig", "1.1.2"),
        ];

        for (language, grammar_name, grammar_version) in cases {
            assert!(language.tree_sitter_language().is_some());
            assert_eq!(language.parser_engine(), "tree-sitter");
            assert_eq!(language.grammar_name(), grammar_name);
            assert_eq!(language.grammar_version(), grammar_version);
            assert!(!language.node_types().is_empty());
            assert!(language.grammar_abi() > 0);
            assert_eq!(
                language.coordinate_unit(),
                "zero_based_row_utf8_byte_column"
            );
        }
    }

    #[test]
    fn ambiguous_extensions_are_not_guessed() {
        let header = Language::detect(Path::new("value.h")).unwrap_err();
        assert!(header.to_string().contains("could be C or C++"));

        let implementation = Language::detect(Path::new("value.m")).unwrap_err();
        assert!(implementation.to_string().contains("Objective-C or MATLAB"));
    }
}
