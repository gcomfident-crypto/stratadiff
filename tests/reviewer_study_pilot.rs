use std::{path::PathBuf, process::Command};

const PILOT: &str = "tools/reviewer-study-v1/pilot.py";

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_pilot(arguments: &[&str]) -> std::process::Output {
    Command::new("python3")
        .arg("-B")
        .arg(PILOT)
        .args(arguments)
        .current_dir(repository_root())
        .output()
        .unwrap()
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "pilot command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reviewer_study_pilot_self_test_passes() {
    let output = run_pilot(&["self-test"]);
    assert_success(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("reviewer-pilot self-test passed"),
        "self-test did not report an explicit success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reviewer_study_pilot_help_lists_the_complete_workflow() {
    let output = run_pilot(&["--help"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    for command in [
        "self-test",
        "plan",
        "enroll",
        "attrition",
        "session",
        "adjudicator",
        "adjudication",
        "follow-up",
        "lock",
        "attest-final",
        "verify",
    ] {
        assert!(
            stdout.contains(command),
            "pilot help omitted {command:?}\nstdout:\n{stdout}"
        );
    }
}
