#[allow(dead_code)]
#[path = "../src/doctor.rs"]
mod doctor;
#[allow(dead_code)]
#[path = "../src/doctor_workflow.rs"]
mod doctor_workflow;

use doctor::{
    DoctorActionCode, DoctorCheckRun, DoctorCollection, DoctorCollectionGap,
    DoctorCollectionStatus, DoctorCollectionSurface, DoctorCommitStatus, DoctorEvaluationTarget,
    DoctorEvaluationTargetKind, DoctorEvaluationTargetResolution, DoctorEvidenceKind,
    DoctorPolicyKind, DoctorPolicyRef, DoctorRequirement, DoctorRequirementKey,
    DoctorRequirementStatus, DoctorTarget, DoctorTargetV2, DoctorVerdict, DoctorWorkflowCollection,
    DoctorWorkflowCollectionGap, DoctorWorkflowCollectionStatus, DoctorWorkflowInventory,
    DoctorWorkflowInventoryFile, DoctorWorkflowProbe, DoctorWorkflowProbeKind,
    DoctorWorkflowProducer, DoctorWorkflowTriggerInvestigation,
    PULL_REQUEST_DOCTOR_REPORT_V2_SCHEMA, PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA,
    PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA, PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA,
    PullRequestDoctorSnapshot, PullRequestDoctorSnapshotV2, PullRequestDoctorSnapshotV3,
    evaluate_pull_request_doctor, evaluate_pull_request_doctor_v2, evaluate_pull_request_doctor_v3,
    render_pull_request_doctor_markdown, render_pull_request_doctor_v2_markdown,
};
use doctor_workflow::{
    DoctorWorkflowTriggerInput, PullRequestWorkflowTrigger, WorkflowChangedFiles,
    WorkflowDefinition, WorkflowExpectedApp, WorkflowJob, WorkflowMergeableState,
    WorkflowProviderCapability, WorkflowState, WorkflowSyntax, WorkflowTarget, WorkflowTargetKind,
    WorkflowTriggers,
};

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const EVALUATION_BASE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";
const EVALUATION_SHA: &str = "dddddddddddddddddddddddddddddddddddddddd";
const ACTIONS_APP_ID: u64 = 15_368;
const CONTEXT: &str = "CI / test";

fn ruleset(id: &str) -> DoctorPolicyRef {
    DoctorPolicyRef {
        kind: DoctorPolicyKind::Ruleset,
        id: id.to_owned(),
        name: format!("main ruleset {id}"),
        url: format!("https://github.com/acme/widgets/rules/{id}"),
    }
}

fn branch_protection() -> DoctorPolicyRef {
    DoctorPolicyRef {
        kind: DoctorPolicyKind::BranchProtection,
        id: "main".to_owned(),
        name: "Branch protection for main".to_owned(),
        url: "https://github.com/acme/widgets/settings/branches".to_owned(),
    }
}

fn requirement(expected_app_id: Option<u64>) -> DoctorRequirement {
    DoctorRequirement {
        context: CONTEXT.to_owned(),
        expected_app_id,
        policies: vec![ruleset("71")],
    }
}

fn check(id: u64, app_id: Option<u64>, status: &str, conclusion: Option<&str>) -> DoctorCheckRun {
    DoctorCheckRun {
        id,
        url: format!("https://github.com/acme/widgets/checks/{id}"),
        name: CONTEXT.to_owned(),
        app_id,
        app_slug: app_id.map(|id| {
            if id == ACTIONS_APP_ID {
                "github-actions".to_owned()
            } else {
                "other-app".to_owned()
            }
        }),
        status: status.to_owned(),
        conclusion: conclusion.map(str::to_owned),
    }
}

fn legacy(id: u64, state: &str) -> DoctorCommitStatus {
    DoctorCommitStatus {
        id,
        url: format!("https://api.github.com/repos/acme/widgets/statuses/{HEAD_SHA}"),
        context: CONTEXT.to_owned(),
        creator_id: Some(7),
        creator_login: Some("legacy-ci".to_owned()),
        state: state.to_owned(),
    }
}

