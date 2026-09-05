use std::{path::PathBuf, process::Command};

use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn assert_success(output: std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: std::process::Output, expected_stderr: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "tampered evaluation unexpectedly passed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
    assert!(
        stderr.contains(expected_stderr),
        "tampered evaluation failed for the wrong reason\nstderr:\n{stderr}"
    );
}

fn run_verifier(verifier: &PathBuf, evaluation: &PathBuf) -> std::process::Output {
    Command::new("python3")
        .arg("-B")
        .arg(verifier)
        .arg("--evaluation")
        .arg(evaluation)
        .output()
        .unwrap()
}

fn case_mut<'a>(evaluation: &'a mut Value, id: &str) -> &'a mut Value {
    evaluation["cases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|case| case["id"] == id)
        .unwrap()
}

#[test]
fn controlled_review_delta_contract_passes_end_to_end() {
    let root = repository_root();
    let temporary = tempfile::tempdir().unwrap();
    let evaluation = temporary.path().join("evaluation.json");
    let runner = root.join("tools/review-delta-v1/run.py");
    let verifier = root.join("tools/review-delta-v1/verify.py");

    assert_success(
        Command::new("python3")
            .arg("-B")
            .arg(runner)
            .arg("--stratadiff")
            .arg(env!("CARGO_BIN_EXE_stratadiff"))
            .arg("--output")
            .arg(&evaluation)
            .output()
            .unwrap(),
    );
    assert_success(run_verifier(&verifier, &evaluation));

    let original: Value = serde_json::from_slice(&std::fs::read(&evaluation).unwrap()).unwrap();

    let mut forged_full_identity = original.clone();
    let full_case = case_mut(
        &mut forged_full_identity,
        "pure-rebase-carries-reviewed-edit",
    );
    let forged_blob = json!("0000000000000000000000000000000000000001");
    full_case["full_scope_git_identities"][0]["after_blob"] = forged_blob.clone();
    full_case["full_scope_report_identities"][0]["after_blob"] = forged_blob;
    std::fs::write(
        &evaluation,
        serde_json::to_vec(&forged_full_identity).unwrap(),
    )
    .unwrap();
    assert_failure(
        run_verifier(&verifier, &evaluation),
        "recorded Full identities differ from the manifest history",
    );

    let mut forged_resume_source = original.clone();
    case_mut(&mut forged_resume_source, "noninteracting-author-followup")["delta"]["entries"][0]
        ["before_sha256"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    std::fs::write(
        &evaluation,
        serde_json::to_vec(&forged_resume_source).unwrap(),
    )
    .unwrap();
    assert_failure(
        run_verifier(&verifier, &evaluation),
        "recorded delta entries differ",
    );

    let mut forged_gate = original;
    case_mut(&mut forged_gate, "pure-rebase-carries-reviewed-edit")["gate_exit_code"] = json!(1);
    std::fs::write(&evaluation, serde_json::to_vec(&forged_gate).unwrap()).unwrap();
    assert_failure(
        run_verifier(&verifier, &evaluation),
        "recorded gate exit differs",
    );
}

#[test]
fn controlled_history_materialization_is_deterministic() {
    let output = Command::new("python3")
        .arg("-B")
        .arg(repository_root().join("tools/review-delta-v1/review_delta_v1.py"))
        .arg("self-test")
        .output()
        .unwrap();
    assert_success(output);
}
