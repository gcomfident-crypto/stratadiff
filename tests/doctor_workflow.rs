use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use stratadiff::doctor_workflow::{
    DoctorWorkflowTriggerDiagnosis, DoctorWorkflowTriggerInput, WorkflowRunEvent,
    WorkflowRunObservation, WorkflowRunStatus, WorkflowTriggerCause, WorkflowTriggerConfidence,
    classify_workflow_trigger,
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

fn benchmark_input(id: &str) -> DoctorWorkflowTriggerInput {
    let (cases, _) = load_assets();
    cases
        .cases
        .into_iter()
        .find(|case| case.id == id)
        .unwrap_or_else(|| panic!("missing benchmark case {id}"))
        .input
}

fn add_queued_pull_request_run(input: &mut DoctorWorkflowTriggerInput) {
    input.runs.push(WorkflowRunObservation {
        conclusion: None,
        event: WorkflowRunEvent::PullRequest,
        head_sha: input.target.sha.clone(),
        status: WorkflowRunStatus::Queued,
        workflow_path: input.workflows[0].path.clone(),
    });
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

#[test]
fn diagnoses_missing_pull_request_trigger() {
    let mut input = benchmark_input("branch-filter-excluded");
    input.workflows[0].triggers.pull_request = None;

    let diagnosis = classify_workflow_trigger(&input).expect("classify missing trigger");
    assert_eq!(
        diagnosis.cause_code,
        WorkflowTriggerCause::PullRequestTriggerMissing
    );
    assert_eq!(diagnosis.confidence, WorkflowTriggerConfidence::Certain);
    assert_eq!(
        diagnosis.evidence,
        [
            "target:pull_request_head",
            "workflow:.github/workflows/ci.yml",
            "trigger:pull_request_absent",
        ]
    );
    assert_eq!(diagnosis.fix.action_code, "add_pull_request_trigger");
    assert!(diagnosis.fix.requires_human_edit);
}

#[test]
fn incomplete_changed_files_cannot_prove_path_exclusion() {
    let mut input = benchmark_input("path-filter-excluded");
    input.changed_files.complete = false;

    let diagnosis = classify_workflow_trigger(&input).expect("classify incomplete changed files");
    assert_eq!(
        diagnosis.cause_code,
        WorkflowTriggerCause::WorkflowTriggerUnknown
    );
    assert_eq!(diagnosis.confidence, WorkflowTriggerConfidence::Uncertain);
    assert_eq!(diagnosis.evidence, ["gap:changed_files_incomplete"]);
    assert_eq!(diagnosis.fix.action_code, "collect_changed_files");
    assert!(!diagnosis.fix.requires_human_edit);
}

#[test]
fn exact_pull_request_run_overrides_static_trigger_exclusions() {
    for id in [
        "branch-filter-excluded",
        "path-filter-excluded",
        "activity-type-excludes-synchronize",
    ] {
        let mut input = benchmark_input(id);
        add_queued_pull_request_run(&mut input);

        let diagnosis = classify_workflow_trigger(&input)
            .unwrap_or_else(|error| panic!("{id} classification failed: {error:#}"));
        assert_eq!(
            diagnosis.cause_code,
            WorkflowTriggerCause::None,
            "{id} ignored its observed run"
        );
        assert_eq!(diagnosis.fix.action_code, "none");
    }
}

#[test]
fn observed_run_overrides_missing_static_pull_request_trigger() {
    let mut input = benchmark_input("activity-type-excludes-synchronize");
    input.workflows[0].triggers.pull_request = None;
    add_queued_pull_request_run(&mut input);

    let diagnosis = classify_workflow_trigger(&input).expect("classify observed run");
    assert_eq!(diagnosis.cause_code, WorkflowTriggerCause::None);
    assert_eq!(
        diagnosis.evidence,
        [
            "workflow:.github/workflows/ci.yml",
            "event:pull_request",
            "run_head:exact",
        ]
    );
    assert_eq!(diagnosis.fix.action_code, "none");
}

#[test]
fn unavailable_activity_only_blocks_explicit_type_filters() {
    let mut default_types = benchmark_input("path-filter-excluded");
    default_types.last_activity = "not_applicable".to_owned();
    let diagnosis =
        classify_workflow_trigger(&default_types).expect("classify default activity types");
    assert_eq!(
        diagnosis.cause_code,
        WorkflowTriggerCause::WorkflowPathFilterExcluded
    );

    let mut explicit_types = benchmark_input("activity-type-excludes-synchronize");
    explicit_types.last_activity = "not_applicable".to_owned();
    let diagnosis =
        classify_workflow_trigger(&explicit_types).expect("classify explicit activity types");
    assert_eq!(
        diagnosis.cause_code,
        WorkflowTriggerCause::WorkflowTriggerUnknown
    );
    assert_eq!(diagnosis.confidence, WorkflowTriggerConfidence::Uncertain);
    assert_eq!(diagnosis.evidence, ["activity_filter:activity_unavailable"]);
    assert_eq!(diagnosis.fix.action_code, "collect_pull_request_activity");
}
