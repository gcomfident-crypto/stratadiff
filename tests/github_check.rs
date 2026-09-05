use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use stratadiff::{
    coverage::{ReviewCoveragePassport, build_review_coverage_passport},
    github_check::{
        GithubCheckRunConclusion, GithubCheckRunStatus, MAX_GITHUB_CHECK_RUN_ANNOTATIONS,
        build_github_check_run_payload,
    },
    ledger::{GithubReviewLedger, GithubWebhookIngest, ingest_github_webhook},
    ownership::{
        GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, GithubMembershipState, GithubOwnershipSnapshot,
        GithubOwnershipTeam, GithubOwnershipUser, GithubTeamMembership, GithubTeamPrivacy,
        RepositoryPermission,
    },
};

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_SECRET: &[u8] = b"check-run-webhook-secret";
const RECEIVER_KEY_ID: &str = "check-run-test-key";
const RECEIVER_SIGNING_KEY: [u8; 32] = [7; 32];

struct Fixture {
    repository: tempfile::TempDir,
    base: String,
    head: String,
    passport: ReviewCoveragePassport,
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_stratadiff"))
}

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(repository: &Path, message: &str) -> String {
    git(repository, &["add", "-A"]);
    git(repository, &["commit", "--quiet", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

fn write(repository: &Path, relative: &str, contents: &str) {
    let path = repository.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn initialize(repository: &Path) {
    git(repository, &["init", "--quiet"]);
    git(repository, &["config", "user.name", "StrataDiff Test"]);
    git(
        repository,
        &["config", "user.email", "stratadiff@example.com"],
    );
    write(repository, ".github/CODEOWNERS", "/src/ @acme/reviewers\n");
}

fn ownership(base: &str) -> GithubOwnershipSnapshot {
    GithubOwnershipSnapshot {
        schema: GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA.to_owned(),
        provider_url: "https://github.com".to_owned(),
        repository_id: 99,
        base_commit: base.to_owned(),
        api_version: "2022-11-28".to_owned(),
        observed_at: "2026-09-05T12:00:00Z".to_owned(),
        users: vec![GithubOwnershipUser {
            id: 11,
            login: "alice".to_owned(),
            repository_permission: RepositoryPermission::Write,
        }],
        teams: vec![GithubOwnershipTeam {
            id: 21,
            organization_login: "acme".to_owned(),
            slug: "reviewers".to_owned(),
            privacy: GithubTeamPrivacy::Closed,
            repository_permission: RepositoryPermission::Write,
            members: vec![GithubTeamMembership {
                user_id: 11,
                state: GithubMembershipState::Active,
                inherited: false,
            }],
        }],
    }
}

fn sign(payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET).unwrap();
    mac.update(payload);
    let mut encoded = String::with_capacity(64);
    for byte in mac.finalize().into_bytes() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").unwrap();
    }
    format!("sha256={encoded}")
}

fn review_ledger(checkpoint: &str, base: &str, head: &str) -> GithubReviewLedger {
    let payload = serde_json::to_vec(&json!({
        "action": "submitted",
        "review": {
            "id": 1,
            "node_id": "PRR_1",
            "user": {"id": 11, "node_id": "U_11", "login": "alice", "type": "User"},
            "state": "approved",
            "commit_id": checkpoint,
            "submitted_at": "2026-09-05T12:00:01Z",
            "html_url": "https://github.com/acme/widgets/pull/7#pullrequestreview-1",
            "author_association": "MEMBER"
        },
        "pull_request": {
            "id": 700,
            "node_id": "PR_700",
            "number": 7,
            "base": {"sha": base},
            "head": {"sha": head}
        },
        "repository": {"id": 99, "node_id": "R_99", "full_name": "acme/widgets"}
    }))
    .unwrap();
    ingest_github_webhook(
        None,
        GithubWebhookIngest {
            provider_url: "https://github.com",
            event_name: "pull_request_review",
            delivery_id: "delivery-1",
            received_at: "2026-09-05T12:01:01Z",
            signature_header: &sign(&payload),
            secret: WEBHOOK_SECRET,
            receiver_key_id: RECEIVER_KEY_ID,
            receiver_signing_key: &RECEIVER_SIGNING_KEY,
            payload: &payload,
        },
    )
    .unwrap()
    .0
}

fn current_changes_fixture(file_count: usize, reviewed_at_head: bool) -> Fixture {
    let repository = tempfile::tempdir().unwrap();
    initialize(repository.path());
    let base = commit(repository.path(), "base");
    for index in 0..file_count {
        write(
            repository.path(),
            &format!("src/file-{index:03}.txt"),
            &format!("change {index}\n"),
        );
    }
    let head = commit(repository.path(), "head");
    let checkpoint = if reviewed_at_head { &head } else { &base };
    let passport = build_review_coverage_passport(
        repository.path(),
        &base,
        &head,
        review_ledger(checkpoint, &base, checkpoint),
        ownership(&base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    Fixture {
        repository,
        base,
        head,
        passport,
    }
}

#[test]
fn payload_is_deterministic_head_bound_and_caps_owner_annotations_at_fifty() {
    let fixture = current_changes_fixture(MAX_GITHUB_CHECK_RUN_ANNOTATIONS + 10, false);
    let trusted_key = &fixture.passport.body.ledger.receiver.public_key;
    let first = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        trusted_key,
        &fixture.base,
        &fixture.head,
        Some("https://github.com/acme/widgets/pull/7/checks"),
    )
    .unwrap();
    let second = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        trusted_key,
        &fixture.base,
        &fixture.head,
        Some("https://github.com/acme/widgets/pull/7/checks"),
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.head_sha, fixture.head);
    assert_eq!(first.status, GithubCheckRunStatus::Completed);
    assert_eq!(first.conclusion, GithubCheckRunConclusion::Failure);
    assert_eq!(
        first.output.annotations.len(),
        MAX_GITHUB_CHECK_RUN_ANNOTATIONS
    );
    assert!(
        first
            .output
            .annotations
            .iter()
            .all(|annotation| annotation.message.contains("@acme/reviewers"))
    );
    assert!(first.output.summary.contains("| Needs review | 60 |"));
    assert!(first.output.summary.contains("50 emitted, 10 omitted"));
}

#[test]
fn valid_complete_passport_maps_to_a_successful_check() {
    let fixture = current_changes_fixture(2, true);
    let payload = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        &fixture.passport.body.ledger.receiver.public_key,
        &fixture.base,
        &fixture.head,
        None,
    )
    .unwrap();

    assert_eq!(payload.conclusion, GithubCheckRunConclusion::Success);
    assert!(payload.output.annotations.is_empty());
    assert!(payload.details_url.is_none());
}

#[test]
fn tampering_is_rejected_before_a_check_payload_can_be_built() {
    let fixture = current_changes_fixture(1, false);
    let mut tampered = fixture.passport.clone();
    tampered.body.summary.gate_passed = true;
    let error = build_github_check_run_payload(
        fixture.repository.path(),
        &tampered,
        &fixture.passport.body.ledger.receiver.public_key,
        &fixture.base,
        &fixture.head,
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains("body digest mismatch"));
}

#[test]
fn stale_live_base_and_head_are_rejected_after_verification() {
    let fixture = current_changes_fixture(1, false);
    let trusted_key = &fixture.passport.body.ledger.receiver.public_key;
    let stale_base = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        trusted_key,
        "1111111111111111111111111111111111111111",
        &fixture.head,
        None,
    )
    .unwrap_err();
    assert!(
        stale_base
            .to_string()
            .contains("live pull request base changed")
    );

    let stale_head = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        trusted_key,
        &fixture.base,
        "2222222222222222222222222222222222222222",
        None,
    )
    .unwrap_err();
    assert!(
        stale_head
            .to_string()
            .contains("live pull request head changed")
    );
}

