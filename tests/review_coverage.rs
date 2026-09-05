use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use stratadiff::{
    coverage::{
        FileCoverageState, build_review_coverage_passport, verify_review_coverage_passport,
    },
    ledger::{
        GITHUB_REVIEW_LEDGER_SCHEMA, GithubReviewLedger, GithubWebhookIngest, ingest_github_webhook,
    },
    ownership::{
        GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, GithubMembershipState, GithubOwnershipSnapshot,
        GithubOwnershipTeam, GithubOwnershipUser, GithubTeamMembership, GithubTeamPrivacy,
        RepositoryPermission,
    },
    review::REVIEW_DELTA_SCHEMA,
};

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_SECRET: &[u8] = b"coverage-webhook-secret";
const RECEIVER_KEY_ID: &str = "coverage-test-key";
const RECEIVER_SIGNING_KEY: [u8; 32] = [7; 32];
const RECEIVER_SIGNING_KEY_HEX: &str =
    "0707070707070707070707070707070707070707070707070707070707070707";

fn coverage_schema_errors(instance: &serde_json::Value) -> Vec<String> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-coverage-v1.schema.json")).unwrap();
    let ledger_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schema/github-review-ledger-v1.schema.json"
    ))
    .unwrap();
    let ownership_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schema/github-ownership-snapshot-v1.schema.json"
    ))
    .unwrap();
    let review_delta_schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-delta-v1.schema.json")).unwrap();
    let registry = jsonschema::Registry::new()
        .add(GITHUB_REVIEW_LEDGER_SCHEMA, ledger_schema)
        .unwrap()
        .add(GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, ownership_schema)
        .unwrap()
        .add(REVIEW_DELTA_SCHEMA, review_delta_schema)
        .unwrap()
        .prepare()
        .unwrap();
    let validator = jsonschema::draft202012::options()
        .with_registry(&registry)
        .offline()
        .build(&schema)
        .unwrap();
    validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect()
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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

struct History {
    repository: tempfile::TempDir,
    base: String,
    reviewed: String,
    head: String,
}

fn history() -> History {
    history_with_codeowners("/payments/ @acme/payments\n/security/ @acme/security\n")
}

fn history_with_codeowners(codeowners: &str) -> History {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "--quiet"]);
    git(
        repository.path(),
        &["config", "user.name", "StrataDiff Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "stratadiff@example.com"],
    );
    write(repository.path(), ".github/CODEOWNERS", codeowners);
    write(repository.path(), "payments/charge.txt", "charge=0\n");
    write(repository.path(), "security/policy.txt", "policy=0\n");
    let base = commit(repository.path(), "base");

    write(repository.path(), "payments/charge.txt", "charge=1\n");
    write(repository.path(), "security/policy.txt", "policy=1\n");
    let reviewed = commit(repository.path(), "reviewed");

    write(repository.path(), "payments/charge.txt", "charge=2\n");
    let head = commit(repository.path(), "payments follow-up");
    History {
        repository,
        base,
        reviewed,
        head,
    }
}