fn snapshot(expected_app_id: Option<u64>) -> PullRequestDoctorSnapshot {
    PullRequestDoctorSnapshot {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA.to_owned(),
        captured_at: "2026-09-08T12:00:00Z".to_owned(),
        provider_url: "https://github.com".to_owned(),
        repository: "acme/widgets".to_owned(),
        target: DoctorTarget {
            number: 42,
            url: "https://github.com/acme/widgets/pull/42".to_owned(),
            base_ref: "main".to_owned(),
            base_sha: BASE_SHA.to_owned(),
            head_sha: HEAD_SHA.to_owned(),
        },
        collection: DoctorCollection {
            status: DoctorCollectionStatus::Complete,
            api_calls: 5,
            response_bytes: 12_345,
            gaps: Vec::new(),
        },
        requirements: vec![requirement(expected_app_id)],
        check_runs: vec![check(501, expected_app_id, "completed", Some("success"))],
        statuses: Vec::new(),
    }
}

fn snapshot_v2(kind: DoctorEvaluationTargetKind) -> PullRequestDoctorSnapshotV2 {
    let legacy = snapshot(Some(ACTIONS_APP_ID));
    let (sha, base_sha, queue_entry_id, queue_state) = match kind {
        DoctorEvaluationTargetKind::PrHead => (HEAD_SHA, None, None, None),
        DoctorEvaluationTargetKind::TestMerge => (EVALUATION_SHA, Some(BASE_SHA), None, None),
        DoctorEvaluationTargetKind::MergeGroup => (
            EVALUATION_SHA,
            Some(EVALUATION_BASE_SHA),
            Some("MQE_lQDOA5dJV88AAAABBVoJNs2aL84CwqYU"),
            Some("AWAITING_CHECKS"),
        ),
    };
    PullRequestDoctorSnapshotV2 {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA.to_owned(),
        captured_at: legacy.captured_at,
        provider_url: legacy.provider_url,
        repository: legacy.repository,
        target: DoctorTargetV2 {
            number: legacy.target.number,
            url: legacy.target.url,
            base_ref: legacy.target.base_ref,
            base_sha: legacy.target.base_sha,
            head_sha: legacy.target.head_sha,
            evaluation: DoctorEvaluationTarget {
                kind,
                resolution: DoctorEvaluationTargetResolution::Selected,
                sha: sha.to_owned(),
                base_sha: base_sha.map(str::to_owned),
                queue_entry_id: queue_entry_id.map(str::to_owned),
                queue_state: queue_state.map(str::to_owned),
            },
        },
        signal_sha: sha.to_owned(),
        collection: legacy.collection,
        requirements: legacy.requirements,
        check_runs: legacy.check_runs,
        statuses: legacy.statuses,
    }
}

