use anyhow::Result;
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use stratadiff::ledger::{
    CompletedReviewState, GithubHeadObservation, GithubReviewDismissal, GithubReviewLedger,
    GithubReviewReceipt, GithubWebhookIngest, GithubWebhookKind, ingest_github_webhook,
};

type HmacSha256 = Hmac<Sha256>;

const SECRET: &[u8] = b"fact-binding-test-secret";
const RECEIVER_KEY_ID: &str = "fact-binding-test-key";
const RECEIVER_SIGNING_KEY: [u8; 32] = [19; 32];

fn review_payload(
    action: &str,
    state: &str,
    review_id: u64,
    reviewer_id: u64,
    commit_id: Option<&str>,
    submitted_at: Option<&str>,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": action,
        "review": {
            "id": review_id,
            "node_id": format!("PRR_{review_id}"),
            "user": {
                "id": reviewer_id,
                "node_id": format!("U_{reviewer_id}"),
                "login": format!("reviewer-{reviewer_id}"),
                "type": "User"
            },
            "state": state,
            "commit_id": commit_id,
            "submitted_at": submitted_at,
            "html_url": format!(
                "https://github.com/acme/widgets/pull/7#pullrequestreview-{review_id}"
            ),
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

fn synchronize_payload(before: &str, after: &str, base: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": "synchronize",
        "before": before,
        "after": after,
        "pull_request": {
            "id": 700,
            "node_id": "PR_700",
            "number": 7,
            "base": {"sha": base},
            "head": {"sha": after}
        },
        "repository": {"id": 99, "node_id": "R_99", "full_name": "acme/widgets"}
    }))
    .unwrap()
}

fn signature(payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(SECRET).unwrap();
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    format!("sha256={digest:x}")
}

fn ingest_review(
    ledger: Option<GithubReviewLedger>,
    delivery_id: &str,
    received_at: &str,
    payload: &[u8],
) -> GithubReviewLedger {
    ingest(
        ledger,
        "pull_request_review",
        delivery_id,
        received_at,
        payload,
    )
}

fn ingest_synchronize(
    ledger: Option<GithubReviewLedger>,
    delivery_id: &str,
    received_at: &str,
    payload: &[u8],
) -> GithubReviewLedger {
    ingest(ledger, "pull_request", delivery_id, received_at, payload)
}

fn ingest(
    ledger: Option<GithubReviewLedger>,
    event_name: &str,
    delivery_id: &str,
    received_at: &str,
    payload: &[u8],
) -> GithubReviewLedger {
    ingest_github_webhook(
        ledger,
        GithubWebhookIngest {
            provider_url: "https://github.com",
            event_name,
            delivery_id,
            received_at,
            signature_header: &signature(payload),
            secret: SECRET,
            receiver_key_id: RECEIVER_KEY_ID,
            receiver_signing_key: &RECEIVER_SIGNING_KEY,
            payload,
        },
    )
    .unwrap()
    .0
}

fn approved_payload(review_id: u64, reviewer_id: u64, commit: char) -> Vec<u8> {
    review_payload(
        "submitted",
        "approved",
        review_id,
        reviewer_id,
        Some(&commit.to_string().repeat(40)),
        Some("2026-09-05T01:02:03Z"),
    )
}

fn dismissed_payload(
    review_id: u64,
    reviewer_id: u64,
    commit_id: Option<&str>,
    submitted_at: Option<&str>,
) -> Vec<u8> {
    review_payload(
        "dismissed",
        "dismissed",
        review_id,
        reviewer_id,
        commit_id,
        submitted_at,
    )
}

fn assert_invalid(ledger: &GithubReviewLedger, attack: &str) {
    assert!(
        ledger.validate().is_err(),
        "ledger validation accepted {attack}"
    );
}

fn assert_receipt_tamper(
    original: &GithubReviewLedger,
    attack: &str,
    tamper: impl FnOnce(&mut GithubReviewReceipt),
) {
    let mut ledger = original.clone();
    tamper(&mut ledger.review_receipts[0]);
    ledger.review_receipts[0].receipt_sha256 = receipt_digest(&ledger.review_receipts[0]).unwrap();
    assert_invalid(&ledger, attack);
}

