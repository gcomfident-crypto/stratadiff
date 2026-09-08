use std::collections::BTreeMap;

use serde::Deserialize;
use stratadiff::doctor_candidate::{CandidateSelection, CandidateSelectionInput, select_candidate};

#[derive(Debug, Deserialize)]
struct CasesBundle {
    cases: Vec<CandidateCase>,
}

#[derive(Debug, Deserialize)]
struct CandidateCase {
    id: String,
    observation: CandidateSelectionInput,
}

#[derive(Debug, Deserialize)]
struct OracleBundle {
    outcomes: Vec<OracleOutcome>,
}

#[derive(Debug, Deserialize)]
struct OracleOutcome {
    case_id: String,
    #[serde(flatten)]
    selection: CandidateSelection,
}

fn cases() -> CasesBundle {
    serde_json::from_str(include_str!(
        "../benchmarks/pull-request-candidate-v2/cases.json"
    ))
    .unwrap()
}

fn v1_cases() -> CasesBundle {
    serde_json::from_str(include_str!(
        "../benchmarks/pull-request-candidate-v1/cases.json"
    ))
    .unwrap()
}

fn oracle() -> OracleBundle {
    serde_json::from_str(include_str!(
        "../benchmarks/pull-request-candidate-v2/oracle.json"
    ))
    .unwrap()
}

fn v1_oracle() -> OracleBundle {
    serde_json::from_str(include_str!(
        "../benchmarks/pull-request-candidate-v1/oracle.json"
    ))
    .unwrap()
}

#[test]
fn candidate_selector_matches_every_offline_oracle_case() {
    let cases = cases();
    let mut expected: BTreeMap<_, _> = oracle()
        .outcomes
        .into_iter()
        .map(|outcome| (outcome.case_id, outcome.selection))
        .collect();

    assert_eq!(cases.cases.len(), expected.len());
    for case in cases.cases {
        let expected_selection = expected.remove(&case.id).unwrap();
        let actual = select_candidate(&case.observation).unwrap();
        assert_eq!(actual, expected_selection, "case {}", case.id);
    }
    assert!(expected.is_empty());
}

#[test]
fn v2_supersession_changes_only_the_empty_test_merge_outcome() {
    let v1_observations = v1_cases()
        .cases
        .into_iter()
        .map(|case| (case.id, case.observation))
        .collect::<BTreeMap<_, _>>();
    let v2_observations = cases()
        .cases
        .into_iter()
        .map(|case| (case.id, case.observation))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(v1_observations, v2_observations);

    let mut v1_outcomes = v1_oracle()
        .outcomes
        .into_iter()
        .map(|outcome| (outcome.case_id, outcome.selection))
        .collect::<BTreeMap<_, _>>();
    let mut v2_outcomes = oracle()
        .outcomes
        .into_iter()
        .map(|outcome| (outcome.case_id, outcome.selection))
        .collect::<BTreeMap<_, _>>();
    let changed_case = "github-docs-empty-test-merge";
    let previous = v1_outcomes.remove(changed_case).unwrap();
    let current = v2_outcomes.remove(changed_case).unwrap();
    assert_ne!(previous, current);
    assert_eq!(v1_outcomes, v2_outcomes);
}

#[test]
fn selector_source_has_no_case_specific_dispatch_or_oracle_dependency() {
    let source = include_str!("../src/doctor_candidate.rs");
    assert!(!source.contains("pull-request-candidate-v1"));
    assert!(!source.contains("pull-request-candidate-v2"));
    assert!(!source.contains("oracle.json"));
    for case in cases().cases {
        assert!(
            !source.contains(&case.id),
            "selector names case {}",
            case.id
        );
    }
}

#[test]
fn malformed_signal_observations_are_rejected() {
    let mut input = cases().cases.remove(0).observation;
    input.signals.push(input.signals[0].clone());
    assert!(select_candidate(&input).is_err());

    let mut input = cases().cases.remove(9).observation;
    input.signals[0].sha = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC".to_owned();
    assert!(select_candidate(&input).is_err());
}
