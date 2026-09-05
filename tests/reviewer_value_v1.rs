use std::{path::PathBuf, process::Command};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn reviewer_value_v1_recomputes_exactly() {
    let root = repository_root();
    let output = Command::new("python3")
        .arg("-B")
        .arg(root.join("tools/reviewer-value-v1/reviewer_value_v1.py"))
        .arg("verify")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reviewer_value_v1_rejects_tampered_results() {
    let root = repository_root();
    let temporary = tempfile::tempdir().unwrap();
    let evaluation_path = temporary.path().join("evaluation.json");
    let mut evaluation: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("benchmarks/reviewer-value-v1/evaluation-v1.0.0.json")).unwrap(),
    )
    .unwrap();
    evaluation["summary"]["current_files_carried"] = serde_json::json!(47);
    std::fs::write(&evaluation_path, serde_json::to_vec(&evaluation).unwrap()).unwrap();

    let output = Command::new("python3")
        .arg("-B")
        .arg(root.join("tools/reviewer-value-v1/reviewer_value_v1.py"))
        .arg("verify")
        .arg("--evaluation")
        .arg(&evaluation_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("reviewer-value evaluation differs from recomputation")
    );
}