fn workflow_snapshot_v3() -> PullRequestDoctorSnapshotV3 {
    let mut base = snapshot_v2(DoctorEvaluationTargetKind::MergeGroup);
    base.check_runs.clear();
    base.collection.api_calls = 40;
    base.collection.response_bytes = 40_000;
    let requirement = DoctorRequirementKey {
        context: CONTEXT.to_owned(),
        expected_app_id: Some(ACTIONS_APP_ID),
    };
    let workflow_path = ".github/workflows/ci.yml";
    PullRequestDoctorSnapshotV3 {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA.to_owned(),
        captured_at: base.captured_at,
        provider_url: base.provider_url,
        repository: base.repository,
        target: base.target,
        signal_sha: base.signal_sha,
        collection: base.collection,
        requirements: base.requirements,
        check_runs: base.check_runs,
        statuses: base.statuses,
        workflow_collection: DoctorWorkflowCollection {
            status: DoctorWorkflowCollectionStatus::Complete,
            api_calls: 12,
            response_bytes: 12_000,
            inventory: Some(DoctorWorkflowInventory {
                sha: EVALUATION_SHA.to_owned(),
                files: vec![DoctorWorkflowInventoryFile {
                    blob_sha: "ffffffffffffffffffffffffffffffffffffffff".to_owned(),
                    jobs: vec![WorkflowJob {
                        condition: "always".to_owned(),
                        id: "ci".to_owned(),
                        name: CONTEXT.to_owned(),
                        name_static: true,
                        reusable: false,
                    }],
                    path: workflow_path.to_owned(),
                    triggers: WorkflowTriggers {
                        merge_group: None,
                        pull_request: Some(PullRequestWorkflowTrigger {
                            branches: Vec::new(),
                            branches_ignore: Vec::new(),
                            paths: Vec::new(),
                            paths_ignore: Vec::new(),
                            types: Vec::new(),
                        }),
                    },
                }],
            }),
            probes: vec![DoctorWorkflowProbe {
                kind: DoctorWorkflowProbeKind::PullRequestHead,
                sha: HEAD_SHA.to_owned(),
            }],
            gaps: Vec::new(),
        },
        workflow_trigger_investigations: vec![DoctorWorkflowTriggerInvestigation {
            requirement: requirement.clone(),
            producer: Some(DoctorWorkflowProducer {
                source_sha: HEAD_SHA.to_owned(),
                check_run_id: 501,
                check_run_api_url: "https://api.github.com/repos/acme/widgets/check-runs/501"
                    .to_owned(),
                check_run_url: "https://github.com/acme/widgets/actions/runs/701/job/801"
                    .to_owned(),
                check_name: CONTEXT.to_owned(),
                app_id: ACTIONS_APP_ID,
                app_slug: "github-actions".to_owned(),
                check_suite_id: 601,
                workflow_run_id: 701,
                workflow_run_attempt: 1,
                workflow_run_url: "https://github.com/acme/widgets/actions/runs/701".to_owned(),
                workflow_run_path: format!("{workflow_path}@refs/pull/42/merge"),
                workflow_job_id: 801,
                workflow_job_url: "https://github.com/acme/widgets/actions/runs/701/job/801"
                    .to_owned(),
                workflow_job_name: CONTEXT.to_owned(),
                workflow_job_check_run_url:
                    "https://api.github.com/repos/acme/widgets/check-runs/501".to_owned(),
                workflow_id: 901,
                workflow_path: workflow_path.to_owned(),
                workflow_url: "https://github.com/acme/widgets/actions/workflows/ci.yml".to_owned(),
            }),
            input: Some(DoctorWorkflowTriggerInput {
                changed_files: WorkflowChangedFiles {
                    complete: false,
                    github_filter_file_limit_reached: false,
                    paths: Vec::new(),
                    total: 0,
                },
                collection_gaps: Vec::new(),
                expected_app: WorkflowExpectedApp::GithubActions,
                historical_check_names: vec![CONTEXT.to_owned()],
                last_activity: "not_applicable".to_owned(),
                provider_capability: WorkflowProviderCapability::NotApplicable,
                pull_request: doctor_workflow::WorkflowPullRequest {
                    base_ref: "main".to_owned(),
                    base_sha: BASE_SHA.to_owned(),
                    head_ref: "feature".to_owned(),
                    head_repository_is_fork: false,
                    head_sha: HEAD_SHA.to_owned(),
                    mergeable_state: WorkflowMergeableState::Unknown,
                    number: 42,
                },
                required_context: CONTEXT.to_owned(),
                runs: Vec::new(),
                target: WorkflowTarget {
                    kind: WorkflowTargetKind::MergeGroup,
                    sha: EVALUATION_SHA.to_owned(),
                },
                workflows: vec![WorkflowDefinition {
                    jobs: vec![WorkflowJob {
                        condition: "always".to_owned(),
                        id: "ci".to_owned(),
                        name: CONTEXT.to_owned(),
                        name_static: true,
                        reusable: false,
                    }],
                    path: workflow_path.to_owned(),
                    state: WorkflowState::Active,
                    syntax: WorkflowSyntax::Valid,
                    triggers: WorkflowTriggers {
                        merge_group: None,
                        pull_request: Some(PullRequestWorkflowTrigger {
                            branches: Vec::new(),
                            branches_ignore: Vec::new(),
                            paths: Vec::new(),
                            paths_ignore: Vec::new(),
                            types: Vec::new(),
                        }),
                    },
                }],
            }),
        }],
    }
}

fn only_status(snapshot: &PullRequestDoctorSnapshot) -> DoctorRequirementStatus {
    evaluate_pull_request_doctor(snapshot).unwrap().requirements[0].status
}

#[test]
fn clean_exact_head_is_clear_and_matches_schema() {
    let report = evaluate_pull_request_doctor(&snapshot(Some(ACTIONS_APP_ID))).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksClear);
    assert_eq!(report.summary.satisfied, 1);
    assert!(report.next_actions.is_empty());
    assert_eq!(report.requirements[0].evidence[0].sha, HEAD_SHA);
    assert_eq!(
        report.requirements[0].evidence[0].kind,
        DoctorEvidenceKind::CheckRun
    );

    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/pull-request-doctor-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let instance = serde_json::to_value(report).unwrap();
    if let Err(error) = validator.validate(&instance) {
        panic!("pull-request doctor report did not match its schema: {error}");
    }
}

