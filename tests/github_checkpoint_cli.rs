use std::{fs, process::Command};

use stratadiff::github::MAX_GITHUB_REVIEWS;

fn review_json(id: usize, login: &str, state: &str, commit_id: &str, submitted_at: &str) -> String {
    format!(
        r#"{{"id":{id},"user":{{"login":"{login}","type":"User"}},"state":"{state}","html_url":"https://github.com/example/project/pull/7#pullrequestreview-{id}","commit_id":"{commit_id}","submitted_at":"{submitted_at}","author_association":"MEMBER"}}"#
    )
}

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
fn github_checkpoint_selects_across_one_hundred_and_one_slurped_reviews() {
    let directory = tempfile::tempdir().unwrap();
    let reviews = directory.path().join("review-pages.json");
    let first_commit = "a".repeat(40);
    let second_commit = "b".repeat(40);
    let first_page = (1..=100)
        .map(|id| {
            if id == 1 {
                review_json(
                    id,
                    "alice",
                    "APPROVED",
                    &first_commit,
                    "2026-09-04T17:10:09Z",
                )
            } else {
                review_json(
                    id,
                    "other",
                    "COMMENTED",
                    &first_commit,
                    "2026-09-04T18:10:09Z",
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let second_page = review_json(
        101,
        "alice",
        "CHANGES_REQUESTED",
        &second_commit,
        "2026-09-04T19:10:09Z",
    );
    fs::write(&reviews, format!("[[{first_page}],[{second_page}]]")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(&reviews)
        .arg("--reviewer")
        .arg("alice")
        .arg("--gh-slurp-pages")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["observed_reviews"], 101);
    assert_eq!(value["eligible_reviews"], 2);
    assert_eq!(value["checkpoint"]["review_id"], 101);
    assert_eq!(value["checkpoint"]["commit_id"], "b".repeat(40));
}

#[test]
fn github_checkpoint_rejects_slurped_reviews_over_the_global_count_limit() {
    let directory = tempfile::tempdir().unwrap();
    let reviews = directory.path().join("review-pages.json");
    let commit_id = "a".repeat(40);
    let records = (1..=MAX_GITHUB_REVIEWS + 1)
        .map(|id| review_json(id, "other", "COMMENTED", &commit_id, "2026-09-04T18:10:09Z"))
        .collect::<Vec<_>>();
    let pages = records
        .chunks(100)
        .map(|page| format!("[{}]", page.join(",")))
        .collect::<Vec<_>>()
        .join(",");
    fs::write(&reviews, format!("[{pages}]")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(&reviews)
        .arg("--reviewer")
        .arg("alice")
        .arg("--gh-slurp-pages")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("GitHub review count limit exceeded: observed at least 10001, limit 10000")
    );
}

#[test]
fn github_checkpoint_decodes_one_bounded_gh_included_response() {
    let directory = reviews_fixture();
    let body = fs::read(directory.path().join("reviews.json")).unwrap();
    let response = directory.path().join("reviews-response.txt");
    let mut included =
        b"HTTP/2.0 200 OK\nContent-Type: application/json; charset=utf-8\r\nEtag: \"snapshot\"\r\n\r\n"
            .to_vec();
    included.extend_from_slice(&body);
    fs::write(&response, included).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(&response)
        .arg("--reviewer")
        .arg("alice")
        .arg("--gh-included-response")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, format!("{}\n", "b".repeat(40)).as_bytes());
}

#[test]
fn github_checkpoint_included_response_rejects_pagination_before_selection() {
    let directory = tempfile::tempdir().unwrap();
    let response = directory.path().join("reviews-response.txt");
    fs::write(
        &response,
        b"HTTP/2.0 200 OK\nContent-Type: application/json\r\nLink: <https://api.github.example/reviews?page=2>; rel=\"next\"\r\n\r\n[]",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("github-checkpoint")
        .arg(&response)
        .arg("--reviewer")
        .arg("alice")
        .arg("--gh-included-response")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("GitHub review pagination is incomplete")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("--paginate --slurp"));
    assert!(output.stdout.is_empty());
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
