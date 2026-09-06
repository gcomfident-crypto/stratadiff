use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const INBOX_EVENT_SCHEMA: &str = "stratadiff-review-inbox-event-v3";
pub const MAX_INBOX_EVENT_TOKEN_BYTES: usize = 4096;

const EVENT_ID_DOMAIN: &[u8] = b"stratadiff-review-inbox-event-v3";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxEventTrigger {
    HeadChanged,
    BaseDrift,
    ReviewReRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxEventBinding {
    pub provider_host: String,
    pub repository: String,
    pub repository_node_id: String,
    pub pull_request_number: u64,
    pub pull_request_node_id: String,
    pub reviewer_login: String,
    pub reviewer_node_id: String,
    pub review_database_id: u64,
    pub review_state: String,
    pub review_node_id: String,
    pub checkpoint_oid: String,
    pub checkpoint_base_oid: Option<String>,
    pub current_base_oid: Option<String>,
    pub head_oid: String,
    pub review_request_active: bool,
    pub triggers: Vec<InboxEventTrigger>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxEventEnvelope {
    pub schema: String,
    pub event_id: String,
    pub provider_host: String,
    pub repository: String,
    pub repository_node_id: String,
    pub pull_request_number: u64,
    pub pull_request_node_id: String,
    pub reviewer_login: String,
    pub reviewer_node_id: String,
    pub review_database_id: u64,
    pub review_state: String,
    pub review_node_id: String,
    pub checkpoint_oid: String,
    pub checkpoint_base_oid: Option<String>,
    pub current_base_oid: Option<String>,
    pub head_oid: String,
    pub review_request_active: bool,
    pub triggers: Vec<InboxEventTrigger>,
}

#[derive(Serialize)]
struct EventIdInput<'a> {
    schema: &'a str,
    provider_host: &'a str,
    repository: &'a str,
    repository_node_id: &'a str,
    pull_request_number: u64,
    pull_request_node_id: &'a str,
    reviewer_login: &'a str,
    reviewer_node_id: &'a str,
    review_database_id: u64,
    review_state: &'a str,
    review_node_id: &'a str,
    checkpoint_oid: &'a str,
    checkpoint_base_oid: Option<&'a str>,
    current_base_oid: Option<&'a str>,
    head_oid: &'a str,
    review_request_active: bool,
    triggers: &'a [InboxEventTrigger],
}

impl InboxEventEnvelope {
    pub fn new(binding: InboxEventBinding) -> Result<Self> {
        let mut envelope = Self {
            schema: INBOX_EVENT_SCHEMA.to_owned(),
            event_id: String::new(),
            provider_host: binding.provider_host,
            repository: binding.repository,
            repository_node_id: binding.repository_node_id,
            pull_request_number: binding.pull_request_number,
            pull_request_node_id: binding.pull_request_node_id,
            reviewer_login: binding.reviewer_login,
            reviewer_node_id: binding.reviewer_node_id,
            review_database_id: binding.review_database_id,
            review_state: binding.review_state,
            review_node_id: binding.review_node_id,
            checkpoint_oid: binding.checkpoint_oid,
            checkpoint_base_oid: binding.checkpoint_base_oid,
            current_base_oid: binding.current_base_oid,
            head_oid: binding.head_oid,
            review_request_active: binding.review_request_active,
            triggers: binding.triggers,
        };
        envelope.validate_claims()?;
        envelope.event_id = envelope.compute_event_id()?;
        Ok(envelope)
    }

    pub fn to_token(&self) -> Result<String> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).context("failed to encode Inbox event envelope")?;
        let token = URL_SAFE_NO_PAD.encode(encoded);
        ensure!(
            token.len() <= MAX_INBOX_EVENT_TOKEN_BYTES,
            "Inbox event token exceeds {MAX_INBOX_EVENT_TOKEN_BYTES} bytes"
        );
        Ok(token)
    }

    pub fn from_token(token: &str) -> Result<Self> {
        ensure!(!token.is_empty(), "Inbox event token is empty");
        ensure!(
            token.len() <= MAX_INBOX_EVENT_TOKEN_BYTES,
            "Inbox event token exceeds {MAX_INBOX_EVENT_TOKEN_BYTES} bytes"
        );
        let encoded = URL_SAFE_NO_PAD
            .decode(token)
            .context("Inbox event token is not canonical base64url without padding")?;
        ensure!(
            URL_SAFE_NO_PAD.encode(&encoded) == token,
            "Inbox event token is not canonical base64url without padding"
        );
        let envelope: Self = serde_json::from_slice(&encoded)
            .context("Inbox event token is not a valid envelope")?;
        ensure!(
            serde_json::to_vec(&envelope)? == encoded,
            "Inbox event token JSON is not canonical"
        );
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_claims()?;
        ensure!(
            valid_sha256(&self.event_id),
            "Inbox event ID must be 64 lowercase hexadecimal characters"
        );
        ensure!(
            self.event_id == self.compute_event_id()?,
            "Inbox event ID does not match its immutable binding"
        );
        Ok(())
    }

    pub fn compute_event_id(&self) -> Result<String> {
        self.validate_claims()?;
        let input = EventIdInput {
            schema: &self.schema,
            provider_host: &self.provider_host,
            repository: &self.repository,
            repository_node_id: &self.repository_node_id,
            pull_request_number: self.pull_request_number,
            pull_request_node_id: &self.pull_request_node_id,
            reviewer_login: &self.reviewer_login,
            reviewer_node_id: &self.reviewer_node_id,
            review_database_id: self.review_database_id,
            review_state: &self.review_state,
            review_node_id: &self.review_node_id,
            checkpoint_oid: &self.checkpoint_oid,
            checkpoint_base_oid: self.checkpoint_base_oid.as_deref(),
            current_base_oid: self.current_base_oid.as_deref(),
            head_oid: &self.head_oid,
            review_request_active: self.review_request_active,
            triggers: &self.triggers,
        };
        let canonical =
            serde_json::to_vec(&input).context("failed to encode Inbox event identity")?;
        let mut digest = Sha256::new();
        digest.update(EVENT_ID_DOMAIN);
        digest.update([0]);
        digest.update(canonical);
        Ok(format!("{:x}", digest.finalize()))
    }

    fn validate_claims(&self) -> Result<()> {
        ensure!(
            self.schema == INBOX_EVENT_SCHEMA,
            "Inbox event schema must be {INBOX_EVENT_SCHEMA}"
        );
        validate_host(&self.provider_host)?;
        validate_repository(&self.repository)?;
        validate_node_id(&self.repository_node_id, "repository node ID")?;
        ensure!(
            self.pull_request_number > 0,
            "pull request number must be positive"
        );
        validate_node_id(&self.pull_request_node_id, "pull request node ID")?;
        validate_login(&self.reviewer_login)?;
        validate_node_id(&self.reviewer_node_id, "reviewer node ID")?;
        ensure!(
            self.review_database_id > 0,
            "review database ID must be positive"
        );
        ensure!(
            matches!(self.review_state.as_str(), "approved" | "changes_requested"),
            "review state must be approved or changes_requested"
        );
        validate_node_id(&self.review_node_id, "review node ID")?;
        validate_oid(&self.checkpoint_oid, "checkpoint OID")?;
        if let Some(checkpoint_base_oid) = &self.checkpoint_base_oid {
            validate_oid(checkpoint_base_oid, "checkpoint base OID")?;
        }
        if let Some(current_base_oid) = &self.current_base_oid {
            validate_oid(current_base_oid, "current base OID")?;
        }
        validate_oid(&self.head_oid, "head OID")?;

        let head_changed = self.checkpoint_oid != self.head_oid;
        if !head_changed {
            ensure!(
                self.checkpoint_base_oid.is_some(),
                "an unchanged head requires a checkpoint base OID"
            );
            ensure!(
                self.current_base_oid.is_some(),
                "an unchanged head requires a current base OID"
            );
        }
        let base_drift = self
            .checkpoint_base_oid
            .as_ref()
            .zip(self.current_base_oid.as_ref())
            .is_some_and(|(checkpoint, current)| checkpoint != current);
        let mut expected = Vec::with_capacity(3);
        if head_changed {
            expected.push(InboxEventTrigger::HeadChanged);
        }
        if base_drift {
            expected.push(InboxEventTrigger::BaseDrift);
        }
        if self.review_request_active {
            expected.push(InboxEventTrigger::ReviewReRequested);
        }
        ensure!(
            !expected.is_empty(),
            "Inbox event is not actionable because it has no trigger"
        );
        ensure!(
            self.triggers == expected,
            "Inbox event triggers do not match the bound facts or canonical order"
        );
        Ok(())
    }
}