#[test]
fn v1_wire_shape_remains_unchanged() {
    let snapshot = snapshot(Some(ACTIONS_APP_ID));
    let snapshot_instance = serde_json::to_value(&snapshot).unwrap();
    let report = evaluate_pull_request_doctor(&snapshot).unwrap();
    let instance = serde_json::to_value(report).unwrap();

    assert!(snapshot_instance.get("signal_sha").is_none());
    assert_eq!(instance["schema"], "stratadiff-pull-request-doctor-v1");
    assert!(instance.get("signal_sha").is_none());
    assert!(instance["target"].get("evaluation").is_none());
}

#[test]
fn v2_binds_evidence_and_actions_to_each_evaluation_target() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/pull-request-doctor-v2.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();

    for kind in [
        DoctorEvaluationTargetKind::PrHead,
        DoctorEvaluationTargetKind::TestMerge,
        DoctorEvaluationTargetKind::MergeGroup,
    ] {
        let mut input = snapshot_v2(kind);
        input.check_runs[0].conclusion = Some("failure".to_owned());
        let expected_sha = input.target.evaluation.sha.clone();
        let report = evaluate_pull_request_doctor_v2(&input).unwrap();

        assert_eq!(report.schema, PULL_REQUEST_DOCTOR_REPORT_V2_SCHEMA);
        assert_eq!(report.requirements[0].evidence[0].sha, expected_sha);
        assert!(report.next_actions[0].argv[4].contains(&expected_sha));
        let mut instance = serde_json::to_value(report).unwrap();
        if let Err(error) = validator.validate(&instance) {
            panic!("pull-request doctor v2 report did not match its schema: {error}");
        }
        let mut missing_resolution = instance.clone();
        missing_resolution["target"]["evaluation"]
            .as_object_mut()
            .unwrap()
            .remove("resolution");
        assert!(validator.validate(&missing_resolution).is_err());
        instance["target"]
            .as_object_mut()
            .unwrap()
            .remove("evaluation");
        assert!(validator.validate(&instance).is_err());
    }
}

#[test]
fn provisional_v2_targets_fail_closed_for_every_kind() {
    for kind in [
        DoctorEvaluationTargetKind::PrHead,
        DoctorEvaluationTargetKind::TestMerge,
        DoctorEvaluationTargetKind::MergeGroup,
    ] {
        let mut input = snapshot_v2(kind);
        input.target.evaluation.resolution = DoctorEvaluationTargetResolution::Provisional;
        let report = evaluate_pull_request_doctor_v2(&input).unwrap();

        assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
        assert!(!report.claim_boundary.required_check_readiness_supported);
        assert_eq!(report.next_actions.len(), 1);
        assert_eq!(
            report.next_actions[0].code,
            DoctorActionCode::CompleteCollection
        );
        assert_eq!(report.next_actions[0].requirement, None);
        assert_eq!(
            report.requirements[0].evidence[0].sha,
            input.target.evaluation.sha
        );

        let markdown = render_pull_request_doctor_v2_markdown(&report);
        assert!(markdown.contains("Resolution: <code>provisional</code>"));
        assert!(markdown.contains("has not proved that GitHub selected it"));
        assert!(!markdown.contains("exact head"));
    }
}

