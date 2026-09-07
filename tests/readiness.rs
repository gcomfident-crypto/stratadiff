use stratadiff::readiness::{
    AuditVerdict, BranchPolicySnapshot, CheckRunSnapshot, CollectionGap, CollectionStatus,
    CollectionSurface, FindingRule, GITHUB_ACTIONS_APP_ID, MergeReadinessSnapshot,
    PullRequestSnapshot, PullRequestState, RepositorySnapshot, RequiredCheckSnapshot, RulesetRef,
    SnapshotCollection, TriggerSnapshot, UnknownCode, WorkflowDefinition, WorkflowRunSnapshot,
    WorkflowSnapshot, evaluate_merge_readiness, render_merge_readiness_markdown,
};

const MAIN_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CONTEXT: &str = "CI / test";

fn ruleset() -> RulesetRef {
    RulesetRef {
        id: 71,
        name: "main protection".to_owned(),
        url: "https://github.com/acme/widgets/rules/71".to_owned(),
        source_type: "Repository".to_owned(),
        source: "acme/widgets".to_owned(),
    }
}

fn trigger() -> TriggerSnapshot {
    TriggerSnapshot {
        paths: Vec::new(),
        paths_ignore: Vec::new(),
    }
}

fn healthy_snapshot() -> MergeReadinessSnapshot {
    MergeReadinessSnapshot {
        schema: "stratadiff-merge-readiness-snapshot-v1".to_owned(),
        captured_at: "2026-09-08T12:00:00Z".to_owned(),
        repository: RepositorySnapshot {
            database_id: 99,
            name_with_owner: "acme/widgets".to_owned(),
            url: "https://github.com/acme/widgets".to_owned(),
            default_branch: "main".to_owned(),
            default_branch_head_sha: MAIN_SHA.to_owned(),
        },
        collection: SnapshotCollection {
            status: CollectionStatus::Complete,
            api_calls: 12,
            response_bytes: 8_192,
            gaps: Vec::new(),
        },
        branch_policy: BranchPolicySnapshot {
            branch_protected: true,
            effective_rule_count: 3,
            pull_request_rule: true,
            merge_queue_rule: true,
            required_checks: vec![RequiredCheckSnapshot {
                context: CONTEXT.to_owned(),
                integration_id: Some(GITHUB_ACTIONS_APP_ID),
                rulesets: vec![ruleset()],
            }],
        },
        workflows: vec![WorkflowSnapshot {
            id: 80,
            name: "CI".to_owned(),
            path: ".github/workflows/ci.yml".to_owned(),
            state: "active".to_owned(),
            url: "https://github.com/acme/widgets/actions/workflows/ci.yml".to_owned(),
            definition: Some(WorkflowDefinition {
                pull_request: Some(trigger()),
                pull_request_target: None,
                merge_group: Some(trigger()),
            }),
        }],
        pull_requests: vec![PullRequestSnapshot {
            number: 42,
            url: "https://github.com/acme/widgets/pull/42".to_owned(),
            state: PullRequestState::Merged,
            head_sha: HEAD_SHA.to_owned(),
            collection: CollectionStatus::Complete,
            check_runs: vec![CheckRunSnapshot {
                id: 501,
                url: "https://github.com/acme/widgets/actions/runs/601/job/501".to_owned(),
                name: CONTEXT.to_owned(),
                app_id: Some(GITHUB_ACTIONS_APP_ID),
                app_slug: Some("github-actions".to_owned()),
                check_suite_id: Some(701),
                status: "completed".to_owned(),
                conclusion: Some("success".to_owned()),
            }],
            statuses: Vec::new(),
            workflow_runs: vec![WorkflowRunSnapshot {
                id: 601,
                url: "https://github.com/acme/widgets/actions/runs/601".to_owned(),
                workflow_id: 80,
                path: ".github/workflows/ci.yml".to_owned(),
                event: "pull_request".to_owned(),
                check_suite_id: 701,
                run_attempt: 1,
            }],
        }],
    }
}

