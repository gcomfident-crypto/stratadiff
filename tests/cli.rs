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

fn assert_verify_rejects(report: &Value, before: &[u8], after: &[u8], expected_error: &str) {
    let directory = tempfile::tempdir().unwrap();
    let report_path = directory.path().join("report.json");
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    fs::write(&report_path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    fs::write(&before_path, before).unwrap();
    fs::write(&after_path, after).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);

    assert!(!result.status.success(), "legacy report was accepted");
    assert!(
        stderr.contains(expected_error),
        "unexpected stderr: {stderr}"
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
    let stdout = String::from_utf8(diff.stdout).unwrap();
    assert!(stdout.contains("exact byte edits (1):"), "{stdout}");
    assert!(stdout.contains("- utf8 \"1\""), "{stdout}");
    assert!(stdout.contains("+ utf8 \"2\""), "{stdout}");
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
fn universal_mode_handles_unknown_extensions_and_arbitrary_bytes_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before.blob");
    let after_path = directory.path().join("after.blob");
    let report_path = directory.path().join("change.axd");
    let output_path = directory.path().join("rebuilt.blob");
    let before = [0xff, 0x00, b'a', b'\n'];
    let after = [0xfe, 0x00, b'b', b'\n'];
    fs::write(&before_path, before).unwrap();
    fs::write(&after_path, after).unwrap();

    let implicit = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .arg("--output")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(!implicit.status.success());
    assert!(String::from_utf8_lossy(&implicit.stderr).contains("unsupported extension .blob"));
    assert!(!report_path.exists());

    let diff = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .arg("--language")
        .arg("universal")
        .arg("--output")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["parser"]["language"], "universal");
    assert_eq!(report["parser"]["engine"], "stratadiff-universal");

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
    assert_eq!(fs::read(output_path).unwrap(), after);
}

#[test]
fn terminal_diff_escapes_control_characters_instead_of_emitting_them() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    fs::write(&before_path, b"value=1\n").unwrap();
    fs::write(&after_path, b"value = 1\r\n").unwrap();

    let diff = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let stdout = String::from_utf8(diff.stdout).unwrap();
    assert!(stdout.contains("exact byte edits"), "{stdout}");
    assert!(stdout.contains("\\r"), "{stdout}");
    assert!(!stdout.as_bytes().contains(&b'\r'), "{stdout:?}");
}

#[test]
fn terminal_summary_quotes_untrusted_paths_without_control_sequences() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before\u{1b}[31m\n\u{202e}.py");
    let after_path = directory.path().join("after\u{1b}[0m\n\u{202e}.py");
    fs::write(&before_path, b"value=1\n").unwrap();
    fs::write(&after_path, b"value=2\n").unwrap();

    let diff = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let stdout = String::from_utf8(diff.stdout).unwrap();
    let header = stdout.lines().next().unwrap();
    assert!(header.contains("\\u001b[31m\\n\\u202e.py\""), "{header}");
    assert!(header.contains("\\u001b[0m\\n\\u202e.py\""), "{header}");
    assert!(!stdout.as_bytes().contains(&0x1b), "{stdout:?}");
    assert!(!stdout.contains('\u{202e}'), "{stdout:?}");

    let json = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json_stdout = String::from_utf8(json.stdout).unwrap();
    assert!(!json_stdout.as_bytes().contains(&0x1b), "{json_stdout:?}");
    assert!(!json_stdout.contains('\u{202e}'), "{json_stdout:?}");
    let report: Value = serde_json::from_str(&json_stdout).unwrap();
    assert_eq!(
        report["before"]["path"],
        before_path.to_string_lossy().as_ref()
    );
    assert_eq!(
        report["after"]["path"],
        after_path.to_string_lossy().as_ref()
    );
}

#[test]
fn language_detection_errors_escape_untrusted_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before.bad\u{1b}[31m\n\u{202e}");
    let after_path = directory.path().join("after.py");
    fs::write(&before_path, b"value=1\n").unwrap();
    fs::write(&after_path, b"value=2\n").unwrap();

    let diff = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(!diff.status.success());
    let stderr = String::from_utf8(diff.stderr).unwrap();
    assert!(stderr.contains(".bad\\u{1b}[31m\\n\\u{202e}"), "{stderr}");
    assert!(!stderr.as_bytes().contains(&0x1b), "{stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "{stderr:?}");
}

#[test]
fn argument_errors_escape_untrusted_values() {
    let result = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("diff")
        .arg("before.py")
        .arg("after.py")
        .arg("--language")
        .arg("bad\u{1b}[31m\n\u{202e}")
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("\\u000a"), "{stderr}");
    assert!(stderr.contains("\\u202e"), "{stderr}");
    assert!(!stderr.as_bytes().contains(&0x1b), "{stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "{stderr:?}");
}

#[test]
fn verifier_errors_escape_untrusted_report_fields() {
    let before = b"value=1\n";
    let after = b"value=2\n";
    let mut report = serde_json::to_value(
        analyze_bytes(
            before.to_vec(),
            after.to_vec(),
            "before.py".to_owned(),
            "after.py".to_owned(),
            Language::Python,
        )
        .unwrap(),
    )
    .unwrap();
    report["schema"] = json!("evil\u{1b}[31m\n\u{202e}");

    let directory = tempfile::tempdir().unwrap();
    let report_path = directory.path().join("report.json");
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    fs::write(&before_path, before).unwrap();
    fs::write(&after_path, after).unwrap();

    let verify = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("verify")
        .arg(&report_path)
        .arg(&before_path)
        .arg(&after_path)
        .output()
        .unwrap();
    assert!(!verify.status.success());
    let stderr = String::from_utf8(verify.stderr).unwrap();
    assert!(stderr.contains("evil\\u001b[31m\\u000a\\u202e"), "{stderr}");
    assert!(!stderr.as_bytes().contains(&0x1b), "{stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "{stderr:?}");
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
        "rerun StrataDiff on the original snapshots to create a v3 report",
    );
    assert_verify_rejects(
        &report,
        source,
        source,
        "rerun StrataDiff on the original snapshots to create a v3 report",
    );
}

#[test]
fn legacy_v2_reports_fail_with_rerun_guidance() {
    let source = b"value = 1\n";
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
        "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json"
    );

    assert_apply_rejects(
        &report,
        source,
        "previous parser and patch contracts and cannot be verified as v3",
    );
    assert_verify_rejects(
        &report,
        source,
        source,
        "previous parser and patch contracts and cannot be verified as v3",
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