#[test]
fn selected_v2_target_requires_complete_collection_for_readiness_support() {
    let mut input = snapshot_v2(DoctorEvaluationTargetKind::PrHead);
    input.collection = DoctorCollection {
        status: DoctorCollectionStatus::Partial,
        api_calls: 4,
        response_bytes: 9_000,
        gaps: vec![DoctorCollectionGap {
            surface: DoctorCollectionSurface::Requirements,
            reason: "one ruleset could not be read".to_owned(),
        }],
    };

    let report = evaluate_pull_request_doctor_v2(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert!(!report.claim_boundary.required_check_readiness_supported);
}

#[test]
fn v2_schema_rejects_unsafe_resolution_and_collection_claims() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/pull-request-doctor-v2.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let report =
        evaluate_pull_request_doctor_v2(&snapshot_v2(DoctorEvaluationTargetKind::PrHead)).unwrap();
    let clear = serde_json::to_value(report).unwrap();
    assert!(validator.validate(&clear).is_ok());

    let mut provisional_clear = clear.clone();
    provisional_clear["target"]["evaluation"]["resolution"] = serde_json::json!("provisional");
    provisional_clear["claim_boundary"]["required_check_readiness_supported"] =
        serde_json::json!(false);
    assert!(validator.validate(&provisional_clear).is_err());

    let mut provisional_supported = clear.clone();
    provisional_supported["target"]["evaluation"]["resolution"] = serde_json::json!("provisional");
    provisional_supported["verdict"] = serde_json::json!("inconclusive");
    assert!(validator.validate(&provisional_supported).is_err());

    let gap = serde_json::json!({
        "surface": "requirements",
        "reason": "one policy surface was unavailable"
    });
    let mut partial_clear = clear.clone();
    partial_clear["collection"]["status"] = serde_json::json!("partial");
    partial_clear["collection"]["gaps"] = serde_json::json!([gap.clone()]);
    partial_clear["claim_boundary"]["required_check_readiness_supported"] =
        serde_json::json!(false);
    assert!(validator.validate(&partial_clear).is_err());

    let mut partial_supported = clear;
    partial_supported["collection"]["status"] = serde_json::json!("partial");
    partial_supported["collection"]["gaps"] = serde_json::json!([gap]);
    partial_supported["verdict"] = serde_json::json!("inconclusive");
    assert!(validator.validate(&partial_supported).is_err());
}

#[test]
fn v2_markdown_names_the_evaluation_target_and_claim_boundary() {
    for kind in [
        DoctorEvaluationTargetKind::PrHead,
        DoctorEvaluationTargetKind::TestMerge,
        DoctorEvaluationTargetKind::MergeGroup,
    ] {
        let report = evaluate_pull_request_doctor_v2(&snapshot_v2(kind)).unwrap();
        let markdown = render_pull_request_doctor_v2_markdown(&report);

        assert!(markdown.contains(&format!(
            "Evaluation target: <code>{}</code> at <code>{}</code>",
            match kind {
                DoctorEvaluationTargetKind::PrHead => "pr&#95;head",
                DoctorEvaluationTargetKind::TestMerge => "test&#95;merge",
                DoctorEvaluationTargetKind::MergeGroup => "merge&#95;group",
            },
            report.target.evaluation.sha,
        )));
        assert!(markdown.contains("Resolution: <code>selected</code>"));
        assert!(markdown.contains(&format!("PR head: <code>{HEAD_SHA}</code>")));
        assert!(markdown.contains(&format!(
            "PR base: <code>main</code> at <code>{BASE_SHA}</code>"
        )));
        assert!(markdown.contains("## Claim boundary"));
        assert!(markdown.contains("Required-check readiness: supported"));
        assert!(markdown.contains("Mergeability: not supported"));
        assert!(markdown.contains("Review requirements: not supported"));
        assert!(markdown.contains("Compliance: not supported"));
        assert!(markdown.contains("Code safety: not supported"));
        assert!(!markdown.contains("Exact head"));
        assert!(!markdown.contains("exact head"));

        if kind == DoctorEvaluationTargetKind::MergeGroup {
            assert!(markdown.contains("Merge-queue entry:"));
            assert!(markdown.contains("AWAITING&#95;CHECKS"));
        }
    }
}