#[test]
fn healthy_snapshot_has_no_observed_risk_and_matches_schema() {
    let report = evaluate_merge_readiness(&healthy_snapshot()).unwrap();

    assert_eq!(report.collection.status, CollectionStatus::Complete);
    assert_eq!(report.summary.verdict, AuditVerdict::NoObservedRisk);
    assert!(report.findings.is_empty());
    assert!(report.unknowns.is_empty());
    assert!(report.privacy.workflow_definitions_collected);
    assert!(report.privacy.pull_request_text_collected);
    assert!(report.privacy.check_run_output_text_collected);
    assert!(report.privacy.commit_status_description_collected);
    assert!(report.privacy.commit_messages_collected);
    assert!(!report.privacy.repository_source_collected);
    assert!(!report.privacy.review_text_collected);

    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schema/merge-readiness-audit-v1.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let instance = serde_json::to_value(report).unwrap();
    if let Err(error) = validator.validate(&instance) {
        panic!("merge-readiness report did not match its schema: {error}");
    }
}

#[test]
fn risky_snapshot_names_source_trigger_and_path_failures() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy.required_checks[0].integration_id = None;
    let definition = snapshot.workflows[0].definition.as_mut().unwrap();
    definition.merge_group = None;
    definition.pull_request.as_mut().unwrap().paths = vec!["src/**".to_owned()];
    snapshot.pull_requests[0].check_runs.push(CheckRunSnapshot {
        id: 502,
        url: "https://github.com/acme/widgets/checks/502".to_owned(),
        name: CONTEXT.to_owned(),
        app_id: Some(9_999),
        app_slug: Some("other-app".to_owned()),
        check_suite_id: Some(702),
        status: "completed".to_owned(),
        conclusion: Some("success".to_owned()),
    });

    let report = evaluate_merge_readiness(&snapshot).unwrap();
    let rules = report
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();

    assert_eq!(report.collection.status, CollectionStatus::Complete);
    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(
        rules,
        vec![
            FindingRule::RequiredCheckSourceUnpinned,
            FindingRule::MergeGroupTriggerMissing,
            FindingRule::RequiredCheckSourceAmbiguous,
            FindingRule::PathFilteredRequiredWorkflow,
        ]
    );
    assert!(report.unknowns.is_empty());
    let markdown = render_merge_readiness_markdown(&report);
    assert!(markdown.contains("merge_group_trigger_missing"));
    assert!(markdown.contains(".github/workflows/ci.yml"));
    assert!(markdown.contains("pin each required context"));
}

#[test]
fn missing_required_context_on_one_complete_head_is_inconclusive() {
    let mut snapshot = healthy_snapshot();
    let mut missing = snapshot.pull_requests[0].clone();
    missing.number = 10;
    missing.url = "https://github.com/acme/widgets/pull/10".to_owned();
    missing.head_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned();
    missing.check_runs.clear();
    missing.statuses.clear();
    missing.workflow_runs.clear();
    snapshot.pull_requests.push(missing);

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
    assert!(report.unknowns.iter().any(|unknown| {
        unknown.code == UnknownCode::RequiredCheckMissingOnSampledHead
            && unknown.context.as_deref() == Some(CONTEXT)
    }));
}

#[test]
fn partial_snapshot_is_inconclusive_and_does_not_invent_unprotected_branch() {
    let mut snapshot = healthy_snapshot();
    snapshot.collection = SnapshotCollection {
        status: CollectionStatus::Partial,
        api_calls: 5,
        response_bytes: 2_048,
        gaps: vec![
            CollectionGap {
                surface: CollectionSurface::Repository,
                reason: "branch metadata was unavailable".to_owned(),
            },
            CollectionGap {
                surface: CollectionSurface::EffectiveRules,
                reason: "effective rules were unavailable".to_owned(),
            },
            CollectionGap {
                surface: CollectionSurface::WorkflowDefinitions,
                reason: "workflow content was unavailable".to_owned(),
            },
        ],
    };
    snapshot.branch_policy = BranchPolicySnapshot {
        branch_protected: false,
        effective_rule_count: 0,
        pull_request_rule: false,
        merge_queue_rule: false,
        required_checks: Vec::new(),
    };
    snapshot.workflows[0].definition = None;
    snapshot.pull_requests[0].collection = CollectionStatus::Partial;
    snapshot.pull_requests[0].check_runs.clear();
    snapshot.pull_requests[0].workflow_runs.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.collection.status, CollectionStatus::Partial);
    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
    assert!(
        report
            .unknowns
            .iter()
            .any(|unknown| unknown.code == UnknownCode::CollectionIncomplete)
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.rule != FindingRule::DefaultBranchUnprotected)
    );
}

