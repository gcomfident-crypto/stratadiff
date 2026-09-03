use std::{fs, process::Command};

use serde_json::{Value, json};
use stratadiff::{Language, analyze_bytes};

fn assert_apply_rejects(report: &Value, before: &[u8], expected_error: &str) {
    let directory = tempfile::tempdir().unwrap();
    let report_path = directory.path().join("report.json");
    let before_path = directory.path().join("before.py");
    let output_path = directory.path().join("rebuilt.py");
    fs::write(&report_path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    fs::write(&before_path, before).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("apply")
        .arg(&report_path)
        .arg(&before_path)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);

    assert!(!result.status.success(), "tampered report was accepted");
    assert!(
        stderr.contains(expected_error),
        "unexpected stderr: {stderr}"
    );
    assert!(
        !output_path.exists(),
        "apply wrote an output before verifying"
    );
}

#[test]
fn apply_rejects_tampered_reports_before_writing_output() {
    let before = b"def value():\n    return 1\n";
    let after = b"def value():\n    return 2\n";
    let report = analyze_bytes(
        before.to_vec(),
        after.to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let report = serde_json::to_value(report).unwrap();

    let mut tampered_schema = report.clone();
    tampered_schema["schema"] = json!("https://example.invalid/report-v2.schema.json");
    assert_apply_rejects(&tampered_schema, before, "unsupported report schema");

    let mut tampered_summary = report;
    tampered_summary["summary"]["structural_changes"] = json!(999_999);
    assert_apply_rejects(
        &tampered_summary,
        before,
        "summary does not match the verified structural claims",
    );
}

#[test]
fn legacy_v1_reports_fail_with_rerun_guidance() {
    let source = b"def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = analyze_bytes(
        source.to_vec(),
        source.to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let mut report = serde_json::to_value(report).unwrap();
    report["schema"] = json!(
        "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json"
    );
    report["ambiguities"][0]
        .as_object_mut()
        .unwrap()
        .remove("constraint");
    report["ambiguities"][0]["predicate"] = json!("shape_equal");

    assert_apply_rejects(
        &report,
        source,
        "rerun StrataDiff on the original snapshots to create a v2 report",
    );
}