#[test]
fn v2_rejects_invalid_evaluation_metadata_combinations() {
    let mut invalid = Vec::new();

    let mut pr_head_with_base = snapshot_v2(DoctorEvaluationTargetKind::PrHead);
    pr_head_with_base.target.evaluation.base_sha = Some(BASE_SHA.to_owned());
    invalid.push(pr_head_with_base);

    let mut pr_head_with_other_sha = snapshot_v2(DoctorEvaluationTargetKind::PrHead);
    pr_head_with_other_sha.target.evaluation.sha = EVALUATION_SHA.to_owned();
    invalid.push(pr_head_with_other_sha);

    let mut test_merge_without_base = snapshot_v2(DoctorEvaluationTargetKind::TestMerge);
    test_merge_without_base.target.evaluation.base_sha = None;
    invalid.push(test_merge_without_base);

    let mut test_merge_with_wrong_base = snapshot_v2(DoctorEvaluationTargetKind::TestMerge);
    test_merge_with_wrong_base.target.evaluation.base_sha = Some(EVALUATION_BASE_SHA.to_owned());
    invalid.push(test_merge_with_wrong_base);

    let mut test_merge_with_queue = snapshot_v2(DoctorEvaluationTargetKind::TestMerge);
    test_merge_with_queue.target.evaluation.queue_entry_id = Some("MQE_1".to_owned());
    invalid.push(test_merge_with_queue);

    let mut merge_group_without_entry = snapshot_v2(DoctorEvaluationTargetKind::MergeGroup);
    merge_group_without_entry.target.evaluation.queue_entry_id = None;
    invalid.push(merge_group_without_entry);

    let mut merge_group_without_state = snapshot_v2(DoctorEvaluationTargetKind::MergeGroup);
    merge_group_without_state.target.evaluation.queue_state = None;
    invalid.push(merge_group_without_state);

    let mut merge_group_without_base = snapshot_v2(DoctorEvaluationTargetKind::MergeGroup);
    merge_group_without_base.target.evaluation.base_sha = None;
    invalid.push(merge_group_without_base);

    let mut merge_group_on_head = snapshot_v2(DoctorEvaluationTargetKind::MergeGroup);
    merge_group_on_head.target.evaluation.sha = HEAD_SHA.to_owned();
    invalid.push(merge_group_on_head);

    for input in invalid {
        assert!(evaluate_pull_request_doctor_v2(&input).is_err());
    }
}

#[test]
fn v2_rejects_a_signal_sha_from_a_different_endpoint() {
    let mut input = snapshot_v2(DoctorEvaluationTargetKind::TestMerge);
    input.signal_sha = HEAD_SHA.to_owned();

    let error = evaluate_pull_request_doctor_v2(&input).unwrap_err();

    assert_eq!(
        error.to_string(),
        "signal SHA must match the evaluation SHA"
    );
}

#[test]
fn complete_collection_calls_an_absent_requirement_missing() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.check_runs.clear();

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Missing
    );
    assert!(report.requirements[0].evidence.is_empty());
    assert_eq!(report.next_actions.len(), 1);
    assert_eq!(
        report.next_actions[0].argv,
        [
            "gh",
            "api",
            "--hostname",
            "github.com",
            "repos/acme/widgets/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/check-runs?filter=latest&per_page=100"
        ]
    );
}

#[test]
fn a_same_name_check_from_the_wrong_app_is_a_source_mismatch() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.check_runs = vec![check(502, Some(9_999), "completed", Some("success"))];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::SourceMismatch
    );
    assert_eq!(report.requirements[0].evidence[0].app_id, Some(9_999));
}

#[test]
fn correct_app_wins_over_a_wrong_app_with_the_same_name() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input
        .check_runs
        .push(check(502, Some(9_999), "completed", Some("failure")));

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksClear);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Satisfied
    );
    assert_eq!(report.requirements[0].evidence.len(), 2);
}

#[test]
fn a_pending_expected_app_check_blocks_the_pull_request() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.check_runs = vec![check(501, Some(ACTIONS_APP_ID), "in_progress", None)];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Pending
    );
}

#[test]
fn a_failed_expected_app_check_blocks_the_pull_request() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.check_runs = vec![check(
        501,
        Some(ACTIONS_APP_ID),
        "completed",
        Some("failure"),
    )];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Failed
    );
}

#[test]
fn legacy_statuses_are_handled_conservatively() {
    let mut unpinned = snapshot(None);
    unpinned.check_runs.clear();
    unpinned.statuses = vec![legacy(601, "success")];
    assert_eq!(only_status(&unpinned), DoctorRequirementStatus::Satisfied);

    let mut pinned = snapshot(Some(ACTIONS_APP_ID));
    pinned.check_runs.clear();
    pinned.statuses = vec![legacy(602, "success")];
    let report = evaluate_pull_request_doctor(&pinned).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::SourceUnknown
    );

    let mut split_brain = snapshot(None);
    split_brain.statuses = vec![legacy(603, "failure")];
    let report = evaluate_pull_request_doctor(&split_brain).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Failed
    );
    assert_eq!(report.requirements[0].evidence.len(), 2);
}

#[test]
fn a_legacy_failure_still_blocks_a_successful_app_bound_check() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.statuses = vec![legacy(604, "failure")];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Failed
    );
    assert_eq!(report.requirements[0].evidence.len(), 2);
}

