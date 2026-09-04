mod json_preflight;
mod limits;
mod patch;
mod verifier;

pub use stratadiff_core::{
    Language, PARSER_RUNTIME_VERSION, PATCH_ALGORITHM, REPORT_ENGINE_VERSION, REPORT_SCHEMA,
    model::*,
};

pub use limits::{VerificationLimits, VerificationStats};
pub use patch::{apply_patch, replay_patch_with_limits};
pub use verifier::{
    decode_report_bytes, verify_and_replay_report_bytes, verify_and_replay_report_with_limits,
    verify_report, verify_report_bytes, verify_report_with_limits,
};