fn assert_dismissal_tamper(
    original: &GithubReviewLedger,
    attack: &str,
    tamper: impl FnOnce(&mut GithubReviewDismissal),
) {
    let mut ledger = original.clone();
    tamper(&mut ledger.dismissals[0]);
    assert_invalid(&ledger, attack);
}

fn assert_head_tamper(
    original: &GithubReviewLedger,
    attack: &str,
    tamper: impl FnOnce(&mut GithubHeadObservation),
) {
    let mut ledger = original.clone();
    tamper(&mut ledger.head_observations[0]);
    assert_invalid(&ledger, attack);
}

fn receipt_digest(receipt: &GithubReviewReceipt) -> Result<String> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        domain: &'static str,
        review_id: u64,
        review_node_id: &'a str,
        reviewer_id: u64,
        reviewer_node_id: &'a str,
        reviewer_login: &'a str,
        state: CompletedReviewState,
        commit_id: &'a str,
        submitted_at: &'a str,
        html_url: &'a str,
        author_association: &'a str,
        source_delivery_id: &'a str,
        payload_sha256: &'a str,
    }

    let encoded = serde_json::to_vec(&DigestInput {
        domain: "stratadiff-github-review-receipt-v1",
        review_id: receipt.review_id,
        review_node_id: &receipt.review_node_id,
        reviewer_id: receipt.reviewer_id,
        reviewer_node_id: &receipt.reviewer_node_id,
        reviewer_login: &receipt.reviewer_login,
        state: receipt.state,
        commit_id: &receipt.commit_id,
        submitted_at: &receipt.submitted_at,
        html_url: &receipt.html_url,
        author_association: &receipt.author_association,
        source_delivery_id: &receipt.source_delivery_id,
        payload_sha256: &receipt.payload_sha256,
    })?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn delivery_index(ledger: &GithubReviewLedger, kind: GithubWebhookKind) -> usize {
    ledger
        .deliveries
        .iter()
        .position(|delivery| delivery.kind == kind)
        .unwrap()
}

#[test]
fn receipt_semantics_remain_bound_after_recomputing_the_unkeyed_receipt_digest() {
    let payload = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit", "2026-09-05T02:00:00Z", &payload);
    ledger.validate().unwrap();

    assert_receipt_tamper(&ledger, "changed review ID", |receipt| {
        receipt.review_id = 42;
    });
    assert_receipt_tamper(&ledger, "changed review node ID", |receipt| {
        receipt.review_node_id = "PRR_42".to_owned();
    });
    assert_receipt_tamper(&ledger, "changed reviewer ID", |receipt| {
        receipt.reviewer_id = 18;
    });
    assert_receipt_tamper(&ledger, "changed reviewer node ID", |receipt| {
        receipt.reviewer_node_id = "U_18".to_owned();
    });
    assert_receipt_tamper(&ledger, "changed reviewer login", |receipt| {
        receipt.reviewer_login = "mallory".to_owned();
    });
    assert_receipt_tamper(&ledger, "changed review state", |receipt| {
        receipt.state = CompletedReviewState::ChangesRequested;
    });
    assert_receipt_tamper(&ledger, "changed commit", |receipt| {
        receipt.commit_id = "d".repeat(40);
    });
    assert_receipt_tamper(&ledger, "changed submission time", |receipt| {
        receipt.submitted_at = "2026-09-05T01:03:04Z".to_owned();
    });
    assert_receipt_tamper(&ledger, "changed review URL", |receipt| {
        receipt.html_url = "https://github.com/acme/widgets/pull/7#forged".to_owned();
    });
    assert_receipt_tamper(&ledger, "changed author association", |receipt| {
        receipt.author_association = "OWNER".to_owned();
    });
}