#[test]
fn complete_unprotected_snapshot_is_actionable() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy = BranchPolicySnapshot {
        branch_protected: false,
        effective_rule_count: 0,
        pull_request_rule: false,
        merge_queue_rule: false,
        required_checks: Vec::new(),
    };
    snapshot.workflows.clear();
    snapshot.pull_requests.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.findings[0].rule,
        FindingRule::DefaultBranchUnprotected
    );
}

#[test]
fn unrelated_effective_rules_do_not_hide_a_missing_merge_gate() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy = BranchPolicySnapshot {
        branch_protected: false,
        effective_rule_count: 2,
        pull_request_rule: false,
        merge_queue_rule: false,
        required_checks: Vec::new(),
    };
    snapshot.workflows.clear();
    snapshot.pull_requests.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.findings[0].rule,
        FindingRule::DefaultBranchUnprotected
    );
    assert!(
        report.findings[0]
            .explanation
            .contains("Other effective rules")
    );
}

#[test]
fn complete_but_unmapped_required_check_is_inconclusive() {
    let mut snapshot = healthy_snapshot();
    snapshot.pull_requests[0].check_runs.clear();
    snapshot.pull_requests[0].workflow_runs.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.collection.status, CollectionStatus::Complete);
    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
    assert_eq!(report.unknowns.len(), 1);
    assert_eq!(
        report.unknowns[0].code,
        UnknownCode::RequiredCheckNeverObserved
    );
}

#[test]
fn pinned_required_check_from_the_wrong_app_is_actionable() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy.merge_queue_rule = false;
    snapshot.branch_policy.required_checks[0].integration_id = Some(8_888);
    snapshot.pull_requests[0].check_runs[0].app_id = Some(9_999);
    snapshot.pull_requests[0].check_runs[0].app_slug = Some("wrong-app".to_owned());
    snapshot.pull_requests[0].workflow_runs.clear();
    snapshot.workflows.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.findings[0].rule,
        FindingRule::RequiredCheckSourceMismatch
    );
    assert!(report.unknowns.is_empty());
}

#[test]
fn missing_publisher_identity_is_unknown_instead_of_a_mismatch() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy.merge_queue_rule = false;
    snapshot.branch_policy.required_checks[0].integration_id = Some(8_888);
    snapshot.pull_requests[0].check_runs[0].app_id = None;
    snapshot.pull_requests[0].check_runs[0].app_slug = None;
    snapshot.pull_requests[0].workflow_runs.clear();
    snapshot.workflows.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
    assert_eq!(report.unknowns.len(), 1);
    assert_eq!(
        report.unknowns[0].code,
        UnknownCode::RequiredCheckSourceUnresolved
    );
}

#[test]
fn unobserved_external_required_check_is_inconclusive() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy.merge_queue_rule = false;
    snapshot.branch_policy.required_checks[0].integration_id = Some(8_888);
    snapshot.pull_requests[0].check_runs.clear();
    snapshot.pull_requests[0].workflow_runs.clear();
    snapshot.workflows.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
    assert_eq!(report.unknowns.len(), 1);
    assert_eq!(
        report.unknowns[0].code,
        UnknownCode::RequiredCheckNeverObserved
    );
}

