use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use stratadiff::doctor::{
    DoctorCollectionGap, DoctorCollectionStatus, DoctorRequirementStatus, DoctorVerdict,
    PullRequestDoctorReport, PullRequestDoctorSnapshot, evaluate_pull_request_doctor,
};

const CASES_SCHEMA: &str = "stratadiff-pull-request-doctor-cases-v1";
const ORACLE_SCHEMA: &str = "stratadiff-pull-request-doctor-oracle-v1";
const DATASET_VERSION: &str = "1.0.0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CasesBundle {
    schema: String,
    dataset_version: String,
    snapshot_defaults: Value,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    #[serde(rename = "description")]
    _description: String,
    #[serde(rename = "covers")]
    _covers: Vec<String>,
    snapshot: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OracleBundle {
    schema: String,
    dataset_version: String,
    cases: BTreeMap<String, NormalizedOutcome>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NormalizedOutcome {
    verdict: DoctorVerdict,
    collection_status: DoctorCollectionStatus,
    gaps: Vec<DoctorCollectionGap>,
    requirements: Vec<NormalizedRequirement>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NormalizedRequirement {
    context: String,
    expected_app_id: Option<u64>,
    status: DoctorRequirementStatus,
    evidence: Vec<String>,
    policies: Vec<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(relative_path: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to decode {}: {error}", path.display()))
}

fn object(value: &Value, label: &str) -> Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{label} must be a JSON object"))
        .clone()
}

fn materialize_snapshot(defaults: &Value, case: &Case) -> PullRequestDoctorSnapshot {
    let mut snapshot = object(defaults, "snapshot_defaults");
    for (key, value) in object(&case.snapshot, &format!("case {} snapshot", case.id)) {
        assert!(
            snapshot.insert(key.clone(), value).is_none(),
            "case {} unexpectedly overrides snapshot default {key}",
            case.id,
        );
    }
    serde_json::from_value(Value::Object(snapshot)).unwrap_or_else(|error| {
        panic!(
            "case {} did not materialize as PullRequestDoctorSnapshot: {error}",
            case.id
        )
    })
}

fn wire_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("doctor enum must serialize")
        .as_str()
        .expect("doctor enum must serialize as a string")
        .to_owned()
}

fn normalize(report: &PullRequestDoctorReport) -> NormalizedOutcome {
    NormalizedOutcome {
        verdict: report.verdict,
        collection_status: report.collection.status,
        gaps: report.collection.gaps.clone(),
        requirements: report
            .requirements
            .iter()
            .map(|diagnosis| NormalizedRequirement {
                context: diagnosis.key.context.clone(),
                expected_app_id: diagnosis.key.expected_app_id,
                status: diagnosis.status,
                evidence: diagnosis
                    .evidence
                    .iter()
                    .map(|evidence| format!("{}:{}", wire_name(&evidence.kind), evidence.id))
                    .collect(),
                policies: diagnosis
                    .policies
                    .iter()
                    .map(|policy| format!("{}:{}", wire_name(&policy.kind), policy.id))
                    .collect(),
            })
            .collect(),
    }
}

#[test]
fn rust_doctor_matches_every_pull_request_doctor_v1_oracle() {
    let cases: CasesBundle = read_json("benchmarks/pull-request-doctor-v1/cases.json");
    let oracle: OracleBundle = read_json("benchmarks/pull-request-doctor-v1/oracle.json");
    assert_eq!(cases.schema, CASES_SCHEMA);
    assert_eq!(oracle.schema, ORACLE_SCHEMA);
    assert_eq!(cases.dataset_version, DATASET_VERSION);
    assert_eq!(oracle.dataset_version, DATASET_VERSION);

    let case_ids = cases
        .cases
        .iter()
        .map(|case| case.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(case_ids.len(), cases.cases.len(), "case IDs are not unique");
    assert_eq!(
        case_ids,
        oracle.cases.keys().cloned().collect(),
        "case and oracle membership differs",
    );

    let schema: Value =
        serde_json::from_str(include_str!("../schema/pull-request-doctor-v1.schema.json"))
            .expect("published pull-request doctor schema is invalid JSON");
    let validator = jsonschema::draft202012::new(&schema)
        .expect("published pull-request doctor schema is invalid");

    let mut actual_cases = BTreeMap::new();
    for case in &cases.cases {
        let snapshot = materialize_snapshot(&cases.snapshot_defaults, case);
        let report = evaluate_pull_request_doctor(&snapshot)
            .unwrap_or_else(|error| panic!("doctor rejected case {}: {error:#}", case.id));
        let instance = serde_json::to_value(&report)
            .unwrap_or_else(|error| panic!("case {} report is not serializable: {error}", case.id));
        if let Err(error) = validator.validate(&instance) {
            panic!(
                "case {} report failed the published schema: {error}",
                case.id
            );
        }

        let actual = normalize(&report);
        let expected = &oracle.cases[&case.id];
        assert_eq!(
            &actual, expected,
            "Rust doctor output differs from the frozen oracle for {}",
            case.id,
        );
        assert!(actual_cases.insert(case.id.clone(), actual).is_none());
    }

    assert_eq!(actual_cases, oracle.cases);
}