#[test]
fn details_url_requires_safe_https_without_credentials() {
    let fixture = current_changes_fixture(1, false);
    let trusted_key = &fixture.passport.body.ledger.receiver.public_key;
    for invalid in [
        "http://github.com/acme/widgets",
        "https://token@github.com/acme/widgets",
        "https://github.com/acme/widgets\nspoofed",
    ] {
        let error = build_github_check_run_payload(
            fixture.repository.path(),
            &fixture.passport,
            trusted_key,
            &fixture.base,
            &fixture.head,
            Some(invalid),
        )
        .unwrap_err();
        assert!(error.to_string().contains("details URL"));
    }
}

#[test]
fn retired_residue_remains_check_level_and_never_becomes_a_line_annotation() {
    let repository = tempfile::tempdir().unwrap();
    initialize(repository.path());
    write(repository.path(), "src/dropped.txt", "before\n");
    let base = commit(repository.path(), "base");
    write(repository.path(), "src/dropped.txt", "reviewed\n");
    let reviewed = commit(repository.path(), "reviewed");
    let passport = build_review_coverage_passport(
        repository.path(),
        &base,
        &base,
        review_ledger(&reviewed, &base, &reviewed),
        ownership(&base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let payload = build_github_check_run_payload(
        repository.path(),
        &passport,
        &passport.body.ledger.receiver.public_key,
        &base,
        &base,
        None,
    )
    .unwrap();

    assert_eq!(payload.conclusion, GithubCheckRunConclusion::Failure);
    assert!(payload.output.annotations.is_empty());
    assert!(payload.output.summary.contains("| Retired residue | 1 |"));
    assert!(
        payload
            .output
            .summary
            .contains("1 reported at check level only")
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_current_path_remains_check_level_only() {
    let repository = tempfile::tempdir().unwrap();
    initialize(repository.path());
    let base = commit(repository.path(), "base");
    let source = repository
        .path()
        .join("src")
        .join(OsString::from_vec(vec![0xff, b'.', b't', b'x', b't']));
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(source, b"change\n").unwrap();
    let head = commit(repository.path(), "non-UTF-8 path");
    let passport = build_review_coverage_passport(
        repository.path(),
        &base,
        &head,
        review_ledger(&base, &base, &base),
        ownership(&base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let payload = build_github_check_run_payload(
        repository.path(),
        &passport,
        &passport.body.ledger.receiver.public_key,
        &base,
        &head,
        None,
    )
    .unwrap();

    assert!(payload.output.annotations.is_empty());
    assert!(
        payload
            .output
            .summary
            .contains("1 reported at check level only")
    );
}

#[test]
fn cli_writes_the_verified_create_check_run_json_without_publishing() {
    let fixture = current_changes_fixture(2, false);
    let artifacts = tempfile::tempdir().unwrap();
    let passport_path = artifacts.path().join("passport.json");
    let output_path = artifacts.path().join("check-run.json");
    std::fs::write(
        &passport_path,
        serde_json::to_vec(&fixture.passport).unwrap(),
    )
    .unwrap();
    let output = Command::new(binary())
        .arg("github-check-run")
        .arg(&passport_path)
        .arg("--repo")
        .arg(fixture.repository.path())
        .arg("--trusted-receiver-public-key")
        .arg(&fixture.passport.body.ledger.receiver.public_key)
        .arg("--expected-base")
        .arg(&fixture.base)
        .arg("--expected-head")
        .arg(&fixture.head)
        .arg("--details-url")
        .arg("https://github.com/acme/widgets/pull/7/checks")
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("(not published)"));
    let actual: Value = serde_json::from_slice(&std::fs::read(&output_path).unwrap()).unwrap();
    let expected = build_github_check_run_payload(
        fixture.repository.path(),
        &fixture.passport,
        &fixture.passport.body.ledger.receiver.public_key,
        &fixture.base,
        &fixture.head,
        Some("https://github.com/acme/widgets/pull/7/checks"),
    )
    .unwrap();
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
}

#[test]
fn cli_help_states_github_app_write_constraint() {
    let output = Command::new(binary())
        .arg("github-check-run")
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("GitHub grants Check Run write access only to GitHub Apps"));
    assert!(help.contains("ordinary personal access token cannot publish"));
}
