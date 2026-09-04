use std::{fs, process::Command};

fn reviews_fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("reviews.json"),
        br#"[
          {
            "id": 101,
            "user": {"login": "alice", "type": "User"},
            "state": "APPROVED",
            "html_url": "https://github.com/example/project/pull/7#pullrequestreview-101",
            "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "submitted_at": "2026-09-04T17:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 102,
            "user": {"login": "alice", "type": "User"},
            "state": "CHANGES_REQUESTED",
            "html_url": "https://github.com/example/project/pull/7#pullrequestreview-102",
            "commit_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "submitted_at": "2026-09-04T19:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 103,
            "user": {"login": "alice", "type": "User"},
            "state": "COMMENTED",
            "html_url": "https://github.com/example/project/pull/7#pullrequestreview-103",
            "commit_id": "cccccccccccccccccccccccccccccccccccccccc",
            "submitted_at": "2026-09-04T20:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 104,
            "user": {"login": "alice", "type": "User"},
            "state": "DISMISSED",
            "html_url": "https://github.com/example/project/pull/7#pullrequestreview-104",
            "commit_id": "dddddddddddddddddddddddddddddddddddddddd",
            "submitted_at": "2026-09-04T21:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 105,
            "user": {"login": "alice", "type": "Bot"},
            "state": "APPROVED",
            "html_url": "https://github.com/example/project/pull/7#pullrequestreview-105",
            "commit_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "submitted_at": "2026-09-04T22:10:09Z",
            "author_association": "CONTRIBUTOR"
          }
        ]"#,
    )
    .unwrap();
    directory
}

#[test]
fn github_checkpoint_prints_the_latest_completed_human_review_commit() {
    let directory = reviews_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(directory.path().join("reviews.json"))
        .arg("--reviewer")
        .arg("ALICE")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, format!("{}\n", "b".repeat(40)).as_bytes());
    assert!(output.stderr.is_empty());
}

#[test]
fn github_checkpoint_json_exposes_the_selection_boundary() {
    let directory = reviews_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(directory.path().join("reviews.json"))
        .arg("--reviewer")
        .arg("alice")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["selection_policy"],
        "latest_nondismissed_human_approved_or_changes_requested_review_for_explicit_reviewer"
    );
    assert_eq!(value["observed_reviews"], 5);
    assert_eq!(value["matching_reviewer_reviews"], 5);
    assert_eq!(value["eligible_reviews"], 2);
    assert_eq!(value["checkpoint"]["review_id"], 102);
    assert_eq!(value["checkpoint"]["review_state"], "changes_requested");
}

#[test]
fn github_checkpoint_returns_no_sha_when_the_reviewer_has_no_completed_review() {
    let directory = reviews_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(directory.path().join("reviews.json"))
        .arg("--reviewer")
        .arg("bob")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn github_checkpoint_fails_closed_on_an_invalid_selected_commit() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("reviews.json"),
        br#"[{
          "id": 1,
          "user": {"login": "alice", "type": "User"},
          "state": "APPROVED",
          "html_url": "https://github.com/example/project/pull/7#pullrequestreview-1",
          "commit_id": "not-an-object-id",
          "submitted_at": "2026-09-04T17:10:09Z",
          "author_association": "MEMBER"
        }]"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(directory.path().join("reviews.json"))
        .arg("--reviewer")
        .arg("alice")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("GitHub review 1 has an invalid commit_id")
    );
}

#[test]
fn github_commit_object_requires_the_provider_response_to_match_the_review_sha() {
    let directory = tempfile::tempdir().unwrap();
    let expected = "a".repeat(40);
    let object = directory.path().join("commit.json");
    fs::write(
        &object,
        format!(
            r#"{{"sha":"{expected}","tree":{{"sha":"{}"}}}}"#,
            "b".repeat(40)
        ),
    )
    .unwrap();

    let accepted = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-commit-object")
        .arg(&object)
        .arg("--expected")
        .arg(&expected)
        .output()
        .unwrap();
    assert!(accepted.status.success());
    assert_eq!(accepted.stdout, format!("{expected}\n").as_bytes());
    assert!(accepted.stderr.is_empty());

    let rejected = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-commit-object")
        .arg(&object)
        .arg("--expected")
        .arg("c".repeat(40))
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("GitHub Git commit object resolved to")
    );
}