#[test]
fn partial_pull_request_cannot_prove_a_source_mismatch() {
    let mut snapshot = healthy_snapshot();
    snapshot.collection = SnapshotCollection {
        status: CollectionStatus::Partial,
        api_calls: 12,
        response_bytes: 8_192,
        gaps: vec![CollectionGap {
            surface: CollectionSurface::CheckRuns,
            reason: "one pull request check-run page was unavailable".to_owned(),
        }],
    };
    snapshot.branch_policy.merge_queue_rule = false;
    snapshot.branch_policy.required_checks[0].integration_id = Some(8_888);
    snapshot.pull_requests[0].collection = CollectionStatus::Partial;
    snapshot.pull_requests[0].check_runs[0].app_id = Some(9_999);
    snapshot.pull_requests[0].check_runs[0].app_slug = Some("wrong-app".to_owned());
    snapshot.pull_requests[0].workflow_runs.clear();
    snapshot.workflows.clear();

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.rule != FindingRule::RequiredCheckSourceMismatch)
    );
    assert!(
        report
            .unknowns
            .iter()
            .any(|unknown| unknown.code == UnknownCode::RequiredCheckSourceUnresolved)
    );
}

#[test]
fn invalid_capture_timestamp_is_rejected_before_report_generation() {
    let mut snapshot = healthy_snapshot();
    snapshot.captured_at = "2026-02-30T12:00:00Z".to_owned();

    let error = evaluate_merge_readiness(&snapshot).unwrap_err();

    assert!(error.to_string().contains("RFC 3339 UTC timestamp"));
}

#[test]
fn missing_one_of_multiple_required_app_bindings_is_actionable() {
    let mut snapshot = healthy_snapshot();
    snapshot
        .branch_policy
        .required_checks
        .push(RequiredCheckSnapshot {
            context: CONTEXT.to_owned(),
            integration_id: Some(8_888),
            rulesets: vec![RulesetRef {
                id: 72,
                name: "organization protection".to_owned(),
                url: "https://github.com/acme/widgets/rules/72".to_owned(),
                source_type: "Organization".to_owned(),
                source: "acme".to_owned(),
            }],
        });

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert!(report.findings.iter().any(|finding| {
        finding.rule == FindingRule::RequiredCheckSourceMismatch && finding.rulesets.len() == 2
    }));
}

#[test]
fn multiple_explicit_app_bindings_are_not_ambiguous_when_all_are_observed() {
    let mut snapshot = healthy_snapshot();
    snapshot.branch_policy.merge_queue_rule = false;
    snapshot
        .branch_policy
        .required_checks
        .push(RequiredCheckSnapshot {
            context: CONTEXT.to_owned(),
            integration_id: Some(8_888),
            rulesets: vec![RulesetRef {
                id: 72,
                name: "organization protection".to_owned(),
                url: "https://github.com/acme/widgets/rules/72".to_owned(),
                source_type: "Organization".to_owned(),
                source: "acme".to_owned(),
            }],
        });
    snapshot.pull_requests[0].check_runs.push(CheckRunSnapshot {
        id: 502,
        url: "https://github.com/acme/widgets/checks/502".to_owned(),
        name: CONTEXT.to_owned(),
        app_id: Some(8_888),
        app_slug: Some("policy-app".to_owned()),
        check_suite_id: None,
        status: "completed".to_owned(),
        conclusion: Some("success".to_owned()),
    });

    let report = evaluate_merge_readiness(&snapshot).unwrap();

    assert_eq!(report.summary.verdict, AuditVerdict::NoObservedRisk);
    assert!(report.findings.is_empty());
    assert!(report.unknowns.is_empty());
}

#[test]
fn markdown_replaces_bidirectional_format_controls() {
    let mut snapshot = healthy_snapshot();
    snapshot.repository.default_branch = "main&rlm;\u{202e}txt".to_owned();

    let report = evaluate_merge_readiness(&snapshot).unwrap();
    let markdown = render_merge_readiness_markdown(&report);

    assert!(!markdown.contains('\u{202e}'));
    assert!(!markdown.contains("&rlm;"));
    assert!(markdown.contains("&amp;rlm;"));
    assert!(markdown.contains('\u{fffd}'));
}