fn validate_host(host: &str) -> Result<()> {
    let bytes = host.as_bytes();
    ensure!(
        (1..=253).contains(&bytes.len())
            && bytes[0].is_ascii_alphanumeric()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            && host == host.to_ascii_lowercase(),
        "provider host is not canonical"
    );
    Ok(())
}

fn validate_repository(repository: &str) -> Result<()> {
    let Some((owner, name)) = repository.split_once('/') else {
        anyhow::bail!("repository must be OWNER/REPO");
    };
    ensure!(
        !owner.is_empty()
            && !name.is_empty()
            && !name.contains('/')
            && repository.len() <= 202
            && owner.bytes().all(valid_repository_byte)
            && name.bytes().all(valid_repository_byte),
        "repository must be a canonical OWNER/REPO name"
    );
    Ok(())
}

fn valid_repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
}

fn validate_login(login: &str) -> Result<()> {
    let bytes = login.as_bytes();
    ensure!(
        (1..=255).contains(&bytes.len())
            && bytes[0].is_ascii_alphanumeric()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')),
        "reviewer login is invalid"
    );
    Ok(())
}

fn validate_node_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        (1..=256).contains(&value.len())
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b':' | b'+' | b'/' | b'=' | b'-')
            }),
        "{label} is invalid"
    );
    Ok(())
}

fn validate_oid(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "{label} must be a full lowercase SHA-1"
    );
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
