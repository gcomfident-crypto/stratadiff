use std::{fs, fs::File, process::Command};

use serde_json::{Value, json};
use stratadiff::{Language, VerificationLimits, analyze_bytes};

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
fn emitted_report_verifies_and_applies_with_default_limits() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    let report_path = directory.path().join("change.axd");
    let output_path = directory.path().join("rebuilt.py");
    fs::write(&before_path, b"def value():\n    return 1\n").unwrap();
    fs::write(&after_path, b"def value():\n    return 2\n").unwrap();

    let diff = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .arg("--output")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    assert!(!fs::read(&report_path).unwrap().contains(&b'\n'));

    let verify = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );

    let apply = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("apply")
        .arg(&report_path)
        .arg(&before_path)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert_eq!(
        fs::read(output_path).unwrap(),
        fs::read(after_path).unwrap()
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

#[test]
fn verify_and_apply_reject_oversized_inputs_before_verification_or_output() {
    let limits = VerificationLimits::default();
    let directory = tempfile::tempdir().unwrap();
    let report_path = directory.path().join("report.json");
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    let output_path = directory.path().join("rebuilt.py");
    fs::write(&before_path, b"").unwrap();
    fs::write(&after_path, b"").unwrap();

    File::create(&report_path)
        .unwrap()
        .set_len(u64::try_from(limits.max_report_bytes + 1).unwrap())
        .unwrap();
    let oversized_report = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(!oversized_report.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_report.stderr).contains("report bytes limit exceeded")
    );

    let report = analyze_bytes(
        Vec::new(),
        Vec::new(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    File::create(&before_path)
        .unwrap()
        .set_len(u64::try_from(limits.max_source_bytes + 1).unwrap())
        .unwrap();
    let oversized_before = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(!oversized_before.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_before.stderr)
            .contains("before source bytes limit exceeded")
    );

    fs::write(&before_path, b"").unwrap();
    File::create(&after_path)
        .unwrap()
        .set_len(u64::try_from(limits.max_source_bytes + 1).unwrap())
        .unwrap();
    let oversized_after = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(!oversized_after.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_after.stderr)
            .contains("after source bytes limit exceeded")
    );

    File::create(&before_path)
        .unwrap()
        .set_len(u64::try_from(limits.max_source_bytes + 1).unwrap())
        .unwrap();
    let oversized_apply = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("apply")
        .arg(&report_path)
        .arg(&before_path)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(!oversized_apply.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_apply.stderr)
            .contains("before source bytes limit exceeded")
    );
    assert!(!output_path.exists(), "apply wrote an oversized input");
}
