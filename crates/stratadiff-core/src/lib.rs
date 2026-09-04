pub mod language;
pub mod model;
#[doc(hidden)]
pub mod syntax;

pub use language::Language;
pub use model::*;
pub use syntax::{ParseLimits, parse_with_limits};

pub const REPORT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json";
pub const REPORT_ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PARSER_RUNTIME_VERSION: &str = "0.27.0";