#[test]
fn submitted_delivery_and_receipt_cardinality_is_bidirectional() {
    let payload = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit", "2026-09-05T02:00:00Z", &payload);

    let mut missing = ledger.clone();
    missing.review_receipts.clear();
    assert_invalid(&missing, "submitted delivery with no receipt");

    let mut extra = ledger;
    let mut forged = extra.review_receipts[0].clone();
    forged.review_id = 42;
    forged.review_node_id = "PRR_42".to_owned();
    forged.receipt_sha256 = receipt_digest(&forged).unwrap();
    extra.review_receipts.push(forged);
    assert_invalid(&extra, "submitted delivery with two receipts");
}

#[test]
fn submitted_fact_cannot_move_between_signed_deliveries() {
    let first = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit-a", "2026-09-05T02:00:00Z", &first);
    let second = approved_payload(42, 18, 'd');
    let mut ledger = ingest_review(
        Some(ledger),
        "delivery-submit-b",
        "2026-09-05T02:01:00Z",
        &second,
    );

    let first_source = ledger.review_receipts[0].source_delivery_id.clone();
    let first_payload = ledger.review_receipts[0].payload_sha256.clone();
    ledger.review_receipts[0].source_delivery_id =
        ledger.review_receipts[1].source_delivery_id.clone();
    ledger.review_receipts[0].payload_sha256 = ledger.review_receipts[1].payload_sha256.clone();
    ledger.review_receipts[1].source_delivery_id = first_source;
    ledger.review_receipts[1].payload_sha256 = first_payload;
    for receipt in &mut ledger.review_receipts {
        receipt.receipt_sha256 = receipt_digest(receipt).unwrap();
    }

    assert_invalid(&ledger, "receipts moved between signed deliveries");
}

#[test]
fn signed_ignored_comment_cannot_be_converted_into_an_approval() {
    let commented = review_payload(
        "submitted",
        "commented",
        41,
        17,
        Some("cccccccccccccccccccccccccccccccccccccccc"),
        Some("2026-09-05T01:02:03Z"),
    );
    let mut ledger = ingest_review(None, "delivery-comment", "2026-09-05T02:00:00Z", &commented);
    assert!(ledger.review_receipts.is_empty());

    let approved = approved_payload(41, 17, 'c');
    let template = ingest_review(None, "delivery-template", "2026-09-05T02:00:00Z", &approved);
    let mut forged = template.review_receipts[0].clone();
    forged.source_delivery_id = ledger.deliveries[0].delivery_id.clone();
    forged.payload_sha256 = ledger.deliveries[0].payload_sha256.clone();
    forged.receipt_sha256 = receipt_digest(&forged).unwrap();
    ledger.review_receipts.push(forged);

    assert_invalid(&ledger, "approval attached to a signed ignored comment");
}

#[test]
fn dismissal_semantics_and_nullable_metadata_are_bound() {
    let payload = dismissed_payload(
        41,
        17,
        Some("cccccccccccccccccccccccccccccccccccccccc"),
        Some("2026-09-05T01:02:03Z"),
    );
    let ledger = ingest_review(None, "delivery-dismiss", "2026-09-05T02:00:00Z", &payload);
    ledger.validate().unwrap();

    assert_dismissal_tamper(&ledger, "changed dismissed review ID", |dismissal| {
        dismissal.review_id = 42;
    });
    assert_dismissal_tamper(&ledger, "changed dismissed reviewer ID", |dismissal| {
        dismissal.reviewer_id = 18;
    });
    assert_dismissal_tamper(&ledger, "changed dismissed reviewer node ID", |dismissal| {
        dismissal.reviewer_node_id = "U_18".to_owned();
    });
    assert_dismissal_tamper(&ledger, "changed dismissed reviewer login", |dismissal| {
        dismissal.reviewer_login = "mallory".to_owned();
    });
    assert_dismissal_tamper(&ledger, "changed dismissed commit", |dismissal| {
        dismissal.commit_id = Some("d".repeat(40));
    });
    assert_dismissal_tamper(&ledger, "removed dismissed commit", |dismissal| {
        dismissal.commit_id = None;
    });
    assert_dismissal_tamper(&ledger, "changed dismissed submission time", |dismissal| {
        dismissal.submitted_at = Some("2026-09-05T01:03:04Z".to_owned());
    });
    assert_dismissal_tamper(&ledger, "removed dismissed submission time", |dismissal| {
        dismissal.submitted_at = None;
    });
    assert_dismissal_tamper(&ledger, "changed dismissal receive time", |dismissal| {
        dismissal.received_at = "2026-09-05T02:01:00Z".to_owned();
    });

    let null_payload = dismissed_payload(42, 18, None, None);
    let null_ledger = ingest_review(
        None,
        "delivery-dismiss-null",
        "2026-09-05T02:02:00Z",
        &null_payload,
    );
    assert_dismissal_tamper(&null_ledger, "invented dismissed commit", |dismissal| {
        dismissal.commit_id = Some("e".repeat(40));
    });
    assert_dismissal_tamper(
        &null_ledger,
        "invented dismissed submission time",
        |dismissal| {
            dismissal.submitted_at = Some("2026-09-05T01:04:05Z".to_owned());
        },
    );
}