#[test]
fn legacy_success_cannot_prove_an_app_bound_requirement() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.statuses = vec![legacy(606, "success")];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::SourceUnknown
    );
}

#[test]
fn partial_absence_is_unknown_and_can_never_clear_or_block() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.collection = DoctorCollection {
        status: DoctorCollectionStatus::Partial,
        api_calls: 3,
        response_bytes: 8_000,
        gaps: vec![DoctorCollectionGap {
            surface: DoctorCollectionSurface::CheckRuns,
            reason: "check-run pagination stopped after an API error".to_owned(),
        }],
    };
    input.check_runs.clear();

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::SourceUnknown
    );
    assert_eq!(report.summary.missing, 0);
    assert_eq!(report.next_actions.len(), 2);
    assert_eq!(
        report.next_actions[1].argv,
        [
            "stratadiff",
            "doctor",
            "https://github.com/acme/widgets/pull/42",
            "--format",
            "json",
        ]
    );

    let mut observed_success = snapshot(Some(ACTIONS_APP_ID));
    observed_success.collection = DoctorCollection {
        status: DoctorCollectionStatus::Partial,
        api_calls: 4,
        response_bytes: 9_000,
        gaps: vec![DoctorCollectionGap {
            surface: DoctorCollectionSurface::Requirements,
            reason: "one ruleset could not be read".to_owned(),
        }],
    };
    let report = evaluate_pull_request_doctor(&observed_success).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
}

#[test]
fn an_unresolved_check_target_overrides_a_known_head_blocker() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.collection = DoctorCollection {
        status: DoctorCollectionStatus::Partial,
        api_calls: 8,
        response_bytes: 12_000,
        gaps: vec![DoctorCollectionGap {
            surface: DoctorCollectionSurface::Target,
            reason: "the test-merge commit has status signals".to_owned(),
        }],
    };
    input.check_runs = vec![check(
        605,
        Some(ACTIONS_APP_ID),
        "completed",
        Some("failure"),
    )];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert!(!report.claim_boundary.required_check_readiness_supported);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.next_actions.len(), 1);
    assert_eq!(report.next_actions[0].requirement, None);
}

#[test]
fn a_requirements_only_gap_does_not_hide_known_signal_failures() {
    let mut missing = snapshot(Some(ACTIONS_APP_ID));
    missing.collection = DoctorCollection {
        status: DoctorCollectionStatus::Partial,
        api_calls: 4,
        response_bytes: 9_000,
        gaps: vec![DoctorCollectionGap {
            surface: DoctorCollectionSurface::Requirements,
            reason: "one unrelated ruleset could not be read".to_owned(),
        }],
    };
    missing.check_runs.clear();

    let report = evaluate_pull_request_doctor(&missing).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Missing
    );
}

#[test]
fn multiple_known_apps_for_an_unbound_context_are_ambiguous() {
    let mut input = snapshot(None);
    input.check_runs = vec![
        check(501, Some(ACTIONS_APP_ID), "completed", Some("success")),
        check(502, Some(9_999), "completed", Some("success")),
    ];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::SourceUnknown
    );
    assert!(report.requirements[0].explanation.contains("ambiguous"));
}

#[test]
fn duplicate_requirement_keys_union_and_dedupe_policy_provenance() {
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.requirements = vec![
        DoctorRequirement {
            context: CONTEXT.to_owned(),
            expected_app_id: Some(ACTIONS_APP_ID),
            policies: vec![ruleset("71"), ruleset("71")],
        },
        DoctorRequirement {
            context: CONTEXT.to_owned(),
            expected_app_id: Some(ACTIONS_APP_ID),
            policies: vec![branch_protection(), ruleset("71")],
        },
    ];

    let report = evaluate_pull_request_doctor(&input).unwrap();

    assert_eq!(report.requirements.len(), 1);
    assert_eq!(report.requirements[0].policies.len(), 2);
    assert_eq!(report.summary.requirements, 1);
}

