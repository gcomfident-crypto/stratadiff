use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GITHUB_REVIEW_LEDGER_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/github-review-ledger-v1.schema.json";
pub const MAX_GITHUB_WEBHOOK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_GITHUB_LEDGER_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_LEDGER_DELIVERIES: usize = 10_000;
pub const MAX_LEDGER_REVIEWS: usize = 1_000;
const DELIVERY_ATTESTATION_DOMAIN: &str = "stratadiff-github-delivery-v2";
const RECEIPT_DIGEST_DOMAIN: &str = "stratadiff-github-review-receipt-v1";
const DISMISSAL_DIGEST_DOMAIN: &str = "stratadiff-github-review-dismissal-v1";
const HEAD_OBSERVATION_DIGEST_DOMAIN: &str = "stratadiff-github-head-observation-v1";
const LEDGER_BODY_DIGEST_DOMAIN: &str = "stratadiff-github-review-ledger-body-v1";
const LEDGER_SNAPSHOT_ATTESTATION_DOMAIN: &str = "stratadiff-github-review-ledger-snapshot-v1";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GithubWebhookKind {
    PullRequestReviewSubmitted,
    PullRequestReviewDismissed,
    PullRequestSynchronize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletedReviewState {
    Approved,
    ChangesRequested,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubRepositoryIdentity {
    pub id: u64,
    pub node_id: String,
    pub full_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubPullRequestIdentity {
    pub id: u64,
    pub node_id: String,
    pub number: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubWebhookDelivery {
    pub delivery_id: String,
    pub kind: GithubWebhookKind,
    pub received_at: String,
    pub payload_sha256: String,
    pub derived_fact_sha256: Option<String>,
    pub receiver_signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReceiverIdentity {
    pub algorithm: String,
    pub key_id: String,
    pub public_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubReviewReceipt {
    pub review_id: u64,
    pub review_node_id: String,
    pub reviewer_id: u64,
    pub reviewer_node_id: String,
    pub reviewer_login: String,
    pub state: CompletedReviewState,
    pub commit_id: String,
    pub submitted_at: String,
    pub html_url: String,
    pub author_association: String,
    pub source_delivery_id: String,
    pub payload_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubReviewDismissal {
    pub review_id: u64,
    pub reviewer_id: u64,
    pub reviewer_node_id: String,
    pub reviewer_login: String,
    pub commit_id: Option<String>,
    pub submitted_at: Option<String>,
    pub source_delivery_id: String,
    pub received_at: String,
    pub payload_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubHeadObservation {
    pub before_commit: String,
    pub head_commit: String,
    pub base_commit: String,
    pub source_delivery_id: String,
    pub received_at: String,
    pub payload_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubLedgerSnapshot {
    pub revision: u64,
    pub delivery_count: u64,
    pub review_receipt_count: u64,
    pub dismissal_count: u64,
    pub head_observation_count: u64,
    pub body_sha256: String,
    pub receiver_signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubEffectivePullRequest {
    pub base_commit: String,
    pub head_commit: String,
    pub audited_transition_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubReviewLedger {
    pub schema: String,
    pub provider_url: String,
    pub repository: GithubRepositoryIdentity,
    pub pull_request: GithubPullRequestIdentity,
    pub receiver: ReceiverIdentity,
    pub deliveries: Vec<GithubWebhookDelivery>,
    pub review_receipts: Vec<GithubReviewReceipt>,
    pub dismissals: Vec<GithubReviewDismissal>,
    pub head_observations: Vec<GithubHeadObservation>,
    pub snapshot: GithubLedgerSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestOutcome {
    Applied,
    Duplicate,
}

pub struct GithubWebhookIngest<'a> {
    pub provider_url: &'a str,
    pub event_name: &'a str,
    pub delivery_id: &'a str,
    pub received_at: &'a str,
    pub signature_header: &'a str,
    pub secret: &'a [u8],
    pub receiver_key_id: &'a str,
    pub receiver_signing_key: &'a [u8; 32],
    pub payload: &'a [u8],
}

#[derive(Debug, Deserialize)]
struct RepositoryPayload {
    id: u64,
    node_id: String,
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestPayload {
    id: u64,
    node_id: String,
    number: u64,
    base: PullRequestBranchPayload,
    head: PullRequestBranchPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestBranchPayload {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GithubUserPayload {
    id: u64,
    node_id: String,
    login: String,
    #[serde(rename = "type")]
    account_type: String,
}

#[derive(Debug, Deserialize)]
struct GithubReviewPayload {
    id: u64,
    node_id: String,
    user: Option<GithubUserPayload>,
    state: String,
    commit_id: Option<String>,
    submitted_at: Option<String>,
    html_url: String,
    author_association: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestReviewWebhook {
    action: String,
    review: GithubReviewPayload,
    pull_request: PullRequestPayload,
    repository: RepositoryPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestSynchronizeWebhook {
    action: String,
    before: String,
    after: String,
    pull_request: PullRequestPayload,
    repository: RepositoryPayload,
}

pub fn verify_github_webhook_signature(
    secret: &[u8],
    payload: &[u8],
    signature_header: &str,
) -> Result<()> {
    ensure!(
        !secret.is_empty(),
        "GitHub webhook secret must not be empty"
    );
    ensure!(
        payload.len() <= MAX_GITHUB_WEBHOOK_BYTES,
        "GitHub webhook payload bytes limit exceeded: observed {}, limit {MAX_GITHUB_WEBHOOK_BYTES}",
        payload.len()
    );
    let encoded = signature_header
        .strip_prefix("sha256=")
        .context("GitHub webhook signature must use the sha256= prefix")?;
    let expected = decode_lower_hex_32(encoded)?;
    let mut mac =
        HmacSha256::new_from_slice(secret).context("failed to initialize GitHub webhook HMAC")?;
    mac.update(payload);
    mac.verify_slice(&expected)
        .context("GitHub webhook signature verification failed")
}

pub fn ingest_github_webhook(
    ledger: Option<GithubReviewLedger>,
    ingest: GithubWebhookIngest<'_>,
) -> Result<(GithubReviewLedger, IngestOutcome)> {
    let GithubWebhookIngest {
        provider_url,
        event_name,
        delivery_id,
        received_at,
        signature_header,
        secret,
        receiver_key_id,
        receiver_signing_key,
        payload,
    } = ingest;
    validate_provider_url(provider_url)?;
    validate_delivery_id(delivery_id)?;
    validate_receiver_key_id(receiver_key_id)?;
    ensure!(
        valid_github_timestamp(received_at),
        "GitHub webhook received_at must be an RFC 3339 UTC timestamp with second precision"
    );
    verify_github_webhook_signature(secret, payload, signature_header)?;
    let payload_sha256 = hex_sha256(payload);

    let parsed = match event_name {
        "pull_request_review" => {
            let webhook: PullRequestReviewWebhook = serde_json::from_slice(payload)
                .context("failed to decode GitHub pull_request_review webhook")?;
            ParsedWebhook::Review(webhook)
        }
        "pull_request" => {
            let webhook: PullRequestSynchronizeWebhook = serde_json::from_slice(payload)
                .context("failed to decode GitHub pull_request webhook")?;
            ensure!(
                webhook.action == "synchronize",
                "unsupported GitHub pull_request action: {}",
                webhook.action
            );
            ParsedWebhook::Synchronize(webhook)
        }
        _ => bail!("unsupported GitHub webhook event: {event_name}"),
    };

    let kind = parsed.kind()?;
    let (repository, pull_request) = parsed.identities();
    validate_repository(repository)?;
    validate_pull_request(pull_request)?;
    let signing_key = SigningKey::from_bytes(receiver_signing_key);
    let receiver = ReceiverIdentity {
        algorithm: "ed25519".to_owned(),
        key_id: receiver_key_id.to_owned(),
        public_key: encode_lower_hex(signing_key.verifying_key().as_bytes()),
    };
    let mut ledger = match ledger {
        Some(ledger) => {
            ledger.validate()?;
            ensure!(
                ledger.provider_url == provider_url,
                "GitHub provider URL changed"
            );
            ensure!(
                ledger.repository == repository_identity(repository),
                "GitHub webhook repository identity changed"
            );
            ensure!(
                ledger.pull_request == pull_request_identity(pull_request),
                "GitHub webhook pull request identity changed"
            );
            ensure!(
                ledger.receiver == receiver,
                "receiver signing identity changed"
            );
            ledger
        }
        None => GithubReviewLedger {
            schema: GITHUB_REVIEW_LEDGER_SCHEMA.to_owned(),
            provider_url: provider_url.to_owned(),
            repository: repository_identity(repository),
            pull_request: pull_request_identity(pull_request),
            receiver,
            deliveries: Vec::new(),
            review_receipts: Vec::new(),
            dismissals: Vec::new(),
            head_observations: Vec::new(),
            snapshot: GithubLedgerSnapshot {
                revision: 0,
                delivery_count: 0,
                review_receipt_count: 0,
                dismissal_count: 0,
                head_observation_count: 0,
                body_sha256: String::new(),
                receiver_signature: String::new(),
            },
        },
    };

    if let Some(existing) = ledger
        .deliveries
        .iter()
        .find(|existing| existing.delivery_id == delivery_id)
    {
        ensure!(
            existing.kind == kind && existing.payload_sha256 == payload_sha256,
            "GitHub delivery ID {delivery_id} was reused with different content"
        );
        return Ok((ledger, IngestOutcome::Duplicate));
    }
    ensure!(
        ledger.deliveries.len() < MAX_LEDGER_DELIVERIES,
        "GitHub ledger delivery limit exceeded"
    );

    let fact = DerivedFact::from_parsed(parsed, delivery_id, received_at, &payload_sha256)?;
    let mut delivery = GithubWebhookDelivery {
        delivery_id: delivery_id.to_owned(),
        kind,
        received_at: received_at.to_owned(),
        payload_sha256: payload_sha256.clone(),
        derived_fact_sha256: fact.digest()?,
        receiver_signature: String::new(),
    };
    delivery.receiver_signature = sign_delivery(&ledger, &delivery, &signing_key)?;
    fact.append_to(&mut ledger)?;
    ledger.deliveries.push(delivery);
    ledger.sort();
    let revision = ledger
        .snapshot
        .revision
        .checked_add(1)
        .context("GitHub ledger snapshot revision overflow")?;
    sign_ledger_snapshot(&mut ledger, revision, &signing_key)?;
    ledger.validate()?;
    Ok((ledger, IngestOutcome::Applied))
}

enum ParsedWebhook {
    Review(PullRequestReviewWebhook),
    Synchronize(PullRequestSynchronizeWebhook),
}

impl ParsedWebhook {
    fn kind(&self) -> Result<GithubWebhookKind> {
        match self {
            Self::Review(webhook) => match webhook.action.as_str() {
                "submitted" => Ok(GithubWebhookKind::PullRequestReviewSubmitted),
                "dismissed" => Ok(GithubWebhookKind::PullRequestReviewDismissed),
                action => bail!("unsupported GitHub pull_request_review action: {action}"),
            },
            Self::Synchronize(_) => Ok(GithubWebhookKind::PullRequestSynchronize),
        }
    }

    fn identities(&self) -> (&RepositoryPayload, &PullRequestPayload) {
        match self {
            Self::Review(webhook) => (&webhook.repository, &webhook.pull_request),
            Self::Synchronize(webhook) => (&webhook.repository, &webhook.pull_request),
        }
    }
}

enum DerivedFact {
    ReviewReceipt(GithubReviewReceipt),
    Dismissal(GithubReviewDismissal),
    HeadObservation(GithubHeadObservation),
    IgnoredReview,
}

impl DerivedFact {
    fn from_parsed(
        parsed: ParsedWebhook,
        delivery_id: &str,
        received_at: &str,
        payload_sha256: &str,
    ) -> Result<Self> {
        match parsed {
            ParsedWebhook::Review(webhook) => match webhook.action.as_str() {
                "submitted" => Ok(
                    match submitted_receipt(&webhook.review, delivery_id, payload_sha256)? {
                        Some(receipt) => Self::ReviewReceipt(receipt),
                        None => Self::IgnoredReview,
                    },
                ),
                "dismissed" => {
                    ensure!(
                        webhook.review.state.eq_ignore_ascii_case("dismissed"),
                        "dismissed review webhook has non-dismissed review state"
                    );
                    let user = webhook
                        .review
                        .user
                        .as_ref()
                        .context("dismissed GitHub review has no reviewer identity")?;
                    ensure!(
                        user.account_type == "User",
                        "dismissed GitHub review was not submitted by a human user"
                    );
                    let dismissal = GithubReviewDismissal {
                        review_id: webhook.review.id,
                        reviewer_id: user.id,
                        reviewer_node_id: user.node_id.clone(),
                        reviewer_login: user.login.clone(),
                        commit_id: webhook.review.commit_id,
                        submitted_at: webhook.review.submitted_at,
                        source_delivery_id: delivery_id.to_owned(),
                        received_at: received_at.to_owned(),
                        payload_sha256: payload_sha256.to_owned(),
                    };
                    validate_review_dismissal(&dismissal)?;
                    Ok(Self::Dismissal(dismissal))
                }
                _ => unreachable!("review action was validated while deriving the kind"),
            },
            ParsedWebhook::Synchronize(webhook) => {
                ensure!(
                    is_sha1(&webhook.before),
                    "synchronize before is not a full SHA-1"
                );
                ensure!(
                    is_sha1(&webhook.after),
                    "synchronize after is not a full SHA-1"
                );
                ensure!(
                    webhook.after == webhook.pull_request.head.sha,
                    "synchronize after does not equal pull request head SHA"
                );
                ensure!(
                    is_sha1(&webhook.pull_request.base.sha),
                    "pull request base is not a full SHA-1"
                );
                let observation = GithubHeadObservation {
                    before_commit: webhook.before,
                    head_commit: webhook.after,
                    base_commit: webhook.pull_request.base.sha,
                    source_delivery_id: delivery_id.to_owned(),
                    received_at: received_at.to_owned(),
                    payload_sha256: payload_sha256.to_owned(),
                };
                validate_head_observation(&observation)?;
                Ok(Self::HeadObservation(observation))
            }
        }
    }

    fn digest(&self) -> Result<Option<String>> {
        match self {
            Self::ReviewReceipt(receipt) => Ok(Some(receipt_digest(receipt)?)),
            Self::Dismissal(dismissal) => Ok(Some(dismissal_digest(dismissal)?)),
            Self::HeadObservation(observation) => Ok(Some(head_observation_digest(observation)?)),
            Self::IgnoredReview => Ok(None),
        }
    }

    fn append_to(self, ledger: &mut GithubReviewLedger) -> Result<()> {
        match self {
            Self::ReviewReceipt(receipt) => insert_review_receipt(ledger, receipt),
            Self::Dismissal(dismissal) => {
                ledger.dismissals.push(dismissal);
                Ok(())
            }
            Self::HeadObservation(observation) => {
                ledger.head_observations.push(observation);
                Ok(())
            }
            Self::IgnoredReview => Ok(()),
        }
    }
}

impl GithubReviewLedger {
    pub fn active_receipts(&self) -> Vec<&GithubReviewReceipt> {
        enum Latest<'a> {
            Receipt(&'a GithubReviewReceipt),
            Dismissal {
                dismissal: &'a GithubReviewDismissal,
                submitted_at: &'a str,
            },
        }
        impl Latest<'_> {
            fn key(&self) -> (&str, u64, u8) {
                match self {
                    Self::Receipt(receipt) => (receipt.submitted_at.as_str(), receipt.review_id, 0),
                    Self::Dismissal {
                        dismissal,
                        submitted_at,
                    } => (submitted_at, dismissal.review_id, 1),
                }
            }
        }
        let receipts_by_id = self
            .review_receipts
            .iter()
            .map(|receipt| (receipt.review_id, receipt))
            .collect::<HashMap<_, _>>();
        let mut latest = HashMap::<u64, Latest<'_>>::new();
        let mut ambiguous_reviewers = HashSet::new();
        for receipt in &self.review_receipts {
            let candidate = Latest::Receipt(receipt);
            let replace = latest
                .get(&receipt.reviewer_id)
                .is_none_or(|existing| existing.key() < candidate.key());
            if replace {
                latest.insert(receipt.reviewer_id, candidate);
            }
        }
        for dismissal in &self.dismissals {
            let submitted_at = dismissal.submitted_at.as_deref().or_else(|| {
                receipts_by_id
                    .get(&dismissal.review_id)
                    .map(|receipt| receipt.submitted_at.as_str())
            });
            let Some(submitted_at) = submitted_at else {
                ambiguous_reviewers.insert(dismissal.reviewer_id);
                continue;
            };
            let candidate = Latest::Dismissal {
                dismissal,
                submitted_at,
            };
            let replace = latest
                .get(&dismissal.reviewer_id)
                .is_none_or(|existing| existing.key() < candidate.key());
            if replace {
                latest.insert(dismissal.reviewer_id, candidate);
            }
        }
        let mut receipts = latest
            .into_values()
            .filter_map(|latest| match latest {
                Latest::Receipt(receipt) if !ambiguous_reviewers.contains(&receipt.reviewer_id) => {
                    Some(receipt)
                }
                Latest::Receipt(_) | Latest::Dismissal { .. } => None,
            })
            .collect::<Vec<_>>();
        receipts.sort_by_key(|receipt| receipt.reviewer_id);
        receipts
    }

    pub fn reconcile_current_head(
        &self,
        authoritative_base_commit: &str,
        authoritative_head_commit: &str,
    ) -> Result<GithubEffectivePullRequest> {
        ensure!(
            is_object_id(authoritative_base_commit),
            "authoritative pull request base is not a full object ID"
        );
        ensure!(
            is_object_id(authoritative_head_commit),
            "authoritative pull request head is not a full object ID"
        );
        self.validate()?;

        let mut successors = HashMap::<&str, &str>::new();
        let mut predecessors = HashMap::<&str, &str>::new();
        let mut transitions = HashSet::<(&str, &str)>::new();
        for observation in &self.head_observations {
            ensure!(
                observation.before_commit != observation.head_commit,
                "synchronize transition does not advance the pull request head"
            );
            let edge = (
                observation.before_commit.as_str(),
                observation.head_commit.as_str(),
            );
            if !transitions.insert(edge) {
                continue;
            }
            if let Some(existing) = successors.insert(edge.0, edge.1) {
                ensure!(
                    existing == edge.1,
                    "synchronize transition history has conflicting successors"
                );
            }
            if let Some(existing) = predecessors.insert(edge.1, edge.0) {
                ensure!(
                    existing == edge.0,
                    "synchronize transition history has conflicting predecessors"
                );
            }
        }

        if !transitions.is_empty() {
            let mut visited = HashSet::new();
            let mut cursor = authoritative_head_commit;
            while let Some(before) = predecessors.get(cursor).copied() {
                ensure!(
                    visited.insert((before, cursor)),
                    "synchronize transition history contains a cycle"
                );
                cursor = before;
            }
            ensure!(
                visited.len() == transitions.len(),
                "synchronize transition history is disconnected from the authoritative pull request head"
            );
        }

        Ok(GithubEffectivePullRequest {
            base_commit: authoritative_base_commit.to_owned(),
            head_commit: authoritative_head_commit.to_owned(),
            audited_transition_count: self.head_observations.len(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == GITHUB_REVIEW_LEDGER_SCHEMA,
            "unsupported GitHub review ledger schema"
        );
        validate_provider_url(&self.provider_url)?;
        validate_repository_identity(&self.repository)?;
        validate_pull_request_identity(&self.pull_request)?;
        validate_receiver_identity(&self.receiver)?;
        ensure!(
            self.deliveries.len() <= MAX_LEDGER_DELIVERIES,
            "GitHub ledger delivery limit exceeded"
        );
        ensure!(
            self.review_receipts.len() <= MAX_LEDGER_REVIEWS,
            "GitHub ledger review receipt limit exceeded"
        );

        let mut delivery_ids = HashSet::new();
        let mut deliveries_by_id = HashMap::new();
        for delivery in &self.deliveries {
            validate_delivery_id(&delivery.delivery_id)?;
            ensure!(
                delivery_ids.insert(delivery.delivery_id.as_str()),
                "duplicate GitHub delivery ID in ledger"
            );
            deliveries_by_id.insert(delivery.delivery_id.as_str(), delivery);
            ensure!(
                valid_github_timestamp(&delivery.received_at),
                "invalid GitHub delivery timestamp"
            );
            ensure!(
                is_sha256(&delivery.payload_sha256),
                "invalid payload SHA-256"
            );
            if let Some(digest) = &delivery.derived_fact_sha256 {
                ensure!(is_sha256(digest), "invalid derived fact SHA-256");
            }
            verify_delivery_signature(self, delivery)?;
        }

        let mut review_ids = HashSet::new();
        let mut receipt_delivery_ids = HashSet::new();
        for receipt in &self.review_receipts {
            ensure!(
                review_ids.insert(receipt.review_id),
                "duplicate review receipt"
            );
            validate_review_receipt(receipt)?;
            let delivery = deliveries_by_id
                .get(receipt.source_delivery_id.as_str())
                .context("review receipt references an unknown delivery")?;
            ensure!(
                delivery.kind == GithubWebhookKind::PullRequestReviewSubmitted,
                "review receipt references a non-submitted delivery"
            );
            ensure!(
                receipt_delivery_ids.insert(receipt.source_delivery_id.as_str()),
                "submitted delivery has multiple review receipts"
            );
            ensure!(
                receipt.payload_sha256 == delivery.payload_sha256,
                "review receipt does not match its submitted delivery"
            );
            let digest = receipt_digest(receipt)?;
            ensure!(
                receipt.receipt_sha256 == digest,
                "review receipt digest mismatch"
            );
            ensure!(
                delivery.derived_fact_sha256.as_deref() == Some(digest.as_str()),
                "review receipt does not match its signed delivery fact"
            );
        }
        let mut dismissal_delivery_ids = HashSet::new();
        for dismissal in &self.dismissals {
            validate_review_dismissal(dismissal)?;
            let delivery = deliveries_by_id
                .get(dismissal.source_delivery_id.as_str())
                .context("review dismissal references an unknown delivery")?;
            ensure!(
                delivery.kind == GithubWebhookKind::PullRequestReviewDismissed,
                "review dismissal references a non-dismissed delivery"
            );
            ensure!(
                dismissal_delivery_ids.insert(dismissal.source_delivery_id.as_str()),
                "dismissed delivery has multiple review dismissals"
            );
            ensure!(
                dismissal.received_at == delivery.received_at
                    && dismissal.payload_sha256 == delivery.payload_sha256,
                "review dismissal does not match its dismissed delivery"
            );
            let digest = dismissal_digest(dismissal)?;
            ensure!(
                delivery.derived_fact_sha256.as_deref() == Some(digest.as_str()),
                "review dismissal does not match its signed delivery fact"
            );
            if let Some(receipt) = self
                .review_receipts
                .iter()
                .find(|receipt| receipt.review_id == dismissal.review_id)
            {
                ensure!(
                    receipt.reviewer_id == dismissal.reviewer_id
                        && receipt.reviewer_node_id == dismissal.reviewer_node_id
                        && receipt
                            .reviewer_login
                            .eq_ignore_ascii_case(&dismissal.reviewer_login)
                        && dismissal
                            .commit_id
                            .as_ref()
                            .is_none_or(|commit_id| receipt.commit_id == *commit_id)
                        && dismissal
                            .submitted_at
                            .as_ref()
                            .is_none_or(|submitted_at| receipt.submitted_at == *submitted_at),
                    "review dismissal conflicts with its immutable receipt"
                );
            }
        }
        let mut head_delivery_ids = HashSet::new();
        for head in &self.head_observations {
            validate_head_observation(head)?;
            let delivery = deliveries_by_id
                .get(head.source_delivery_id.as_str())
                .context("head observation references an unknown delivery")?;
            ensure!(
                delivery.kind == GithubWebhookKind::PullRequestSynchronize,
                "head observation references a non-synchronize delivery"
            );
            ensure!(
                head_delivery_ids.insert(head.source_delivery_id.as_str()),
                "synchronize delivery has multiple head observations"
            );
            ensure!(
                head.received_at == delivery.received_at
                    && head.payload_sha256 == delivery.payload_sha256,
                "head observation does not match its synchronize delivery"
            );
            let digest = head_observation_digest(head)?;
            ensure!(
                delivery.derived_fact_sha256.as_deref() == Some(digest.as_str()),
                "head observation does not match its signed delivery fact"
            );
        }
        for delivery in &self.deliveries {
            match delivery.kind {
                GithubWebhookKind::PullRequestReviewSubmitted => {
                    if delivery.derived_fact_sha256.is_some() {
                        ensure!(
                            receipt_delivery_ids.contains(delivery.delivery_id.as_str()),
                            "submitted delivery is missing its review receipt"
                        );
                    } else {
                        ensure!(
                            !receipt_delivery_ids.contains(delivery.delivery_id.as_str()),
                            "ignored submitted delivery has a review receipt"
                        );
                    }
                }
                GithubWebhookKind::PullRequestReviewDismissed => {
                    ensure!(
                        delivery.derived_fact_sha256.is_some(),
                        "dismissed delivery is missing its signed fact digest"
                    );
                    ensure!(
                        dismissal_delivery_ids.contains(delivery.delivery_id.as_str()),
                        "dismissed delivery is missing its review dismissal"
                    );
                }
                GithubWebhookKind::PullRequestSynchronize => {
                    ensure!(
                        delivery.derived_fact_sha256.is_some(),
                        "synchronize delivery is missing its signed fact digest"
                    );
                    ensure!(
                        head_delivery_ids.contains(delivery.delivery_id.as_str()),
                        "synchronize delivery is missing its head observation"
                    );
                }
            }
        }
        verify_ledger_snapshot(self)?;
        Ok(())
    }

    fn sort(&mut self) {
        self.deliveries
            .sort_by(|left, right| left.delivery_id.cmp(&right.delivery_id));
        self.review_receipts
            .sort_by_key(|receipt| receipt.review_id);
        self.dismissals.sort_by(|left, right| {
            (left.review_id, left.source_delivery_id.as_str())
                .cmp(&(right.review_id, right.source_delivery_id.as_str()))
        });
        self.head_observations
            .sort_by(|left, right| left.source_delivery_id.cmp(&right.source_delivery_id));
    }
}

fn submitted_receipt(
    review: &GithubReviewPayload,
    delivery_id: &str,
    payload_sha256: &str,
) -> Result<Option<GithubReviewReceipt>> {
    let state = match review.state.to_ascii_lowercase().as_str() {
        "approved" => CompletedReviewState::Approved,
        "changes_requested" => CompletedReviewState::ChangesRequested,
        "commented" => return Ok(None),
        state => bail!("unsupported submitted GitHub review state: {state}"),
    };
    let user = review
        .user
        .as_ref()
        .context("completed GitHub review has no reviewer identity")?;
    ensure!(
        user.account_type == "User",
        "completed GitHub review was not submitted by a human user"
    );
    let commit_id = review
        .commit_id
        .as_ref()
        .context("completed GitHub review has no commit ID")?;
    let submitted_at = review
        .submitted_at
        .as_ref()
        .context("completed GitHub review has no submission time")?;
    let mut receipt = GithubReviewReceipt {
        review_id: review.id,
        review_node_id: review.node_id.clone(),
        reviewer_id: user.id,
        reviewer_node_id: user.node_id.clone(),
        reviewer_login: user.login.clone(),
        state,
        commit_id: commit_id.clone(),
        submitted_at: submitted_at.clone(),
        html_url: review.html_url.clone(),
        author_association: review.author_association.clone(),
        source_delivery_id: delivery_id.to_owned(),
        payload_sha256: payload_sha256.to_owned(),
        receipt_sha256: String::new(),
    };
    validate_review_receipt(&receipt)?;
    receipt.receipt_sha256 = receipt_digest(&receipt)?;
    Ok(Some(receipt))
}

fn insert_review_receipt(
    ledger: &mut GithubReviewLedger,
    receipt: GithubReviewReceipt,
) -> Result<()> {
    if let Some(existing) = ledger
        .review_receipts
        .iter()
        .find(|existing| existing.review_id == receipt.review_id)
    {
        ensure!(
            existing == &receipt,
            "GitHub review ID {} was reused with different immutable fields",
            receipt.review_id
        );
        return Ok(());
    }
    ensure!(
        ledger.review_receipts.len() < MAX_LEDGER_REVIEWS,
        "GitHub ledger review receipt limit exceeded"
    );
    ledger.review_receipts.push(receipt);
    Ok(())
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
        domain: RECEIPT_DIGEST_DOMAIN,
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
    Ok(hex_sha256(&encoded))
}

fn dismissal_digest(dismissal: &GithubReviewDismissal) -> Result<String> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        domain: &'static str,
        review_id: u64,
        reviewer_id: u64,
        reviewer_node_id: &'a str,
        reviewer_login: &'a str,
        commit_id: Option<&'a str>,
        submitted_at: Option<&'a str>,
        source_delivery_id: &'a str,
        received_at: &'a str,
        payload_sha256: &'a str,
    }
    let encoded = serde_json::to_vec(&DigestInput {
        domain: DISMISSAL_DIGEST_DOMAIN,
        review_id: dismissal.review_id,
        reviewer_id: dismissal.reviewer_id,
        reviewer_node_id: &dismissal.reviewer_node_id,
        reviewer_login: &dismissal.reviewer_login,
        commit_id: dismissal.commit_id.as_deref(),
        submitted_at: dismissal.submitted_at.as_deref(),
        source_delivery_id: &dismissal.source_delivery_id,
        received_at: &dismissal.received_at,
        payload_sha256: &dismissal.payload_sha256,
    })?;
    Ok(hex_sha256(&encoded))
}

fn head_observation_digest(observation: &GithubHeadObservation) -> Result<String> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        domain: &'static str,
        before_commit: &'a str,
        head_commit: &'a str,
        base_commit: &'a str,
        source_delivery_id: &'a str,
        received_at: &'a str,
        payload_sha256: &'a str,
    }
    let encoded = serde_json::to_vec(&DigestInput {
        domain: HEAD_OBSERVATION_DIGEST_DOMAIN,
        before_commit: &observation.before_commit,
        head_commit: &observation.head_commit,
        base_commit: &observation.base_commit,
        source_delivery_id: &observation.source_delivery_id,
        received_at: &observation.received_at,
        payload_sha256: &observation.payload_sha256,
    })?;
    Ok(hex_sha256(&encoded))
}

fn validate_review_receipt(receipt: &GithubReviewReceipt) -> Result<()> {
    ensure!(receipt.review_id != 0, "GitHub review ID must not be zero");
    ensure!(
        receipt.reviewer_id != 0,
        "GitHub reviewer ID must not be zero"
    );
    ensure!(
        !receipt.review_node_id.is_empty(),
        "GitHub review node ID is empty"
    );
    ensure!(
        !receipt.reviewer_node_id.is_empty(),
        "GitHub reviewer node ID is empty"
    );
    ensure!(
        !receipt.reviewer_login.is_empty(),
        "GitHub reviewer login is empty"
    );
    ensure!(
        receipt.reviewer_login.trim() == receipt.reviewer_login,
        "GitHub reviewer login contains surrounding whitespace"
    );
    ensure!(
        is_sha1(&receipt.commit_id),
        "GitHub review commit is not a full SHA-1"
    );
    ensure!(
        valid_github_timestamp(&receipt.submitted_at),
        "GitHub review submission time is invalid"
    );
    ensure!(!receipt.html_url.is_empty(), "GitHub review URL is empty");
    ensure!(
        !receipt.author_association.is_empty(),
        "GitHub review author association is empty"
    );
    validate_delivery_id(&receipt.source_delivery_id)?;
    ensure!(
        is_sha256(&receipt.payload_sha256),
        "invalid payload SHA-256"
    );
    ensure!(
        receipt.receipt_sha256.is_empty() || is_sha256(&receipt.receipt_sha256),
        "invalid review receipt SHA-256"
    );
    Ok(())
}

fn validate_review_dismissal(dismissal: &GithubReviewDismissal) -> Result<()> {
    ensure!(
        dismissal.review_id != 0,
        "GitHub review ID must not be zero"
    );
    ensure!(
        dismissal.reviewer_id != 0,
        "GitHub reviewer ID must not be zero"
    );
    ensure!(
        !dismissal.reviewer_node_id.is_empty(),
        "GitHub reviewer node ID is empty"
    );
    ensure!(
        !dismissal.reviewer_login.is_empty(),
        "GitHub reviewer login is empty"
    );
    ensure!(
        dismissal.reviewer_login.trim() == dismissal.reviewer_login,
        "GitHub reviewer login contains surrounding whitespace"
    );
    if let Some(commit_id) = &dismissal.commit_id {
        ensure!(
            is_sha1(commit_id),
            "dismissed GitHub review commit is not a full SHA-1"
        );
    }
    if let Some(submitted_at) = &dismissal.submitted_at {
        ensure!(
            valid_github_timestamp(submitted_at),
            "dismissed GitHub review submission time is invalid"
        );
    }
    validate_delivery_id(&dismissal.source_delivery_id)?;
    ensure!(
        valid_github_timestamp(&dismissal.received_at),
        "invalid review dismissal timestamp"
    );
    ensure!(
        is_sha256(&dismissal.payload_sha256),
        "invalid payload SHA-256"
    );
    Ok(())
}

fn validate_head_observation(observation: &GithubHeadObservation) -> Result<()> {
    ensure!(
        is_sha1(&observation.before_commit)
            && is_sha1(&observation.head_commit)
            && is_sha1(&observation.base_commit),
        "head observation contains an invalid commit ID"
    );
    validate_delivery_id(&observation.source_delivery_id)?;
    ensure!(
        valid_github_timestamp(&observation.received_at),
        "invalid head observation timestamp"
    );
    ensure!(
        is_sha256(&observation.payload_sha256),
        "invalid payload SHA-256"
    );
    Ok(())
}

fn repository_identity(payload: &RepositoryPayload) -> GithubRepositoryIdentity {
    GithubRepositoryIdentity {
        id: payload.id,
        node_id: payload.node_id.clone(),
        full_name: payload.full_name.clone(),
    }
}

fn pull_request_identity(payload: &PullRequestPayload) -> GithubPullRequestIdentity {
    GithubPullRequestIdentity {
        id: payload.id,
        node_id: payload.node_id.clone(),
        number: payload.number,
    }
}

fn validate_repository(payload: &RepositoryPayload) -> Result<()> {
    validate_repository_identity(&repository_identity(payload))
}

fn validate_repository_identity(identity: &GithubRepositoryIdentity) -> Result<()> {
    ensure!(identity.id != 0, "GitHub repository ID must not be zero");
    ensure!(
        !identity.node_id.is_empty(),
        "GitHub repository node ID is empty"
    );
    ensure!(
        identity.full_name.split_once('/').is_some(),
        "GitHub repository full name must contain owner/name"
    );
    Ok(())
}

fn validate_pull_request(payload: &PullRequestPayload) -> Result<()> {
    validate_pull_request_identity(&pull_request_identity(payload))?;
    ensure!(
        is_sha1(&payload.base.sha),
        "pull request base is not a full SHA-1"
    );
    ensure!(
        is_sha1(&payload.head.sha),
        "pull request head is not a full SHA-1"
    );
    Ok(())
}

fn validate_pull_request_identity(identity: &GithubPullRequestIdentity) -> Result<()> {
    ensure!(identity.id != 0, "GitHub pull request ID must not be zero");
    ensure!(
        !identity.node_id.is_empty(),
        "GitHub pull request node ID is empty"
    );
    ensure!(
        identity.number != 0,
        "GitHub pull request number must not be zero"
    );
    Ok(())
}

fn validate_provider_url(value: &str) -> Result<()> {
    let authority = value.strip_prefix("https://").unwrap_or_default();
    ensure!(
        !authority.is_empty()
            && !authority.contains('/')
            && !authority.chars().any(char::is_whitespace),
        "GitHub provider URL must be an HTTPS origin without a path or trailing slash"
    );
    Ok(())
}

pub fn decode_ed25519_signing_key(value: &str) -> Result<[u8; 32]> {
    decode_lower_hex_32(value).context("receiver Ed25519 signing key is invalid")
}

fn validate_receiver_identity(identity: &ReceiverIdentity) -> Result<()> {
    ensure!(
        identity.algorithm == "ed25519",
        "unsupported receiver signature algorithm"
    );
    validate_receiver_key_id(&identity.key_id)?;
    decode_lower_hex_32(&identity.public_key).context("receiver Ed25519 public key is invalid")?;
    Ok(())
}

fn validate_receiver_key_id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            }),
        "receiver signing key ID is invalid"
    );
    Ok(())
}

fn sign_delivery(
    ledger: &GithubReviewLedger,
    delivery: &GithubWebhookDelivery,
    signing_key: &SigningKey,
) -> Result<String> {
    let message = delivery_attestation_message(ledger, delivery)?;
    Ok(encode_lower_hex(&signing_key.sign(&message).to_bytes()))
}

fn verify_delivery_signature(
    ledger: &GithubReviewLedger,
    delivery: &GithubWebhookDelivery,
) -> Result<()> {
    let public_key = decode_lower_hex_32(&ledger.receiver.public_key)
        .context("receiver Ed25519 public key is invalid")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .context("receiver Ed25519 public key is not valid")?;
    let signature_bytes = decode_lower_hex_64(&delivery.receiver_signature)
        .context("receiver Ed25519 signature is invalid")?;
    let signature = Signature::from_bytes(&signature_bytes);
    let message = delivery_attestation_message(ledger, delivery)?;
    verifying_key
        .verify(&message, &signature)
        .context("receiver delivery attestation verification failed")
}

fn delivery_attestation_message(
    ledger: &GithubReviewLedger,
    delivery: &GithubWebhookDelivery,
) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct AttestationInput<'a> {
        domain: &'static str,
        receiver_key_id: &'a str,
        receiver_public_key: &'a str,
        provider_url: &'a str,
        repository: &'a GithubRepositoryIdentity,
        pull_request: &'a GithubPullRequestIdentity,
        delivery_id: &'a str,
        kind: GithubWebhookKind,
        received_at: &'a str,
        payload_sha256: &'a str,
        derived_fact_sha256: Option<&'a str>,
    }
    Ok(serde_json::to_vec(&AttestationInput {
        domain: DELIVERY_ATTESTATION_DOMAIN,
        receiver_key_id: &ledger.receiver.key_id,
        receiver_public_key: &ledger.receiver.public_key,
        provider_url: &ledger.provider_url,
        repository: &ledger.repository,
        pull_request: &ledger.pull_request,
        delivery_id: &delivery.delivery_id,
        kind: delivery.kind,
        received_at: &delivery.received_at,
        payload_sha256: &delivery.payload_sha256,
        derived_fact_sha256: delivery.derived_fact_sha256.as_deref(),
    })?)
}

fn ledger_body_digest(ledger: &GithubReviewLedger) -> Result<String> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        domain: &'static str,
        schema: &'a str,
        provider_url: &'a str,
        repository: &'a GithubRepositoryIdentity,
        pull_request: &'a GithubPullRequestIdentity,
        receiver: &'a ReceiverIdentity,
        deliveries: &'a [GithubWebhookDelivery],
        review_receipts: &'a [GithubReviewReceipt],
        dismissals: &'a [GithubReviewDismissal],
        head_observations: &'a [GithubHeadObservation],
    }
    let encoded = serde_json::to_vec(&DigestInput {
        domain: LEDGER_BODY_DIGEST_DOMAIN,
        schema: &ledger.schema,
        provider_url: &ledger.provider_url,
        repository: &ledger.repository,
        pull_request: &ledger.pull_request,
        receiver: &ledger.receiver,
        deliveries: &ledger.deliveries,
        review_receipts: &ledger.review_receipts,
        dismissals: &ledger.dismissals,
        head_observations: &ledger.head_observations,
    })?;
    Ok(hex_sha256(&encoded))
}

fn ledger_snapshot_attestation_message(
    ledger: &GithubReviewLedger,
    snapshot: &GithubLedgerSnapshot,
) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct AttestationInput<'a> {
        domain: &'static str,
        receiver_key_id: &'a str,
        receiver_public_key: &'a str,
        revision: u64,
        delivery_count: u64,
        review_receipt_count: u64,
        dismissal_count: u64,
        head_observation_count: u64,
        body_sha256: &'a str,
    }
    Ok(serde_json::to_vec(&AttestationInput {
        domain: LEDGER_SNAPSHOT_ATTESTATION_DOMAIN,
        receiver_key_id: &ledger.receiver.key_id,
        receiver_public_key: &ledger.receiver.public_key,
        revision: snapshot.revision,
        delivery_count: snapshot.delivery_count,
        review_receipt_count: snapshot.review_receipt_count,
        dismissal_count: snapshot.dismissal_count,
        head_observation_count: snapshot.head_observation_count,
        body_sha256: &snapshot.body_sha256,
    })?)
}

fn sign_ledger_snapshot(
    ledger: &mut GithubReviewLedger,
    revision: u64,
    signing_key: &SigningKey,
) -> Result<()> {
    let mut snapshot = GithubLedgerSnapshot {
        revision,
        delivery_count: ledger.deliveries.len() as u64,
        review_receipt_count: ledger.review_receipts.len() as u64,
        dismissal_count: ledger.dismissals.len() as u64,
        head_observation_count: ledger.head_observations.len() as u64,
        body_sha256: ledger_body_digest(ledger)?,
        receiver_signature: String::new(),
    };
    let message = ledger_snapshot_attestation_message(ledger, &snapshot)?;
    snapshot.receiver_signature = encode_lower_hex(&signing_key.sign(&message).to_bytes());
    ledger.snapshot = snapshot;
    Ok(())
}

fn verify_ledger_snapshot(ledger: &GithubReviewLedger) -> Result<()> {
    let snapshot = &ledger.snapshot;
    ensure!(
        snapshot.revision != 0,
        "GitHub ledger snapshot revision is zero"
    );
    ensure!(
        snapshot.delivery_count == ledger.deliveries.len() as u64
            && snapshot.review_receipt_count == ledger.review_receipts.len() as u64
            && snapshot.dismissal_count == ledger.dismissals.len() as u64
            && snapshot.head_observation_count == ledger.head_observations.len() as u64,
        "GitHub ledger snapshot counts do not match its body"
    );
    ensure!(
        is_sha256(&snapshot.body_sha256),
        "invalid GitHub ledger body SHA-256"
    );
    ensure!(
        snapshot.body_sha256 == ledger_body_digest(ledger)?,
        "GitHub ledger snapshot body digest mismatch"
    );
    let public_key = decode_lower_hex_32(&ledger.receiver.public_key)
        .context("receiver Ed25519 public key is invalid")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .context("receiver Ed25519 public key is not valid")?;
    let signature_bytes = decode_lower_hex_64(&snapshot.receiver_signature)
        .context("receiver ledger snapshot signature is invalid")?;
    let signature = Signature::from_bytes(&signature_bytes);
    let message = ledger_snapshot_attestation_message(ledger, snapshot)?;
    verifying_key
        .verify(&message, &signature)
        .context("receiver ledger snapshot attestation verification failed")
}

fn validate_delivery_id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "GitHub delivery ID is invalid"
    );
    Ok(())
}

fn decode_lower_hex_32(value: &str) -> Result<[u8; 32]> {
    ensure!(
        value.len() == 64,
        "GitHub webhook SHA-256 signature has the wrong length"
    );
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn decode_lower_hex_64(value: &str) -> Result<[u8; 64]> {
    ensure!(
        value.len() == 128,
        "receiver Ed25519 signature has the wrong length"
    );
    let mut decoded = [0_u8; 64];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => bail!("GitHub webhook SHA-256 signature must use lowercase hexadecimal"),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    encode_lower_hex(&digest)
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_github_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"It's a Secret to Everybody";
    const OFFICIAL_PAYLOAD: &[u8] = b"Hello, World!";
    const OFFICIAL_SIGNATURE: &str =
        "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
    const RECEIVER_KEY_ID: &str = "test-key-2026-09";
    const RECEIVER_SIGNING_KEY: [u8; 32] = [7; 32];

    fn sign(payload: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(SECRET).unwrap();
        mac.update(payload);
        format!("sha256={}", encode_lower_hex(&mac.finalize().into_bytes()))
    }

    fn review_payload(action: &str, state: &str, review_id: u64, commit: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "action": action,
            "review": {
                "id": review_id,
                "node_id": format!("PRR_{review_id}"),
                "user": {"id": 17, "node_id": "U_17", "login": "alice", "type": "User"},
                "state": state,
                "commit_id": commit,
                "submitted_at": "2026-09-05T01:02:03Z",
                "html_url": format!("https://github.com/acme/widgets/pull/7#pullrequestreview-{review_id}"),
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
        serde_json::to_vec(&serde_json::json!({
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

    fn ingest(
        ledger: Option<GithubReviewLedger>,
        delivery_id: &str,
        payload: &[u8],
    ) -> Result<(GithubReviewLedger, IngestOutcome)> {
        ingest_at(ledger, delivery_id, "2026-09-05T02:00:00Z", payload)
    }

    fn ingest_at(
        ledger: Option<GithubReviewLedger>,
        delivery_id: &str,
        received_at: &str,
        payload: &[u8],
    ) -> Result<(GithubReviewLedger, IngestOutcome)> {
        ingest_github_webhook(
            ledger,
            GithubWebhookIngest {
                provider_url: "https://github.com",
                event_name: "pull_request_review",
                delivery_id,
                received_at,
                signature_header: &sign(payload),
                secret: SECRET,
                receiver_key_id: RECEIVER_KEY_ID,
                receiver_signing_key: &RECEIVER_SIGNING_KEY,
                payload,
            },
        )
    }

    fn ingest_synchronize(
        ledger: Option<GithubReviewLedger>,
        delivery_id: &str,
        received_at: &str,
        payload: &[u8],
    ) -> Result<(GithubReviewLedger, IngestOutcome)> {
        ingest_github_webhook(
            ledger,
            GithubWebhookIngest {
                provider_url: "https://github.com",
                event_name: "pull_request",
                delivery_id,
                received_at,
                signature_header: &sign(payload),
                secret: SECRET,
                receiver_key_id: RECEIVER_KEY_ID,
                receiver_signing_key: &RECEIVER_SIGNING_KEY,
                payload,
            },
        )
    }

    #[test]
    fn verifies_githubs_documented_signature_vector() {
        verify_github_webhook_signature(SECRET, OFFICIAL_PAYLOAD, OFFICIAL_SIGNATURE).unwrap();
        assert!(
            verify_github_webhook_signature(SECRET, b"Hello, World?", OFFICIAL_SIGNATURE).is_err()
        );
        assert!(verify_github_webhook_signature(SECRET, OFFICIAL_PAYLOAD, "sha256=00").is_err());
    }

    #[test]
    fn duplicate_delivery_is_idempotent_but_conflicting_delivery_fails() {
        let submitted = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, outcome) = ingest(None, "delivery-1", &submitted).unwrap();
        assert_eq!(outcome, IngestOutcome::Applied);
        let (ledger, outcome) = ingest_at(
            Some(ledger),
            "delivery-1",
            "2026-09-05T03:00:00Z",
            &submitted,
        )
        .unwrap();
        assert_eq!(outcome, IngestOutcome::Duplicate);
        assert_eq!(ledger.review_receipts.len(), 1);
        assert_eq!(ledger.deliveries[0].received_at, "2026-09-05T02:00:00Z");

        let changed = review_payload(
            "submitted",
            "changes_requested",
            42,
            Some("d123456789012345678901234567890123456789"),
        );
        let error = ingest(Some(ledger), "delivery-1", &changed).unwrap_err();
        assert!(error.to_string().contains("reused with different content"));
    }

    #[test]
    fn dismissal_before_submission_is_monotonic_and_a_new_review_reactivates_coverage() {
        let dismissed = review_payload(
            "dismissed",
            "dismissed",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(None, "delivery-dismiss", &dismissed).unwrap();
        assert!(ledger.active_receipts().is_empty());

        let submitted = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-submit", &submitted).unwrap();
        assert!(ledger.active_receipts().is_empty());
        assert_eq!(ledger.review_receipts.len(), 1);
        assert_eq!(ledger.dismissals.len(), 1);

        let later = review_payload(
            "submitted",
            "approved",
            42,
            Some("d123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-later", &later).unwrap();
        let active = ledger.active_receipts();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].review_id, 42);
    }

    #[test]
    fn dismissed_latest_review_never_falls_back_to_an_older_receipt() {
        let older = review_payload(
            "submitted",
            "approved",
            40,
            Some("b123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(None, "delivery-old", &older).unwrap();
        let newer = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-new", &newer).unwrap();
        let dismissed = review_payload(
            "dismissed",
            "dismissed",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-dismiss", &dismissed).unwrap();

        assert!(ledger.active_receipts().is_empty());
    }

    #[test]
    fn dismissal_arriving_before_its_submission_suppresses_an_older_receipt() {
        let older = review_payload(
            "submitted",
            "approved",
            40,
            Some("b123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(None, "delivery-old", &older).unwrap();
        let dismissed = review_payload(
            "dismissed",
            "dismissed",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-dismiss", &dismissed).unwrap();

        assert!(ledger.active_receipts().is_empty());
        assert_eq!(ledger.review_receipts.len(), 1);
        assert_eq!(ledger.dismissals.len(), 1);
    }

    #[test]
    fn nullable_dismissal_metadata_is_an_additive_tombstone() {
        let dismissed = review_payload(
            "dismissed",
            "dismissed",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let mut value: serde_json::Value = serde_json::from_slice(&dismissed).unwrap();
        value["review"]["commit_id"] = serde_json::Value::Null;
        value["review"]["submitted_at"] = serde_json::Value::Null;
        let dismissed = serde_json::to_vec(&value).unwrap();
        let (ledger, _) = ingest(None, "delivery-dismiss", &dismissed).unwrap();
        assert_eq!(ledger.dismissals.len(), 1);
        assert_eq!(ledger.dismissals[0].commit_id, None);
        assert_eq!(ledger.dismissals[0].submitted_at, None);
        assert!(ledger.active_receipts().is_empty());

        let submitted = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(Some(ledger), "delivery-submit", &submitted).unwrap();
        assert_eq!(ledger.review_receipts.len(), 1);
        assert!(ledger.active_receipts().is_empty());

        let encoded = serde_json::to_value(&ledger).unwrap();
        assert!(encoded["dismissals"][0]["commit_id"].is_null());
        assert!(encoded["dismissals"][0]["submitted_at"].is_null());
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../schema/github-review-ledger-v1.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        let errors = validator
            .iter_errors(&encoded)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "schema errors: {errors:#?}");
    }

    #[test]
    fn rejects_unsigned_and_bot_reviews() {
        let payload = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        assert!(
            ingest_github_webhook(
                None,
                GithubWebhookIngest {
                    provider_url: "https://github.com",
                    event_name: "pull_request_review",
                    delivery_id: "delivery-1",
                    received_at: "2026-09-05T02:00:00Z",
                    signature_header: OFFICIAL_SIGNATURE,
                    secret: SECRET,
                    receiver_key_id: RECEIVER_KEY_ID,
                    receiver_signing_key: &RECEIVER_SIGNING_KEY,
                    payload: &payload,
                },
            )
            .is_err()
        );

        let mut value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        value["review"]["user"]["type"] = "Bot".into();
        let bot = serde_json::to_vec(&value).unwrap();
        assert!(ingest(None, "delivery-bot", &bot).is_err());
    }

    #[test]
    fn synchronize_requires_event_after_to_equal_current_head() {
        let payload = synchronize_payload(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "cccccccccccccccccccccccccccccccccccccccc",
        );
        let (ledger, outcome) =
            ingest_synchronize(None, "delivery-sync", "2026-09-05T02:00:00Z", &payload).unwrap();
        assert_eq!(outcome, IngestOutcome::Applied);
        assert_eq!(ledger.head_observations.len(), 1);
        assert_eq!(ledger.head_observations[0].head_commit, "b".repeat(40));
    }

    #[test]
    fn authoritative_head_reduction_ignores_transition_arrival_order() {
        let base = "c".repeat(40);
        let h0 = "0".repeat(40);
        let h1 = "1".repeat(40);
        let h2 = "2".repeat(40);
        let newer = synchronize_payload(&h1, &h2, &base);
        let (ledger, _) =
            ingest_synchronize(None, "delivery-sync-new", "2026-09-05T03:00:00Z", &newer).unwrap();
        let historical_base = "d".repeat(40);
        let older = synchronize_payload(&h0, &h1, &historical_base);
        let (ledger, _) = ingest_synchronize(
            Some(ledger),
            "delivery-sync-old",
            "2026-09-05T03:05:00Z",
            &older,
        )
        .unwrap();

        let effective = ledger.reconcile_current_head(&base, &h2).unwrap();
        assert_eq!(effective.base_commit, base);
        assert_eq!(effective.head_commit, h2);
        assert_eq!(effective.audited_transition_count, 2);
        assert!(
            ledger
                .reconcile_current_head(&effective.base_commit, &h1)
                .is_err()
        );

        let (duplicate, _) = ingest_synchronize(
            Some(ledger.clone()),
            "delivery-sync-new-duplicate",
            "2026-09-05T03:06:00Z",
            &newer,
        )
        .unwrap();
        let effective = duplicate.reconcile_current_head(&base, &h2).unwrap();
        assert_eq!(effective.audited_transition_count, 3);

        let conflicting_successor = synchronize_payload(&h0, &"3".repeat(40), &base);
        let (conflicting_successor, _) = ingest_synchronize(
            Some(ledger.clone()),
            "delivery-sync-conflicting-successor",
            "2026-09-05T03:07:00Z",
            &conflicting_successor,
        )
        .unwrap();
        let error = conflicting_successor
            .reconcile_current_head(&base, &h2)
            .unwrap_err();
        assert!(error.to_string().contains("conflicting successors"));

        let conflicting_predecessor = synchronize_payload(&"3".repeat(40), &h2, &base);
        let (conflicting_predecessor, _) = ingest_synchronize(
            Some(ledger.clone()),
            "delivery-sync-conflicting-predecessor",
            "2026-09-05T03:08:00Z",
            &conflicting_predecessor,
        )
        .unwrap();
        let error = conflicting_predecessor
            .reconcile_current_head(&base, &h2)
            .unwrap_err();
        assert!(error.to_string().contains("conflicting predecessors"));

        let cycle = synchronize_payload(&h2, &h0, &base);
        let (cycle, _) = ingest_synchronize(
            Some(ledger.clone()),
            "delivery-sync-cycle",
            "2026-09-05T03:09:00Z",
            &cycle,
        )
        .unwrap();
        let error = cycle.reconcile_current_head(&base, &h2).unwrap_err();
        assert!(error.to_string().contains("cycle"));

        let disconnected = synchronize_payload(&"3".repeat(40), &"4".repeat(40), &base);
        let (ledger, _) = ingest_synchronize(
            Some(ledger),
            "delivery-sync-disconnected",
            "2026-09-05T03:10:00Z",
            &disconnected,
        )
        .unwrap();
        let error = ledger.reconcile_current_head(&base, &h2).unwrap_err();
        assert!(error.to_string().contains("disconnected"));
    }

    #[test]
    fn authoritative_head_without_transition_history_accepts_full_sha256_ids() {
        let submitted = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (ledger, _) = ingest(None, "delivery-submit", &submitted).unwrap();
        let base = "a".repeat(64);
        let head = "b".repeat(64);
        let effective = ledger.reconcile_current_head(&base, &head).unwrap();
        assert_eq!(effective.base_commit, base);
        assert_eq!(effective.head_commit, head);
        assert_eq!(effective.audited_transition_count, 0);
    }

    #[test]
    fn synchronize_deliveries_and_head_observations_are_one_to_one() {
        let base = "c".repeat(40);
        let payload = synchronize_payload(&"0".repeat(40), &"1".repeat(40), &base);
        let (ledger, _) =
            ingest_synchronize(None, "delivery-sync", "2026-09-05T03:00:00Z", &payload).unwrap();

        let mut missing = ledger.clone();
        missing.head_observations.clear();
        assert!(
            missing
                .validate()
                .unwrap_err()
                .to_string()
                .contains("missing its head observation")
        );

        let mut duplicate = ledger.clone();
        duplicate
            .head_observations
            .push(duplicate.head_observations[0].clone());
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("multiple head observations")
        );

        let mut mismatched = ledger.clone();
        mismatched.head_observations[0].payload_sha256 = "0".repeat(64);
        assert!(
            mismatched
                .validate()
                .unwrap_err()
                .to_string()
                .contains("does not match its synchronize delivery")
        );

        let review = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (mut mixed, _) = ingest(None, "delivery-review", &review).unwrap();
        let review_delivery = mixed.deliveries[0].clone();
        mixed.head_observations = ledger.head_observations;
        mixed.head_observations[0].source_delivery_id = review_delivery.delivery_id;
        mixed.head_observations[0].received_at = review_delivery.received_at;
        mixed.head_observations[0].payload_sha256 = review_delivery.payload_sha256;
        assert!(
            mixed
                .validate()
                .unwrap_err()
                .to_string()
                .contains("non-synchronize delivery")
        );
    }

    #[test]
    fn signed_delivery_rejects_offline_tampering() {
        let submitted = review_payload(
            "submitted",
            "approved",
            41,
            Some("c123456789012345678901234567890123456789"),
        );
        let (mut ledger, _) = ingest(None, "delivery-1", &submitted).unwrap();
        ledger.deliveries[0].payload_sha256 = "0".repeat(64);
        let error = ledger.validate().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("receiver delivery attestation verification failed")
        );
    }
}