#[test]
fn dismissal_delivery_and_fact_cardinality_is_bidirectional() {
    let payload = dismissed_payload(41, 17, None, None);
    let ledger = ingest_review(None, "delivery-dismiss", "2026-09-05T02:00:00Z", &payload);

    let mut missing = ledger.clone();
    missing.dismissals.clear();
    assert_invalid(&missing, "dismissed delivery with no dismissal fact");

    let mut extra = ledger;
    extra.dismissals.push(extra.dismissals[0].clone());
    assert_invalid(&extra, "dismissed delivery with two dismissal facts");
}

#[test]
fn synchronize_commits_and_fact_cardinality_are_bound() {
    let payload = synchronize_payload(
        "0000000000000000000000000000000000000000",
        "1111111111111111111111111111111111111111",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let ledger = ingest_synchronize(None, "delivery-sync", "2026-09-05T02:00:00Z", &payload);
    ledger.validate().unwrap();

    assert_head_tamper(&ledger, "changed synchronize before commit", |head| {
        head.before_commit = "2".repeat(40);
    });
    assert_head_tamper(&ledger, "changed synchronize head commit", |head| {
        head.head_commit = "3".repeat(40);
    });
    assert_head_tamper(&ledger, "changed synchronize base commit", |head| {
        head.base_commit = "4".repeat(40);
    });

    let mut missing = ledger.clone();
    missing.head_observations.clear();
    assert_invalid(&missing, "synchronize delivery with no head observation");

    let mut extra = ledger;
    extra
        .head_observations
        .push(extra.head_observations[0].clone());
    assert_invalid(&extra, "synchronize delivery with two head observations");
}

#[test]
fn facts_cannot_be_attached_to_wrong_delivery_kinds() {
    let submitted = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit", "2026-09-05T02:00:00Z", &submitted);
    let dismissed = dismissed_payload(42, 18, None, None);
    let ledger = ingest_review(
        Some(ledger),
        "delivery-dismiss",
        "2026-09-05T02:01:00Z",
        &dismissed,
    );
    let synchronized = synchronize_payload(
        "0000000000000000000000000000000000000000",
        "1111111111111111111111111111111111111111",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let ledger = ingest_synchronize(
        Some(ledger),
        "delivery-sync",
        "2026-09-05T02:02:00Z",
        &synchronized,
    );
    let submit_index = delivery_index(&ledger, GithubWebhookKind::PullRequestReviewSubmitted);
    let dismiss_index = delivery_index(&ledger, GithubWebhookKind::PullRequestReviewDismissed);

    let mut receipt_on_dismissal = ledger.clone();
    receipt_on_dismissal.review_receipts[0].source_delivery_id = receipt_on_dismissal.deliveries
        [dismiss_index]
        .delivery_id
        .clone();
    receipt_on_dismissal.review_receipts[0].payload_sha256 = receipt_on_dismissal.deliveries
        [dismiss_index]
        .payload_sha256
        .clone();
    receipt_on_dismissal.review_receipts[0].receipt_sha256 =
        receipt_digest(&receipt_on_dismissal.review_receipts[0]).unwrap();
    assert_invalid(
        &receipt_on_dismissal,
        "review receipt attached to a dismissal delivery",
    );

    let mut dismissal_on_submit = ledger.clone();
    dismissal_on_submit.dismissals[0].source_delivery_id = dismissal_on_submit.deliveries
        [submit_index]
        .delivery_id
        .clone();
    dismissal_on_submit.dismissals[0].received_at = dismissal_on_submit.deliveries[submit_index]
        .received_at
        .clone();
    dismissal_on_submit.dismissals[0].payload_sha256 = dismissal_on_submit.deliveries[submit_index]
        .payload_sha256
        .clone();
    assert_invalid(
        &dismissal_on_submit,
        "dismissal attached to a submitted delivery",
    );

    let mut head_on_submit = ledger;
    head_on_submit.head_observations[0].source_delivery_id =
        head_on_submit.deliveries[submit_index].delivery_id.clone();
    head_on_submit.head_observations[0].received_at =
        head_on_submit.deliveries[submit_index].received_at.clone();
    head_on_submit.head_observations[0].payload_sha256 = head_on_submit.deliveries[submit_index]
        .payload_sha256
        .clone();
    assert_invalid(
        &head_on_submit,
        "head observation attached to a submitted delivery",
    );
}

#[test]
fn signed_snapshot_rejects_complete_tuple_deletion_and_splicing() {
    let first_payload = approved_payload(41, 17, 'c');
    let first = ingest_review(None, "delivery-a", "2026-09-05T02:00:00Z", &first_payload);
    let second_payload = approved_payload(42, 18, 'd');
    let second = ingest_review(
        Some(first.clone()),
        "delivery-b",
        "2026-09-05T02:01:00Z",
        &second_payload,
    );

    let mut deleted = second;
    deleted
        .deliveries
        .retain(|delivery| delivery.delivery_id != "delivery-b");
    deleted
        .review_receipts
        .retain(|receipt| receipt.source_delivery_id != "delivery-b");
    assert_invalid(&deleted, "complete signed delivery and fact deletion");

    let standalone_second =
        ingest_review(None, "delivery-b", "2026-09-05T02:01:00Z", &second_payload);
    let mut spliced = first;
    spliced
        .deliveries
        .push(standalone_second.deliveries[0].clone());
    spliced
        .review_receipts
        .push(standalone_second.review_receipts[0].clone());
    assert_invalid(&spliced, "complete signed delivery and fact splicing");
}

#[test]
fn signed_snapshot_binds_revision_counts_body_digest_and_signature() {
    let payload = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit", "2026-09-05T02:00:00Z", &payload);

    let mut revision = ledger.clone();
    revision.snapshot.revision += 1;
    assert_invalid(&revision, "changed snapshot revision");

    let mut counts = ledger.clone();
    counts.snapshot.delivery_count = 0;
    assert_invalid(&counts, "changed snapshot counts");

    let mut body_digest = ledger.clone();
    body_digest.snapshot.body_sha256 = "0".repeat(64);
    assert_invalid(&body_digest, "changed snapshot body digest");

    let mut signature = ledger;
    signature.snapshot.receiver_signature = "0".repeat(128);
    assert_invalid(&signature, "changed snapshot signature");
}

#[test]
fn signed_delivery_fact_digest_cannot_be_replaced_or_detached() {
    let payload = approved_payload(41, 17, 'c');
    let ledger = ingest_review(None, "delivery-submit", "2026-09-05T02:00:00Z", &payload);

    let mut replaced = ledger.clone();
    replaced.deliveries[0].derived_fact_sha256 = Some("0".repeat(64));
    assert_invalid(&replaced, "changed delivery fact digest");

    let mut detached = ledger;
    detached.deliveries[0].derived_fact_sha256 = None;
    assert_invalid(&detached, "removed delivery fact digest");
}