#[test]
fn markdown_escapes_untrusted_policy_and_context_text() {
    let hostile = "CI | </code> **pwn** [link](evil) # heading";
    let mut input = snapshot(Some(ACTIONS_APP_ID));
    input.requirements[0].context = hostile.to_owned();
    input.requirements[0].policies[0].name = "bad | </a> **policy**".to_owned();
    input.check_runs[0].name = hostile.to_owned();

    let report = evaluate_pull_request_doctor(&input).unwrap();
    let markdown = render_pull_request_doctor_markdown(&report);

    assert!(markdown.contains("CI &#124; &lt;/code&gt; &#42;&#42;pwn&#42;&#42;"));
    assert!(markdown.contains("bad &#124; &lt;/a&gt; &#42;&#42;policy&#42;&#42;"));
    assert!(!markdown.contains("[link](evil)"));
    assert!(!markdown.contains("# heading"));
}

#[test]
fn serde_rejects_unknown_fields_and_actions_are_argv_arrays() {
    let input = snapshot(Some(ACTIONS_APP_ID));
    let mut encoded = serde_json::to_value(&input).unwrap();
    encoded["surprise"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PullRequestDoctorSnapshot>(encoded).is_err());

    let mut missing = input;
    missing.check_runs.clear();
    let report = evaluate_pull_request_doctor(&missing).unwrap();
    let encoded = serde_json::to_value(report).unwrap();
    assert!(encoded["next_actions"][0]["argv"].is_array());
    assert!(encoded["next_actions"][0].get("command").is_none());
}

#[test]
fn provider_url_must_be_a_pure_https_origin() {
    for invalid in [
        "https://github.com/",
        "https://github.com/enterprise",
        "https://token@github.com",
        "https://github.com?tenant=acme",
    ] {
        let mut input = snapshot(Some(ACTIONS_APP_ID));
        input.provider_url = invalid.to_owned();
        assert!(evaluate_pull_request_doctor(&input).is_err(), "{invalid}");
    }
}

#[test]
fn v3_reports_a_missing_merge_group_trigger_with_unique_exact_sha_producer() {
    let report = evaluate_pull_request_doctor_v3(&workflow_snapshot_v3()).unwrap();

    assert_eq!(
        report.workflow_trigger_diagnoses[0]
            .diagnosis
            .as_ref()
            .unwrap()
            .cause_code,
        doctor_workflow::WorkflowTriggerCause::MergeGroupTriggerMissing
    );
}

#[test]
fn v3_requires_an_exact_sha_inventory_for_complete_collection() {
    let mut snapshot = workflow_snapshot_v3();
    snapshot.workflow_collection.inventory = None;

    let error = evaluate_pull_request_doctor_v3(&snapshot).unwrap_err();
    assert!(error.to_string().contains("complete workflow collection"));
}

#[test]
fn v3_rejects_a_duplicate_exact_sha_producer() {
    let mut snapshot = workflow_snapshot_v3();
    let mut duplicate = snapshot
        .workflow_collection
        .inventory
        .as_ref()
        .unwrap()
        .files[0]
        .clone();
    duplicate.path = ".github/workflows/duplicate.yml".to_owned();
    duplicate.blob_sha = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned();
    snapshot
        .workflow_collection
        .inventory
        .as_mut()
        .unwrap()
        .files
        .push(duplicate);

    let error = evaluate_pull_request_doctor_v3(&snapshot).unwrap_err();
    assert!(error.to_string().contains("not unique"));
}

#[test]
fn v3_rejects_a_certain_diagnosis_when_the_same_requirement_has_a_gap() {
    let mut snapshot = workflow_snapshot_v3();
    snapshot.workflow_collection.status = DoctorWorkflowCollectionStatus::Partial;
    snapshot.workflow_collection.gaps = vec![DoctorWorkflowCollectionGap {
        requirement: snapshot.workflow_trigger_investigations[0]
            .requirement
            .clone(),
        code: "target_workflow_runs_incomplete".to_owned(),
        reason: "target run pagination was incomplete".to_owned(),
    }];

    let error = evaluate_pull_request_doctor_v3(&snapshot).unwrap_err();
    assert!(error.to_string().contains("blocking evidence gap"));
}

#[test]
fn v3_rejects_a_producer_outside_declared_pr_candidates() {
    let mut snapshot = workflow_snapshot_v3();
    snapshot.workflow_trigger_investigations[0]
        .producer
        .as_mut()
        .unwrap()
        .source_sha = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned();

    let error = evaluate_pull_request_doctor_v3(&snapshot).unwrap_err();
    assert!(error.to_string().contains("declared probe SHA"));
}
