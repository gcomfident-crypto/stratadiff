#[path = "../src/inbox_event.rs"]
mod inbox_event;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use inbox_event::{
    INBOX_EVENT_SCHEMA, InboxEventBinding, InboxEventEnvelope, InboxEventTrigger,
    MAX_INBOX_EVENT_TOKEN_BYTES,
};
use serde_json::Value;

fn oid(character: char) -> String {
    std::iter::repeat_n(character, 40).collect()
}

fn binding() -> InboxEventBinding {
    InboxEventBinding {
        provider_host: "github.com".to_owned(),
        repository: "acme/widget".to_owned(),
        repository_node_id: "R_widget".to_owned(),
        pull_request_number: 17,
        pull_request_node_id: "PR_17".to_owned(),
        reviewer_login: "reviewer".to_owned(),
        reviewer_node_id: "U_reviewer".to_owned(),
        review_database_id: 1001,
        review_state: "approved".to_owned(),
        review_node_id: "PRR_1001".to_owned(),
        checkpoint_oid: oid('a'),
        checkpoint_base_oid: Some(oid('c')),
        current_base_oid: Some(oid('d')),
        head_oid: oid('b'),
        review_request_active: true,
        triggers: vec![
            InboxEventTrigger::HeadChanged,
            InboxEventTrigger::BaseDrift,
            InboxEventTrigger::ReviewReRequested,
        ],
    }
}

fn assert_digest_changes(change: impl FnOnce(&mut InboxEventBinding)) {
    let original = InboxEventEnvelope::new(binding()).unwrap();
    let mut changed = binding();
    change(&mut changed);
    let changed = InboxEventEnvelope::new(changed).unwrap();
    assert_ne!(changed.event_id, original.event_id);
}

#[test]
fn canonical_token_round_trips() {
    let event = InboxEventEnvelope::new(binding()).unwrap();
    assert_eq!(event.schema, INBOX_EVENT_SCHEMA);
    assert_eq!(
        event.event_id,
        "adc6debc053a36ee0fa7f89f0bc964476e82617113f4b5e4aea169e7102f143f"
    );

    let token = event.to_token().unwrap();
    assert!(token.len() <= MAX_INBOX_EVENT_TOKEN_BYTES);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    );
    assert_eq!(InboxEventEnvelope::from_token(&token).unwrap(), event);
    assert_eq!(event.to_token().unwrap(), token);
}

#[test]
fn every_bound_fact_changes_the_event_identity() {
    assert_digest_changes(|value| value.provider_host = "ghe.example".to_owned());
    assert_digest_changes(|value| value.repository = "acme/other".to_owned());
    assert_digest_changes(|value| value.repository_node_id = "R_other".to_owned());
    assert_digest_changes(|value| value.pull_request_number = 18);
    assert_digest_changes(|value| value.pull_request_node_id = "PR_18".to_owned());
    assert_digest_changes(|value| value.reviewer_login = "other-reviewer".to_owned());
    assert_digest_changes(|value| value.reviewer_node_id = "U_other".to_owned());
    assert_digest_changes(|value| value.review_database_id = 1002);
    assert_digest_changes(|value| value.review_state = "changes_requested".to_owned());
    assert_digest_changes(|value| value.review_node_id = "PRR_1002".to_owned());
    assert_digest_changes(|value| value.checkpoint_oid = oid('e'));
    assert_digest_changes(|value| value.checkpoint_base_oid = Some(oid('e')));
    assert_digest_changes(|value| value.current_base_oid = Some(oid('e')));
    assert_digest_changes(|value| value.head_oid = oid('f'));
    assert_digest_changes(|value| {
        value.review_request_active = false;
        value.triggers.pop();
    });
}

