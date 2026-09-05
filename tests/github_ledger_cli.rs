use std::{path::PathBuf, process::Command};

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_stratadiff"))
}

fn payload() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": "submitted",
        "review": {
            "id": 41,
            "node_id": "PRR_41",
            "user": {"id": 17, "node_id": "U_17", "login": "alice", "type": "User"},
            "state": "approved",
            "commit_id": "c123456789012345678901234567890123456789",
            "submitted_at": "2026-09-05T01:02:03Z",
            "html_url": "https://github.com/acme/widgets/pull/7#pullrequestreview-41",
            "author_association": "MEMBER"
        },
        "pull_request": {
            "id": 700,
            "node_id": "PR_700",
            "number": 7,
            "base": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "head": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        },
        "repository": {"id": 99, "node_id": "R_99", "full_name": "acme/widgets"}
    }))
    .unwrap()
}

fn signature(secret: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(payload);
    let mut encoded = String::with_capacity(64);
    for byte in mac.finalize().into_bytes() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").unwrap();
    }
    format!("sha256={encoded}")
}

#[test]
fn cli_ingests_and_deduplicates_a_verified_review_delivery() {
    let directory = tempfile::tempdir().unwrap();
    let payload_path = directory.path().join("payload.json");
    let ledger_path = directory.path().join("ledger.json");
    let secret = "integration-secret";
    let payload = payload();
    std::fs::write(&payload_path, &payload).unwrap();
    let signature = signature(secret.as_bytes(), &payload);

    let first = Command::new(binary())
        .arg("github-ledger-ingest")
        .arg(&payload_path)
        .arg("--event")
        .arg("pull_request_review")
        .arg("--delivery-id")
        .arg("delivery-1")
        .arg("--received-at")
        .arg("2026-09-05T02:00:00Z")
        .arg("--signature")
        .arg(&signature)
        .arg("--provider-url")
        .arg("https://github.com")
        .arg("--receiver-key-id")
        .arg("test-key-2026-09")
        .arg("--output")
        .arg(&ledger_path)
        .env("STRATADIFF_GITHUB_WEBHOOK_SECRET", secret)
        .env(
            "STRATADIFF_RECEIPT_SIGNING_KEY",
            "0707070707070707070707070707070707070707070707070707070707070707",
        )
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stderr).contains("applied GitHub delivery"));

    let ledger: Value = serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    let schema: Value = serde_json::from_str(include_str!(
        "../schema/github-review-ledger-v1.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let errors = validator
        .iter_errors(&ledger)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");
    assert_eq!(ledger["review_receipts"].as_array().unwrap().len(), 1);
    assert_eq!(ledger["review_receipts"][0]["reviewer_id"], 17);
    assert_eq!(
        ledger["review_receipts"][0]["commit_id"],
        "c123456789012345678901234567890123456789"
    );

    let duplicate = Command::new(binary())
        .arg("github-ledger-ingest")
        .arg(&payload_path)
        .arg("--ledger")
        .arg(&ledger_path)
        .arg("--event")
        .arg("pull_request_review")
        .arg("--delivery-id")
        .arg("delivery-1")
        .arg("--received-at")
        .arg("2026-09-05T02:00:00Z")
        .arg("--signature")
        .arg(&signature)
        .arg("--provider-url")
        .arg("https://github.com")
        .arg("--receiver-key-id")
        .arg("test-key-2026-09")
        .arg("--output")
        .arg(&ledger_path)
        .env("STRATADIFF_GITHUB_WEBHOOK_SECRET", secret)
        .env(
            "STRATADIFF_RECEIPT_SIGNING_KEY",
            "0707070707070707070707070707070707070707070707070707070707070707",
        )
        .output()
        .unwrap();
    assert!(duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate GitHub delivery"));
    let duplicate_ledger: Value =
        serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert_eq!(ledger, duplicate_ledger);
}

#[test]
fn cli_rejects_an_unverified_payload_without_writing_a_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let payload_path = directory.path().join("payload.json");
    let ledger_path = directory.path().join("ledger.json");
    std::fs::write(&payload_path, payload()).unwrap();

    let output = Command::new(binary())
        .arg("github-ledger-ingest")
        .arg(&payload_path)
        .arg("--event")
        .arg("pull_request_review")
        .arg("--delivery-id")
        .arg("delivery-1")
        .arg("--received-at")
        .arg("2026-09-05T02:00:00Z")
        .arg("--signature")
        .arg("sha256=0000000000000000000000000000000000000000000000000000000000000000")
        .arg("--provider-url")
        .arg("https://github.com")
        .arg("--receiver-key-id")
        .arg("test-key-2026-09")
        .arg("--output")
        .arg(&ledger_path)
        .env("STRATADIFF_GITHUB_WEBHOOK_SECRET", "wrong-secret")
        .env(
            "STRATADIFF_RECEIPT_SIGNING_KEY",
            "0707070707070707070707070707070707070707070707070707070707070707",
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("signature verification failed"));
    assert!(!ledger_path.exists());
}
