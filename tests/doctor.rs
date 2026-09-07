#[path = "../src/doctor.rs"]
mod doctor;

use doctor::{
    DoctorCheckRun, DoctorCollection, DoctorCollectionGap, DoctorCollectionStatus,
    DoctorCollectionSurface, DoctorCommitStatus, DoctorEvidenceKind, DoctorPolicyKind,
    DoctorPolicyRef, DoctorRequirement, DoctorRequirementStatus, DoctorTarget, DoctorVerdict,
    PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA, PullRequestDoctorSnapshot, evaluate_pull_request_doctor,
    render_pull_request_doctor_markdown,
};

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
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
