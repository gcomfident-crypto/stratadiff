use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use stratadiff::doctor_workflow::{
    DoctorWorkflowTriggerDiagnosis, DoctorWorkflowTriggerInput, WorkflowTriggerCause,
    WorkflowTriggerConfidence, classify_workflow_trigger,
};

const CASES_SCHEMA: &str = "stratadiff-doctor-workflow-trigger-cases-v1";
const ORACLE_SCHEMA: &str = "stratadiff-doctor-workflow-trigger-oracle-v1";
const DATASET_VERSION: &str = "1.0.0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CasesAsset {
    cases: Vec<BenchmarkCase>,
    dataset_version: String,
    schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkCase {
    description: String,
    id: String,
    input: DoctorWorkflowTriggerInput,
    provenance: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OracleAsset {
    cases: BTreeMap<String, DoctorWorkflowTriggerDiagnosis>,
    dataset_version: String,
    schema: String,
}

fn benchmark_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benchmarks/doctor-workflow-trigger-v1")
}

fn load_assets() -> (CasesAsset, OracleAsset) {
    let root = benchmark_root();
    let cases: CasesAsset = serde_json::from_slice(
        &std::fs::read(root.join("cases.json")).expect("read workflow-trigger cases"),
    )
    .expect("decode workflow-trigger cases");
    let oracle: OracleAsset = serde_json::from_slice(
        &std::fs::read(root.join("oracle.json")).expect("read workflow-trigger oracle"),
    )
    .expect("decode workflow-trigger oracle");
    (cases, oracle)
}

#[test]
fn classifier_matches_all_frozen_workflow_trigger_cases() {
    let (cases, oracle) = load_assets();
    assert_eq!(cases.schema, CASES_SCHEMA);
    assert_eq!(oracle.schema, ORACLE_SCHEMA);
    assert_eq!(cases.dataset_version, DATASET_VERSION);
    assert_eq!(oracle.dataset_version, DATASET_VERSION);
    assert!(cases.cases.len() >= 18);
    assert_eq!(cases.cases.len(), oracle.cases.len());

    for case in cases.cases {
        assert!(
            !case.description.is_empty(),
            "{} has no description",
            case.id
        );
        assert!(!case.provenance.is_empty(), "{} has no provenance", case.id);
        let expected = oracle
            .cases
            .get(&case.id)
            .unwrap_or_else(|| panic!("{} has no oracle diagnosis", case.id));
        let actual = classify_workflow_trigger(&case.input)
            .unwrap_or_else(|error| panic!("{} classification failed: {error:#}", case.id));
        assert_eq!(&actual, expected, "{} diagnosis differs", case.id);
    }
}

#[test]
fn benchmark_verifier_remains_independent_and_offline() {
    let root = benchmark_root();
    for command in ["verify", "self-test"] {
        let output = Command::new("python3")
            .arg("-B")
            .arg(root.join("verify.py"))
            .arg(command)
            .output()
            .expect("run independent workflow-trigger verifier");
        assert!(
            output.status.success(),
            "verifier {command} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn uncertain_cases_do_not_become_high_confidence_causes() {
    let (cases, _) = load_assets();
    for case in cases.cases {
        let diagnosis = classify_workflow_trigger(&case.input).expect("classify benchmark case");
        if diagnosis.confidence == WorkflowTriggerConfidence::Uncertain {
            assert!(matches!(
                diagnosis.cause_code,
                WorkflowTriggerCause::ForkApprovalPossible
                    | WorkflowTriggerCause::ProviderRuntimeDeliveryGap
                    | WorkflowTriggerCause::WorkflowTriggerUnknown
            ));
        }
    }
}

#[test]
fn rejects_run_bound_to_a_different_sha() {
    let (mut cases, _) = load_assets();
    let case = cases
        .cases
        .iter_mut()
        .find(|case| case.id == "merge-group-supported-control")
        .expect("merge-group control");
    case.input.runs[0].head_sha = "ffffffffffffffffffffffffffffffffffffffff".to_owned();
    let error =
        classify_workflow_trigger(&case.input).expect_err("drifted run must fail validation");
    assert!(error.to_string().contains("exact target"));
}