#[test]
fn triggers_must_match_the_facts_in_canonical_order() {
    let mut no_trigger = binding();
    no_trigger.head_oid = no_trigger.checkpoint_oid.clone();
    no_trigger.current_base_oid = no_trigger.checkpoint_base_oid.clone();
    no_trigger.review_request_active = false;
    no_trigger.triggers.clear();
    assert!(
        InboxEventEnvelope::new(no_trigger)
            .unwrap_err()
            .to_string()
            .contains("no trigger")
    );

    let mut missing = binding();
    missing.triggers.remove(1);
    assert!(
        InboxEventEnvelope::new(missing)
            .unwrap_err()
            .to_string()
            .contains("canonical order")
    );

    let mut reordered = binding();
    reordered.triggers.swap(0, 1);
    assert!(
        InboxEventEnvelope::new(reordered)
            .unwrap_err()
            .to_string()
            .contains("canonical order")
    );

    let mut unavailable_base = binding();
    unavailable_base.head_oid = unavailable_base.checkpoint_oid.clone();
    unavailable_base.checkpoint_base_oid = None;
    unavailable_base.current_base_oid = None;
    unavailable_base.review_request_active = true;
    unavailable_base.triggers = vec![InboxEventTrigger::ReviewReRequested];
    assert!(
        InboxEventEnvelope::new(unavailable_base)
            .unwrap_err()
            .to_string()
            .contains("checkpoint base OID")
    );
}

#[test]
fn decode_rejects_malformed_noncanonical_and_tampered_tokens() {
    assert!(InboxEventEnvelope::from_token("").is_err());
    assert!(InboxEventEnvelope::from_token("not+a-token").is_err());
    assert!(InboxEventEnvelope::from_token(&"A".repeat(MAX_INBOX_EVENT_TOKEN_BYTES + 1)).is_err());

    let event = InboxEventEnvelope::new(binding()).unwrap();
    let token = event.to_token().unwrap();
    assert!(InboxEventEnvelope::from_token(&format!("{token}=")).is_err());

    let canonical_json = URL_SAFE_NO_PAD.decode(&token).unwrap();
    let mut noncanonical_json = canonical_json.clone();
    noncanonical_json.push(b'\n');
    assert!(InboxEventEnvelope::from_token(&URL_SAFE_NO_PAD.encode(noncanonical_json)).is_err());

    let mut unknown: Value = serde_json::from_slice(&canonical_json).unwrap();
    unknown["unknown"] = Value::Bool(true);
    assert!(
        InboxEventEnvelope::from_token(
            &URL_SAFE_NO_PAD.encode(serde_json::to_vec(&unknown).unwrap())
        )
        .is_err()
    );

    let mut missing: Value = serde_json::from_slice(&canonical_json).unwrap();
    missing.as_object_mut().unwrap().remove("reviewer_node_id");
    assert!(
        InboxEventEnvelope::from_token(
            &URL_SAFE_NO_PAD.encode(serde_json::to_vec(&missing).unwrap())
        )
        .is_err()
    );

    let mut tampered = event.clone();
    tampered.head_oid = oid('f');
    let tampered_token = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&tampered).unwrap());
    assert!(
        InboxEventEnvelope::from_token(&tampered_token)
            .unwrap_err()
            .to_string()
            .contains("does not match")
    );
}

#[test]
fn envelope_rejects_invalid_identity_fields_and_digest() {
    let mut invalid = binding();
    invalid.provider_host = "GitHub.com".to_owned();
    assert!(InboxEventEnvelope::new(invalid).is_err());

    let mut invalid = binding();
    invalid.repository_node_id.clear();
    assert!(InboxEventEnvelope::new(invalid).is_err());

    let mut invalid = binding();
    invalid.review_state = "dismissed".to_owned();
    assert!(InboxEventEnvelope::new(invalid).is_err());

    let mut invalid = binding();
    invalid.checkpoint_oid = "A".repeat(40);
    assert!(InboxEventEnvelope::new(invalid).is_err());

    let mut event = InboxEventEnvelope::new(binding()).unwrap();
    event.schema = "stratadiff-review-inbox-event-v4".to_owned();
    assert!(event.validate().is_err());

    let mut event = InboxEventEnvelope::new(binding()).unwrap();
    event.event_id = "0".repeat(64);
    assert!(event.validate().is_err());
}