fn ownership(base: &str) -> GithubOwnershipSnapshot {
    GithubOwnershipSnapshot {
        schema: GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA.to_owned(),
        provider_url: "https://github.com".to_owned(),
        repository_id: 99,
        base_commit: base.to_owned(),
        api_version: "2022-11-28".to_owned(),
        observed_at: "2026-09-05T12:00:00Z".to_owned(),
        users: vec![
            GithubOwnershipUser {
                id: 11,
                login: "alice".to_owned(),
                repository_permission: RepositoryPermission::Write,
            },
            GithubOwnershipUser {
                id: 12,
                login: "bob".to_owned(),
                repository_permission: RepositoryPermission::Write,
            },
        ],
        teams: vec![
            GithubOwnershipTeam {
                id: 21,
                organization_login: "acme".to_owned(),
                slug: "payments".to_owned(),
                privacy: GithubTeamPrivacy::Closed,
                repository_permission: RepositoryPermission::Write,
                members: vec![GithubTeamMembership {
                    user_id: 11,
                    state: GithubMembershipState::Active,
                    inherited: false,
                }],
            },
            GithubOwnershipTeam {
                id: 22,
                organization_login: "acme".to_owned(),
                slug: "security".to_owned(),
                privacy: GithubTeamPrivacy::Closed,
                repository_permission: RepositoryPermission::Write,
                members: vec![GithubTeamMembership {
                    user_id: 12,
                    state: GithubMembershipState::Active,
                    inherited: false,
                }],
            },
        ],
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

fn add_review(
    ledger: Option<GithubReviewLedger>,
    review_id: u64,
    reviewer_id: u64,
    reviewer: &str,
    checkpoint: &str,
    base: &str,
    head_at_event: &str,
) -> GithubReviewLedger {
    let payload = serde_json::to_vec(&json!({
        "action": "submitted",
        "review": {
            "id": review_id,
            "node_id": format!("PRR_{review_id}"),
            "user": {
                "id": reviewer_id,
                "node_id": format!("U_{reviewer_id}"),
                "login": reviewer,
                "type": "User"
            },
            "state": "approved",
            "commit_id": checkpoint,
            "submitted_at": format!("2026-09-05T12:00:{review_id:02}Z"),
            "html_url": format!("https://github.com/acme/widgets/pull/7#pullrequestreview-{review_id}"),
            "author_association": "MEMBER"
        },
        "pull_request": {
            "id": 700,
            "node_id": "PR_700",
            "number": 7,
            "base": {"sha": base},
            "head": {"sha": head_at_event}
        },
        "repository": {"id": 99, "node_id": "R_99", "full_name": "acme/widgets"}
    }))
    .unwrap();
    ingest_github_webhook(
        ledger,
        GithubWebhookIngest {
            provider_url: "https://github.com",
            event_name: "pull_request_review",
            delivery_id: &format!("delivery-{review_id}"),
            received_at: &format!("2026-09-05T12:01:{review_id:02}Z"),
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

#[test]
fn two_owner_domains_invalidate_only_the_domain_changed_after_review() {
    let history = history();
    let ledger = add_review(
        None,
        1,
        11,
        "alice",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let ledger = add_review(
        Some(ledger),
        2,
        12,
        "bob",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger.clone(),
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();

    let instance = serde_json::to_value(&passport).unwrap();
    let schema_errors = coverage_schema_errors(&instance);
    assert!(
        schema_errors.is_empty(),
        "schema errors: {schema_errors:#?}"
    );

    assert!(!passport.body.summary.gate_passed);
    assert_eq!(passport.body.summary.current_files, 2);
    assert_eq!(passport.body.summary.retired_residue_files, 0);
    assert_eq!(passport.body.summary.total_requirements, 2);
    assert_eq!(passport.body.summary.covered_files, 1);
    assert_eq!(passport.body.summary.needs_review_files, 1);
    assert_eq!(passport.body.summary.blocked_files, 0);
    assert_eq!(passport.body.files.len(), 2);
    assert!(passport.body.non_claims.iter().any(|claim| {
        claim.contains("caller-provided exact object IDs")
            && claim.contains("does not authenticate their provider freshness")
    }));
    assert!(passport.body.non_claims.iter().any(|claim| {
        claim.contains("entire older valid ledger can be replayed")
            && claim.contains("trusted external latest revision or root")
    }));
    assert_eq!(
        passport
            .body
            .files
            .iter()
            .filter(|file| file.path == "payments/charge.txt")
            .count(),
        1
    );
    let payments = passport
        .body
        .files
        .iter()
        .find(|file| file.path == "payments/charge.txt")
        .unwrap();
    assert_eq!(payments.state, FileCoverageState::NeedsReview);
    assert!(
        payments.owner_alternatives[0]
            .covering_review_ids
            .is_empty()
    );
    let security = passport
        .body
        .files
        .iter()
        .find(|file| file.path == "security/policy.txt")
        .unwrap();
    assert_eq!(security.state, FileCoverageState::Covered);
    assert_eq!(security.owner_alternatives[0].covering_review_ids, [2]);

    let artifacts = tempfile::tempdir().unwrap();
    let ledger_path = artifacts.path().join("ledger.json");
    let ownership_path = artifacts.path().join("ownership.json");
    let passport_path = artifacts.path().join("passport.json");
    std::fs::write(&ledger_path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    std::fs::write(
        &ownership_path,
        serde_json::to_vec(&ownership(&history.base)).unwrap(),
    )
    .unwrap();
    let cli = Command::new(binary())
        .arg("review-coverage")
        .arg(&history.base)
        .arg(&history.head)
        .arg("--repo")
        .arg(history.repository.path())
        .arg("--ledger")
        .arg(&ledger_path)
        .arg("--ownership")
        .arg(&ownership_path)
        .arg("--output")
        .arg(&passport_path)
        .env("STRATADIFF_RECEIPT_SIGNING_KEY", RECEIVER_SIGNING_KEY_HEX)
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let cli_passport: stratadiff::coverage::ReviewCoveragePassport =
        serde_json::from_slice(&std::fs::read(&passport_path).unwrap()).unwrap();
    assert_eq!(cli_passport, passport);
    let verify = Command::new(binary())
        .arg("review-coverage-verify")
        .arg(&passport_path)
        .arg("--repo")
        .arg(history.repository.path())
        .arg("--trusted-receiver-public-key")
        .arg(&passport.body.ledger.receiver.public_key)
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );

    verify_review_coverage_passport(
        history.repository.path(),
        &passport,
        &passport.body.ledger.receiver.public_key,
    )
    .unwrap();

    let ledger = add_review(
        Some(ledger),
        3,
        11,
        "alice",
        &history.head,
        &history.base,
        &history.head,
    );
    let complete = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    assert!(complete.body.summary.gate_passed);
    assert_eq!(complete.body.summary.covered_files, 2);
}

#[test]
fn coverage_schema_fully_validates_embedded_versioned_artifacts_offline() {
    let history = history();
    let ledger = add_review(
        None,
        1,
        11,
        "alice",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let instance = serde_json::to_value(passport).unwrap();
    assert!(coverage_schema_errors(&instance).is_empty());

    for pointer in [
        "/body/ledger",
        "/body/ownership",
        "/body/checkpoint_proofs/0/result/review_delta",
    ] {
        let mut malformed = instance.clone();
        malformed
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), true.into());
        assert!(
            !coverage_schema_errors(&malformed).is_empty(),
            "coverage schema accepted malformed nested object at {pointer}"
        );
    }
}

#[test]
fn one_covering_owner_satisfies_an_or_rule_when_another_owner_is_unresolved() {
    let history = history_with_codeowners(
        "/payments/ @acme/payments\n/security/ @acme/security @acme/missing\n",
    );
    let ledger = add_review(
        None,
        2,
        12,
        "bob",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();

    let security = passport
        .body
        .files
        .iter()
        .find(|file| file.path == "security/policy.txt")
        .unwrap();
    assert_eq!(security.state, FileCoverageState::Covered);
    assert_eq!(security.owner_alternatives.len(), 2);
    assert_eq!(security.owner_alternatives[0].covering_review_ids, [2]);
    assert!(!security.owner_alternatives[1].blockers.is_empty());
    assert_eq!(passport.body.summary.blocked_files, 0);
}

#[test]
fn unavailable_review_commit_is_a_blocked_cell_not_a_substituted_checkpoint() {
    let history = history();
    let ledger = add_review(
        None,
        1,
        11,
        "alice",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        &history.base,
        &history.head,
    );
    let passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let payments = passport
        .body
        .files
        .iter()
        .find(|file| file.path == "payments/charge.txt")
        .unwrap();
    assert_eq!(payments.state, FileCoverageState::Blocked);
    assert!(payments.owner_alternatives[0].blockers[0].contains("checkpoint proof is unavailable"));
    assert!(!passport.body.summary.gate_passed);
}

#[test]
fn passport_attestation_detects_tampering_before_offline_recomputation() {
    let history = history();
    let ledger = add_review(
        None,
        1,
        11,
        "alice",
        &history.head,
        &history.base,
        &history.head,
    );
    let mut passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let public_key = passport.body.ledger.receiver.public_key.clone();
    passport.body.files[0].reason.push_str(" tampered");

    let error = verify_review_coverage_passport(history.repository.path(), &passport, &public_key)
        .unwrap_err();
    assert!(error.to_string().contains("body digest mismatch"));
}

#[test]
fn dropped_reviewed_change_remains_owner_residue_until_a_new_review() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "--quiet"]);
    git(
        repository.path(),
        &["config", "user.name", "StrataDiff Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "stratadiff@example.com"],
    );
    write(
        repository.path(),
        ".github/CODEOWNERS",
        "/payments/ @acme/payments\n/security/ @acme/security\n",
    );
    write(repository.path(), "payments/drop.txt", "value=0\n");
    let base = commit(repository.path(), "base");
    write(repository.path(), "payments/drop.txt", "value=1\n");
    let reviewed = commit(repository.path(), "reviewed change");

    let ledger = add_review(None, 1, 11, "alice", &reviewed, &base, &reviewed);
    let passport = build_review_coverage_passport(
        repository.path(),
        &base,
        &base,
        ledger.clone(),
        ownership(&base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    assert_eq!(passport.body.summary.current_files, 0);
    assert_eq!(passport.body.summary.retired_residue_files, 1);
    assert_eq!(passport.body.summary.needs_review_files, 1);
    assert!(!passport.body.summary.gate_passed);
    assert_eq!(
        passport.body.files[0].scope,
        stratadiff::coverage::CoverageFileScope::RetiredResidue
    );
    assert_eq!(passport.body.files[0].path, "payments/drop.txt");

    let ledger = add_review(Some(ledger), 2, 11, "alice", &base, &base, &base);
    let complete = build_review_coverage_passport(
        repository.path(),
        &base,
        &base,
        ledger,
        ownership(&base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    assert!(complete.body.files.is_empty());
    assert!(complete.body.summary.gate_passed);
}

#[test]
fn verified_passport_view_serves_the_coverage_session_and_not_source_bytes() {
    let history = history();
    let ledger = add_review(
        None,
        1,
        11,
        "alice",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let ledger = add_review(
        Some(ledger),
        2,
        12,
        "bob",
        &history.reviewed,
        &history.base,
        &history.reviewed,
    );
    let passport = build_review_coverage_passport(
        history.repository.path(),
        &history.base,
        &history.head,
        ledger,
        ownership(&history.base),
        &RECEIVER_SIGNING_KEY,
    )
    .unwrap();
    let passport_bytes = serde_json::to_vec(&passport).unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let passport_path = artifacts.path().join("passport.json");
    std::fs::write(&passport_path, &passport_bytes).unwrap();

    let child = Command::new(binary())
        .arg("review-coverage-view")
        .arg(&passport_path)
        .arg("--repo")
        .arg(history.repository.path())
        .arg("--trusted-receiver-public-key")
        .arg(&passport.body.ledger.receiver.public_key)
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut first_line = String::new();
        let result = stderr.read_line(&mut first_line).map(|_| first_line);
        let _ = sender.send(result);
    });
    let first_line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("coverage workbench did not print its URL within 30 seconds")
        .unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Review Coverage Passport: http://")
        .unwrap();
    let (address, token) = url.split_once("/?token=").unwrap();

    let session = get_http(address, &format!("/api/session?token={token}"));
    let (_, session_body) = split_http_response(&session);
    let session: serde_json::Value = serde_json::from_slice(session_body).unwrap();
    assert_eq!(session["kind"], "review_coverage_passport");
    assert_eq!(session["verification"]["verified"], true);
    assert_eq!(session["passport"]["body"]["summary"]["gate_passed"], false);

    let download = get_http(address, &format!("/api/passport?token={token}"));
    let (download_headers, download_body) = split_http_response(&download);
    assert!(
        download_headers
            .windows(b"content-disposition: attachment".len())
            .any(|window| window == b"content-disposition: attachment")
    );
    assert_eq!(download_body, passport_bytes);

    let source = get_http(address, &format!("/api/source/after?token={token}"));
    assert!(source.starts_with(b"HTTP/1.1 404 Not Found\r\n"));
    let denied = get_http(address, "/api/passport?token=invalid");
    assert!(denied.starts_with(b"HTTP/1.1 404 Not Found\r\n"));
}

fn get_http(address: &str, path: &str) -> Vec<u8> {
    let address: SocketAddr = address.parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn split_http_response(response: &[u8]) -> (&[u8], &[u8]) {
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    (&response[..boundary], &response[boundary + 4..])
}
