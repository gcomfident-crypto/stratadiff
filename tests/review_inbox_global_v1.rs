use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Command,
};

use serde::Deserialize;
use serde_json::Value;
use stratadiff::inbox_decision::{
    InboxCandidateStatus, decide_inbox_observation_json, decode_inbox_observation,
    evaluate_inbox_observation_for_resume,
};

#[derive(Debug, Deserialize)]
struct MaterializedBundle {
    cases: Vec<MaterializedCase>,
}

#[derive(Debug, Deserialize)]
struct MaterializedCase {
    id: String,
    observation: Value,
    transition_relation: Option<TransitionRelation>,
}

#[derive(Debug, Deserialize)]
struct TransitionRelation {
    kind: String,
    reference: String,
}

#[derive(Debug, Deserialize)]
struct OracleBundle {
    cases: BTreeMap<String, Value>,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn rust_target_policy_matches_all_frozen_global_inbox_cases() {
    let root = repository_root();
    let materialized = Command::new("python3")
        .args([
            "-B",
            "benchmarks/review-inbox-global-v1/verify.py",
            "materialize",
        ])
        .current_dir(&root)
        .output()
        .expect("failed to run the frozen Review Inbox materializer");
    assert!(
        materialized.status.success(),
        "materializer failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&materialized.stdout),
        String::from_utf8_lossy(&materialized.stderr),
    );
    let bundle: MaterializedBundle =
        serde_json::from_slice(&materialized.stdout).expect("materializer returned invalid JSON");
    let oracle: OracleBundle = serde_json::from_slice(
        &std::fs::read(root.join("benchmarks/review-inbox-global-v1/oracle.json"))
            .expect("failed to read the frozen Review Inbox oracle"),
    )
    .expect("frozen Review Inbox oracle is invalid");

    assert_eq!(bundle.cases.len(), 60, "the frozen corpus changed size");
    let materialized_ids = bundle
        .cases
        .iter()
        .map(|case| case.id.clone())
        .collect::<BTreeSet<_>>();
    let oracle_ids = oracle.cases.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(materialized_ids, oracle_ids, "case membership differs");

    let mut transition_keys = BTreeMap::new();
    for case in &bundle.cases {
        let decision = decide_inbox_observation_json(&case.observation);
        let actual = serde_json::to_value(&decision).expect("decision is not serializable");
        assert_eq!(
            actual, oracle.cases[&case.id],
            "Rust Inbox decision differs for {}",
            case.id,
        );
        if let Some(key) = decision.transition_key() {
            transition_keys.insert(case.id.clone(), key.to_owned());
        }
    }

    for case in &bundle.cases {
        let Some(relation) = &case.transition_relation else {
            continue;
        };
        let current = transition_keys
            .get(&case.id)
            .unwrap_or_else(|| panic!("{} has no actionable transition key", case.id));
        let reference = transition_keys.get(&relation.reference).unwrap_or_else(|| {
            panic!(
                "{} relation target has no transition key",
                relation.reference
            )
        });
        let equal = current == reference;
        assert_eq!(
            equal,
            relation.kind == "same",
            "{} transition relation {} failed against {}",
            case.id,
            relation.kind,
            relation.reference,
        );
    }
}

#[test]
fn executable_resume_policy_requires_current_base_for_a_changed_head() {
    let root = repository_root();
    let materialized = Command::new("python3")
        .args([
            "-B",
            "benchmarks/review-inbox-global-v1/verify.py",
            "materialize",
        ])
        .current_dir(&root)
        .output()
        .expect("failed to run the frozen Review Inbox materializer");
    assert!(materialized.status.success());
    let mut bundle: MaterializedBundle =
        serde_json::from_slice(&materialized.stdout).expect("materializer returned invalid JSON");
    let case = bundle
        .cases
        .iter_mut()
        .find(|case| case.id == "missing-checkpoint-base-with-head-change")
        .expect("frozen corpus omitted the changed-head/no-checkpoint-base case");
    case.observation["search"]["candidates"][0]["pull_request"]["current_base_oid"] = Value::Null;
    let observation = decode_inbox_observation(case.observation.clone()).unwrap();
    let evaluation = evaluate_inbox_observation_for_resume(&observation);

    assert_eq!(
        evaluation.decision.status.as_deref(),
        Some("insufficient_evidence")
    );
    assert_eq!(evaluation.decision.counts.unobservable, 1);
    assert_eq!(evaluation.candidates.len(), 1);
    assert_eq!(
        evaluation.candidates[0].status,
        InboxCandidateStatus::Unobservable
    );
    assert_eq!(
        evaluation.candidates[0].reason.as_deref(),
        Some("current_base_oid_unavailable")
    );
}
