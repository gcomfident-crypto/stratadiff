use std::{path::PathBuf, process::Command};

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

#[test]
fn review_continuity_comparison_passes_end_to_end() {
    let root = repository_root();
    let temporary = tempfile::tempdir().unwrap();
    let evaluation = temporary.path().join("evaluation.json");
    let runner = root.join("tools/review-continuity-v1/run.py");
    let verifier = root.join("tools/review-continuity-v1/verify.py");

    assert_success(
        Command::new("python3")
            .arg("-B")
            .arg(&runner)
            .arg("run")
            .arg("--stratadiff")
            .arg(env!("CARGO_BIN_EXE_stratadiff"))
            .arg("--output")
            .arg(&evaluation)
            .output()
            .unwrap(),
    );
    assert_success(
        Command::new("python3")
            .arg("-B")
            .arg(&verifier)
            .arg("verify")
            .arg("--evaluation")
            .arg(&evaluation)
            .output()
            .unwrap(),
    );
}

#[test]
fn review_continuity_freeze_and_tamper_contracts_hold() {
    let root = repository_root();
    let runner = root.join("tools/review-continuity-v1/run.py");
    let verifier = root.join("tools/review-continuity-v1/verify.py");

    for (program, command) in [
        (&runner, "self-test"),
        (&verifier, "verify-bundle"),
        (&verifier, "self-test"),
    ] {
        assert_success(
            Command::new("python3")
                .arg("-B")
                .arg(program)
                .arg(command)
                .output()
                .unwrap(),
        );
    }
}
