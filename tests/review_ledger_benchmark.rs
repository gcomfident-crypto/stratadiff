use std::{path::PathBuf, process::Command};

#[test]
fn offline_runner_reports_every_manifest_case_and_rejects_passport_tampering() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_directory = tempfile::tempdir().unwrap();
    let result_path = output_directory.path().join("result.json");
    let output = Command::new("python3")
        .arg(repository.join("tools/review-ledger-v1/runner.py"))
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("--output")
        .arg(&result_path)
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "runner stdout:\n{}\nrunner stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(result_path).unwrap()).unwrap();
    assert_eq!(report["summary"]["FAIL"], 0);
    assert_eq!(report["summary"]["PASS"], 20);
    assert_eq!(report["summary"]["SKIP"], 0);
    assert_eq!(report["control_summary"]["PASS"], 1);
    assert_eq!(report["control_summary"]["FAIL"], 0);
    assert_eq!(report["controls"][0]["id"], "passport-tamper-detected");
    assert_eq!(report["controls"][0]["status"], "PASS");
}
