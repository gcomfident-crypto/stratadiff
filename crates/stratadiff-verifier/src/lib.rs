mod patch;
mod verifier;

pub use stratadiff_core::{
    Language, PARSER_RUNTIME_VERSION, REPORT_ENGINE_VERSION, REPORT_SCHEMA, model::*,
};

pub use patch::apply_patch;
pub use verifier::verify_report;
