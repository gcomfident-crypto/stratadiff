use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowExpectedApp {
    #[serde(rename = "github-actions")]
    GithubActions,
    #[serde(rename = "third-party")]
    ThirdParty,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowProviderCapability {
    NotApplicable,
    Unknown,
    UnsupportedMergeGroup,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTargetKind {
    MergeGroup,
    PullRequestHead,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTarget {
    pub kind: WorkflowTargetKind,
    pub sha: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMergeableState {
    Conflicting,
    Mergeable,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPullRequest {
    pub base_ref: String,
    pub base_sha: String,
    pub head_ref: String,
    pub head_repository_is_fork: bool,
    pub head_sha: String,
    pub mergeable_state: WorkflowMergeableState,
    pub number: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowChangedFiles {
    pub complete: bool,
    pub github_filter_file_limit_reached: bool,
    pub paths: Vec<String>,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestWorkflowTrigger {
    pub branches: Vec<String>,
    pub branches_ignore: Vec<String>,
    pub paths: Vec<String>,
    pub paths_ignore: Vec<String>,
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeGroupWorkflowTrigger {
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTriggers {
    pub merge_group: Option<MergeGroupWorkflowTrigger>,
    pub pull_request: Option<PullRequestWorkflowTrigger>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowJob {
    pub condition: String,
    pub id: String,
    pub name: String,
    pub name_static: bool,
    pub reusable: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    Active,
    DisabledInactivity,
    DisabledManually,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSyntax {
    Invalid,
    Valid,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub jobs: Vec<WorkflowJob>,
    pub path: String,
    pub state: WorkflowState,
    pub syntax: WorkflowSyntax,
    pub triggers: WorkflowTriggers,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunEvent {
    MergeGroup,
    PullRequest,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Completed,
    InProgress,
    Pending,
    Queued,
    Requested,
    Waiting,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunConclusion {
    ActionRequired,
    Cancelled,
    Failure,
    Neutral,
    Skipped,
    Stale,
    StartupFailure,
    Success,
    TimedOut,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRunObservation {
    pub conclusion: Option<WorkflowRunConclusion>,
    pub event: WorkflowRunEvent,
    pub head_sha: String,
    pub status: WorkflowRunStatus,
    pub workflow_path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowTriggerInput {
    pub changed_files: WorkflowChangedFiles,
    pub collection_gaps: Vec<String>,
    pub expected_app: WorkflowExpectedApp,
    pub historical_check_names: Vec<String>,
    pub last_activity: String,
    pub provider_capability: WorkflowProviderCapability,
    pub pull_request: WorkflowPullRequest,
    pub required_context: String,
    pub runs: Vec<WorkflowRunObservation>,
    pub target: WorkflowTarget,
    pub workflows: Vec<WorkflowDefinition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerCause {
    DuplicateJobNameAmbiguous,
    ForkApprovalPossible,
    ForkApprovalRequired,
    MergeGroupTriggerMissing,
    None,
    ProviderDidNotEmitMergeGroupStatus,
    ProviderRuntimeDeliveryGap,
    PullRequestMergeConflict,
    RequiredContextNotProduced,
    WorkflowActivityExcludesSynchronize,
    WorkflowBranchFilterExcluded,
    WorkflowDefinitionInvalid,
    WorkflowDisabled,
    WorkflowPathFilterExcluded,
    WorkflowTriggerUnknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerConfidence {
    Certain,
    High,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTriggerFix {
    pub action_code: String,
    pub argv: Vec<String>,
    pub requires_human_edit: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowTriggerDiagnosis {
    pub cause_code: WorkflowTriggerCause,
    pub confidence: WorkflowTriggerConfidence,
    pub evidence: Vec<String>,
    pub fix: WorkflowTriggerFix,
    pub target_sha: String,
}

pub fn classify_workflow_trigger(
    input: &DoctorWorkflowTriggerInput,
) -> Result<DoctorWorkflowTriggerDiagnosis> {
    validate_input(input)?;

    if input.expected_app == WorkflowExpectedApp::ThirdParty {
        return Ok(classify_third_party(input));
    }

    let invalid_startup = input.workflows.iter().any(|workflow| {
        workflow.syntax == WorkflowSyntax::Invalid
            && input.runs.iter().any(|run| {
                run.workflow_path == workflow.path
                    && run.status == WorkflowRunStatus::Completed
                    && run.conclusion == Some(WorkflowRunConclusion::StartupFailure)
            })
    });
    if invalid_startup {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowDefinitionInvalid,
            WorkflowTriggerConfidence::Certain,
            ["workflow_syntax:invalid", "run_conclusion:startup_failure"],
            workflow_fix("repair_workflow_syntax", true),
        ));
    }

    let has_dynamic_job = input
        .workflows
        .iter()
        .flat_map(|workflow| &workflow.jobs)
        .any(|job| !job.name_static);
    if has_dynamic_job && has_gap(input, "dynamic_job_name") {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowTriggerUnknown,
            WorkflowTriggerConfidence::Uncertain,
            ["job_name:dynamic", "gap:dynamic_job_name"],
            workflow_fix("collect_expanded_job_names", false),
        ));
    }

    let has_reusable_job = input
        .workflows
        .iter()
        .flat_map(|workflow| &workflow.jobs)
        .any(|job| job.reusable);
    if has_reusable_job && has_gap(input, "reusable_workflow_definition_unavailable") {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowTriggerUnknown,
            WorkflowTriggerConfidence::Uncertain,
            [
                "job:reusable",
                "gap:reusable_workflow_definition_unavailable",
            ],
            workflow_fix("collect_reusable_workflow_definition", false),
        ));
    }

    let matching = matching_jobs(input);
    if matching.is_empty() {
        let current_static = input
            .workflows
            .iter()
            .filter(|workflow| {
                workflow.state == WorkflowState::Active && workflow.syntax == WorkflowSyntax::Valid
            })
            .flat_map(|workflow| {
                workflow
                    .jobs
                    .iter()
                    .filter(|job| job.name_static)
                    .map(move |job| (workflow, job))
            })
            .collect::<Vec<_>>();
        let expected_event = match input.target.kind {
            WorkflowTargetKind::MergeGroup => WorkflowRunEvent::MergeGroup,
            WorkflowTargetKind::PullRequestHead => WorkflowRunEvent::PullRequest,
        };
        if current_static.len() == 1
            && input
                .historical_check_names
                .iter()
                .any(|name| name == &input.required_context)
            && has_observed_run(input, current_static[0].0, expected_event)
        {
            let current_name = &current_static[0].1.name;
            return Ok(diagnosis(
                input,
                WorkflowTriggerCause::RequiredContextNotProduced,
                WorkflowTriggerConfidence::High,
                [
                    format!("required:{}", input.required_context),
                    format!("historical:{}", input.required_context),
                    format!("current_job:{current_name}"),
                ],
                workflow_fix("synchronize_required_context", true),
            ));
        }
        return Ok(unknown(
            input,
            "required_context:unmapped",
            "collect_required_context_producer",
        ));
    }

    let active_matching = matching
        .iter()
        .copied()
        .filter(|(workflow, _)| workflow.state == WorkflowState::Active)
        .collect::<Vec<_>>();
    if active_matching.is_empty() {
        let workflow = matching[0].0;
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowDisabled,
            WorkflowTriggerConfidence::Certain,
            [
                format!("workflow:{}", workflow.path),
                format!("workflow_state:{}", workflow_state_name(workflow.state)),
            ],
            WorkflowTriggerFix {
                action_code: "enable_workflow".to_owned(),
                argv: vec![
                    "gh".to_owned(),
                    "workflow".to_owned(),
                    "enable".to_owned(),
                    workflow.path.clone(),
                ],
                requires_human_edit: false,
            },
        ));
    }
    if active_matching.len() > 1 {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::DuplicateJobNameAmbiguous,
            WorkflowTriggerConfidence::Certain,
            active_matching
                .iter()
                .map(|(workflow, _)| format!("job:{}@{}", input.required_context, workflow.path)),
            workflow_fix("make_required_job_names_unique", true),
        ));
    }

    let (workflow, job) = active_matching[0];
    let approval_required = input.runs.iter().any(|run| {
        run.head_sha == input.target.sha
            && run.workflow_path == workflow.path
            && run.conclusion == Some(WorkflowRunConclusion::ActionRequired)
    });
    if input.pull_request.head_repository_is_fork && approval_required {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::ForkApprovalRequired,
            WorkflowTriggerConfidence::Certain,
            [
                "head_repository:fork",
                "run_conclusion:action_required",
                "run_head:exact",
            ],
            workflow_fix("approve_fork_workflow", false),
        ));
    }

    if input.target.kind == WorkflowTargetKind::PullRequestHead
        && input.pull_request.mergeable_state == WorkflowMergeableState::Conflicting
        && workflow.triggers.pull_request.is_some()
    {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::PullRequestMergeConflict,
            WorkflowTriggerConfidence::Certain,
            [
                "mergeable_state:conflicting",
                "trigger:pull_request",
                "exact_head_run:absent",
            ],
            workflow_fix("resolve_merge_conflict", true),
        ));
    }

    if input.target.kind == WorkflowTargetKind::MergeGroup {
        if workflow.triggers.merge_group.is_none() {
            return Ok(diagnosis(
                input,
                WorkflowTriggerCause::MergeGroupTriggerMissing,
                WorkflowTriggerConfidence::Certain,
                [
                    "target:merge_group".to_owned(),
                    format!("workflow:{}", workflow.path),
                    "trigger:merge_group_absent".to_owned(),
                ],
                workflow_fix("add_merge_group_trigger", true),
            ));
        }
        if has_observed_run(input, workflow, WorkflowRunEvent::MergeGroup) {
            return Ok(diagnosis(
                input,
                WorkflowTriggerCause::None,
                WorkflowTriggerConfidence::Certain,
                [
                    "target:merge_group",
                    "trigger:merge_group_checks_requested",
                    "run:queued",
                ],
                workflow_fix("none", false),
            ));
        }
        return Ok(unknown(
            input,
            "merge_group_run:absent",
            "collect_merge_group_run",
        ));
    }

    let Some(trigger) = workflow.triggers.pull_request.as_ref() else {
        return Ok(unknown(
            input,
            "trigger:pull_request_absent",
            "add_or_inspect_pull_request_trigger",
        ));
    };
    let default_types = ["opened", "reopened", "synchronize"];
    let activity_matches = if trigger.types.is_empty() {
        default_types.contains(&input.last_activity.as_str())
    } else {
        trigger
            .types
            .iter()
            .any(|activity| activity == &input.last_activity)
    };
    if !activity_matches {
        if input.last_activity != "synchronize" {
            return Ok(unknown(
                input,
                "activity_filter:excluded_non_synchronize",
                "inspect_pull_request_activity_filter",
            ));
        }
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowActivityExcludesSynchronize,
            WorkflowTriggerConfidence::Certain,
            [
                format!("last_activity:{}", input.last_activity),
                format!("types:{}", trigger.types.join(",")),
                "exact_head_run:absent".to_owned(),
            ],
            workflow_fix("add_pull_request_synchronize", true),
        ));
    }

    let branch_match = evaluate_ref_filters(
        &trigger.branches,
        &trigger.branches_ignore,
        &input.pull_request.base_ref,
    );
    if branch_match == FilterDecision::Excluded {
        let (filter_name, patterns) = if trigger.branches.is_empty() {
            ("branches_ignore", trigger.branches_ignore.join(","))
        } else {
            ("branches", trigger.branches.join(","))
        };
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowBranchFilterExcluded,
            WorkflowTriggerConfidence::Certain,
            [
                format!("base:{}", input.pull_request.base_ref),
                format!("{filter_name}:{patterns}"),
                format!("workflow:{}", workflow.path),
            ],
            workflow_fix("align_required_base_branch_filter", true),
        ));
    }
    if branch_match == FilterDecision::Unknown {
        return Ok(unknown(
            input,
            "branch_filter:unsupported_pattern",
            "inspect_branch_filter",
        ));
    }

    if input.changed_files.github_filter_file_limit_reached {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowTriggerUnknown,
            WorkflowTriggerConfidence::Uncertain,
            [
                format!("changed_files:{}", input.changed_files.total),
                "github_filter_limit:reached".to_owned(),
                "gap:github_path_filter_evaluation_truncated".to_owned(),
            ],
            workflow_fix("inspect_github_filter_evaluation", false),
        ));
    }

    let path_match = evaluate_path_filters(
        &trigger.paths,
        &trigger.paths_ignore,
        &input.changed_files.paths,
    );
    if path_match == FilterDecision::Excluded {
        let evidence = if !trigger.paths.is_empty() {
            vec![
                format!("changed:{}", input.changed_files.paths[0]),
                format!("paths:{}", trigger.paths.join(",")),
                format!("workflow:{}", workflow.path),
            ]
        } else {
            vec![
                format!("changed:{}", input.changed_files.paths[0]),
                format!("paths_ignore:{}", trigger.paths_ignore[0]),
                format!("workflow:{}", workflow.path),
            ]
        };
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::WorkflowPathFilterExcluded,
            WorkflowTriggerConfidence::Certain,
            evidence,
            workflow_fix("move_required_filter_inside_workflow", true),
        ));
    }
    if path_match == FilterDecision::Unknown {
        return Ok(unknown(
            input,
            "path_filter:unsupported_pattern",
            "inspect_path_filter",
        ));
    }

    if input.pull_request.head_repository_is_fork
        && has_gap(input, "fork_approval_policy_unavailable")
    {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::ForkApprovalPossible,
            WorkflowTriggerConfidence::Uncertain,
            [
                "head_repository:fork",
                "exact_head_run:absent",
                "gap:fork_approval_policy_unavailable",
            ],
            WorkflowTriggerFix {
                action_code: "inspect_fork_approval_policy".to_owned(),
                argv: vec![
                    "gh".to_owned(),
                    "api".to_owned(),
                    "repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval"
                        .to_owned(),
                ],
                requires_human_edit: false,
            },
        ));
    }

    let observed_run = input.runs.iter().find(|run| {
        run.head_sha == input.target.sha
            && run.workflow_path == workflow.path
            && run.event == WorkflowRunEvent::PullRequest
    });
    let Some(observed_run) = observed_run else {
        return Ok(unknown(
            input,
            "pull_request_run:absent",
            "collect_pull_request_run",
        ));
    };
    if job.condition != "always" {
        if observed_run.conclusion != Some(WorkflowRunConclusion::Skipped) {
            return Ok(unknown(
                input,
                "conditional_job:outcome_unproven",
                "inspect_conditional_job",
            ));
        }
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::None,
            WorkflowTriggerConfidence::Certain,
            [
                "workflow:triggered",
                "job:conditional",
                "run:skipped_success",
            ],
            workflow_fix("none", false),
        ));
    }
    if let Some(pattern) = trigger.branches.first() {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::None,
            WorkflowTriggerConfidence::Certain,
            [
                format!("base:{}", input.pull_request.base_ref),
                format!("head:{}", input.pull_request.head_ref),
                format!("branches:{pattern}"),
                "run:queued".to_owned(),
            ],
            workflow_fix("none", false),
        ));
    }
    if trigger.paths.iter().any(|pattern| pattern.starts_with('!')) {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::None,
            WorkflowTriggerConfidence::Certain,
            [
                format!("changed:{}", input.changed_files.paths[0]),
                "paths:ordered_reinclude".to_owned(),
                "run:queued".to_owned(),
            ],
            workflow_fix("none", false),
        ));
    }
    if let Some(pattern) = trigger.paths.first() {
        return Ok(diagnosis(
            input,
            WorkflowTriggerCause::None,
            WorkflowTriggerConfidence::Certain,
            [
                format!("changed:{}", input.changed_files.paths[0]),
                format!("paths:{pattern}"),
                "run:queued".to_owned(),
            ],
            workflow_fix("none", false),
        ));
    }

    Ok(unknown(
        input,
        "trigger:no_supported_explanation",
        "inspect_workflow_run",
    ))
}

fn classify_third_party(input: &DoctorWorkflowTriggerInput) -> DoctorWorkflowTriggerDiagnosis {
    if input.provider_capability == WorkflowProviderCapability::UnsupportedMergeGroup
        && input.target.kind == WorkflowTargetKind::MergeGroup
    {
        return diagnosis(
            input,
            WorkflowTriggerCause::ProviderDidNotEmitMergeGroupStatus,
            WorkflowTriggerConfidence::High,
            [
                "target:merge_group",
                "expected_app:third-party",
                "provider_capability:unsupported_merge_group",
            ],
            workflow_fix("replace_or_ungate_unsupported_provider", true),
        );
    }
    if has_gap(input, "provider_webhook_delivery_unavailable") {
        return diagnosis(
            input,
            WorkflowTriggerCause::ProviderRuntimeDeliveryGap,
            WorkflowTriggerConfidence::Uncertain,
            [
                "expected_app:third-party",
                "exact_target_status:absent",
                "gap:provider_webhook_delivery_unavailable",
            ],
            workflow_fix("inspect_provider_webhook_delivery", false),
        );
    }
    unknown(
        input,
        "provider_capability:unknown",
        "inspect_provider_capability",
    )
}

fn matching_jobs(input: &DoctorWorkflowTriggerInput) -> Vec<(&WorkflowDefinition, &WorkflowJob)> {
    input
        .workflows
        .iter()
        .flat_map(|workflow| {
            workflow.jobs.iter().filter_map(move |job| {
                (job.name_static && job.name == input.required_context).then_some((workflow, job))
            })
        })
        .collect()
}

fn has_observed_run(
    input: &DoctorWorkflowTriggerInput,
    workflow: &WorkflowDefinition,
    event: WorkflowRunEvent,
) -> bool {
    input.runs.iter().any(|run| {
        run.head_sha == input.target.sha && run.workflow_path == workflow.path && run.event == event
    })
}

fn has_gap(input: &DoctorWorkflowTriggerInput, gap: &str) -> bool {
    input.collection_gaps.iter().any(|value| value == gap)
}

fn workflow_state_name(state: WorkflowState) -> &'static str {
    match state {
        WorkflowState::Active => "active",
        WorkflowState::DisabledInactivity => "disabled_inactivity",
        WorkflowState::DisabledManually => "disabled_manually",
    }
}

fn workflow_fix(action_code: &str, requires_human_edit: bool) -> WorkflowTriggerFix {
    WorkflowTriggerFix {
        action_code: action_code.to_owned(),
        argv: Vec::new(),
        requires_human_edit,
    }
}

fn diagnosis(
    input: &DoctorWorkflowTriggerInput,
    cause_code: WorkflowTriggerCause,
    confidence: WorkflowTriggerConfidence,
    evidence: impl IntoIterator<Item = impl Into<String>>,
    fix: WorkflowTriggerFix,
) -> DoctorWorkflowTriggerDiagnosis {
    DoctorWorkflowTriggerDiagnosis {
        cause_code,
        confidence,
        evidence: evidence.into_iter().map(Into::into).collect(),
        fix,
        target_sha: input.target.sha.clone(),
    }
}

fn unknown(
    input: &DoctorWorkflowTriggerInput,
    evidence: &str,
    action_code: &str,
) -> DoctorWorkflowTriggerDiagnosis {
    diagnosis(
        input,
        WorkflowTriggerCause::WorkflowTriggerUnknown,
        WorkflowTriggerConfidence::Uncertain,
        [evidence],
        workflow_fix(action_code, false),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterDecision {
    Included,
    Excluded,
    Unknown,
}

fn evaluate_ref_filters(includes: &[String], ignores: &[String], value: &str) -> FilterDecision {
    if ignores.iter().any(|pattern| pattern.starts_with('!'))
        || includes
            .iter()
            .chain(ignores)
            .map(|pattern| pattern.strip_prefix('!').unwrap_or(pattern))
            .any(|pattern| !supported_pattern(pattern))
    {
        return FilterDecision::Unknown;
    }
    if !includes.is_empty() && !ordered_path_selected(includes, value) {
        return FilterDecision::Excluded;
    }
    if ignores.iter().any(|pattern| glob_matches(pattern, value)) {
        return FilterDecision::Excluded;
    }
    FilterDecision::Included
}

fn evaluate_path_filters(
    includes: &[String],
    ignores: &[String],
    changed_paths: &[String],
) -> FilterDecision {
    if changed_paths.is_empty() {
        return FilterDecision::Unknown;
    }
    if ignores.iter().any(|pattern| pattern.starts_with('!'))
        || includes
            .iter()
            .chain(ignores)
            .map(|pattern| pattern.strip_prefix('!').unwrap_or(pattern))
            .any(|pattern| !supported_pattern(pattern))
    {
        return FilterDecision::Unknown;
    }
    if !includes.is_empty()
        && !changed_paths
            .iter()
            .any(|path| ordered_path_selected(includes, path))
    {
        return FilterDecision::Excluded;
    }
    if !ignores.is_empty()
        && changed_paths
            .iter()
            .all(|path| ignores.iter().any(|pattern| glob_matches(pattern, path)))
    {
        return FilterDecision::Excluded;
    }
    FilterDecision::Included
}

fn ordered_path_selected(patterns: &[String], path: &str) -> bool {
    let mut selected = false;
    for pattern in patterns {
        let (negative, pattern) = match pattern.strip_prefix('!') {
            Some(value) => (true, value),
            None => (false, pattern.as_str()),
        };
        if glob_matches(pattern, path) {
            selected = !negative;
        }
    }
    selected
}

fn supported_pattern(pattern: &str) -> bool {
    !pattern.contains(['[', ']', '+', '{', '}', '\\']) && !pattern.is_empty()
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let mut memo = vec![vec![None; value.len() + 1]; pattern.len() + 1];
    glob_matches_at(&pattern, &value, 0, 0, &mut memo)
}

fn glob_matches_at(
    pattern: &[char],
    value: &[char],
    pattern_index: usize,
    value_index: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(result) = memo[pattern_index][value_index] {
        return result;
    }
    let result = if pattern_index == pattern.len() {
        value_index == value.len()
    } else if pattern[pattern_index] == '*' {
        let is_double = pattern.get(pattern_index + 1) == Some(&'*');
        let followed_by_slash = is_double && pattern.get(pattern_index + 2) == Some(&'/');
        let next_pattern = pattern_index
            + if followed_by_slash {
                3
            } else if is_double {
                2
            } else {
                1
            };
        glob_matches_at(pattern, value, next_pattern, value_index, memo)
            || (value_index < value.len()
                && (is_double || value[value_index] != '/')
                && glob_matches_at(pattern, value, pattern_index, value_index + 1, memo))
    } else if value_index < value.len()
        && ((pattern[pattern_index] == '?' && value[value_index] != '/')
            || pattern[pattern_index] == value[value_index])
    {
        glob_matches_at(pattern, value, pattern_index + 1, value_index + 1, memo)
    } else {
        false
    };
    memo[pattern_index][value_index] = Some(result);
    result
}

fn validate_input(input: &DoctorWorkflowTriggerInput) -> Result<()> {
    validate_non_empty("required_context", &input.required_context)?;
    validate_non_empty("last_activity", &input.last_activity)?;
    validate_sha("target.sha", &input.target.sha)?;
    validate_sha("pull_request.base_sha", &input.pull_request.base_sha)?;
    validate_sha("pull_request.head_sha", &input.pull_request.head_sha)?;
    validate_non_empty("pull_request.base_ref", &input.pull_request.base_ref)?;
    validate_non_empty("pull_request.head_ref", &input.pull_request.head_ref)?;
    ensure!(
        input.pull_request.number > 0,
        "pull_request.number must be positive"
    );
    match input.target.kind {
        WorkflowTargetKind::PullRequestHead => ensure!(
            input.target.sha == input.pull_request.head_sha,
            "pull-request target must equal pull_request.head_sha"
        ),
        WorkflowTargetKind::MergeGroup => ensure!(
            input.target.sha != input.pull_request.head_sha,
            "merge-group target must differ from pull_request.head_sha"
        ),
    }
    ensure!(
        input.changed_files.paths.len() <= MAX_ITEMS,
        "changed_files.paths exceeds {MAX_ITEMS} items"
    );
    ensure!(
        input.changed_files.total >= input.changed_files.paths.len() as u64,
        "changed_files.total is smaller than paths"
    );
    if input.changed_files.github_filter_file_limit_reached {
        ensure!(
            input.changed_files.total > 300,
            "GitHub filter limit requires more than 300 changed files"
        );
    } else if input.changed_files.complete {
        ensure!(
            input.changed_files.total == input.changed_files.paths.len() as u64,
            "complete changed-file collection must include every path"
        );
    }
    validate_unique_non_empty("changed_files.paths", &input.changed_files.paths)?;
    validate_unique_non_empty("collection_gaps", &input.collection_gaps)?;
    validate_unique_non_empty("historical_check_names", &input.historical_check_names)?;
    ensure!(
        input.runs.len() <= MAX_ITEMS,
        "runs exceeds {MAX_ITEMS} items"
    );
    ensure!(
        input.workflows.len() <= MAX_ITEMS,
        "workflows exceeds {MAX_ITEMS} items"
    );
    ensure!(
        input.expected_app != WorkflowExpectedApp::GithubActions
            || input.provider_capability == WorkflowProviderCapability::NotApplicable,
        "GitHub Actions input cannot assert third-party provider capability"
    );

    let mut workflow_paths = BTreeSet::new();
    for workflow in &input.workflows {
        validate_non_empty("workflow.path", &workflow.path)?;
        ensure!(
            workflow.path.starts_with(".github/workflows/")
                && (workflow.path.ends_with(".yml") || workflow.path.ends_with(".yaml")),
            "workflow path is not under .github/workflows"
        );
        ensure!(
            workflow_paths.insert(workflow.path.as_str()),
            "workflow paths must be unique"
        );
        ensure!(
            workflow.jobs.len() <= MAX_ITEMS,
            "workflow jobs exceeds {MAX_ITEMS} items"
        );
        if workflow.syntax == WorkflowSyntax::Invalid {
            ensure!(
                workflow.jobs.is_empty(),
                "invalid workflow cannot claim parsed jobs"
            );
        }
        let mut job_ids = BTreeSet::new();
        for job in &workflow.jobs {
            validate_non_empty("job.id", &job.id)?;
            validate_non_empty("job.name", &job.name)?;
            validate_non_empty("job.condition", &job.condition)?;
            ensure!(
                job_ids.insert(job.id.as_str()),
                "job ids must be unique per workflow"
            );
        }
        if let Some(trigger) = &workflow.triggers.pull_request {
            ensure!(
                trigger.branches.is_empty() || trigger.branches_ignore.is_empty(),
                "pull_request cannot combine branches and branches_ignore"
            );
            ensure!(
                trigger.paths.is_empty() || trigger.paths_ignore.is_empty(),
                "pull_request cannot combine paths and paths_ignore"
            );
            validate_unique_non_empty("pull_request.branches", &trigger.branches)?;
            validate_unique_non_empty("pull_request.branches_ignore", &trigger.branches_ignore)?;
            validate_unique_non_empty("pull_request.paths", &trigger.paths)?;
            validate_unique_non_empty("pull_request.paths_ignore", &trigger.paths_ignore)?;
            validate_unique_non_empty("pull_request.types", &trigger.types)?;
        }
        if let Some(trigger) = &workflow.triggers.merge_group {
            validate_unique_non_empty("merge_group.types", &trigger.types)?;
            ensure!(
                trigger.types.is_empty()
                    || trigger
                        .types
                        .iter()
                        .all(|value| value == "checks_requested"),
                "merge_group only supports checks_requested"
            );
        }
    }
    for run in &input.runs {
        validate_non_empty("run.workflow_path", &run.workflow_path)?;
        validate_sha("run.head_sha", &run.head_sha)?;
        ensure!(
            run.head_sha == input.target.sha,
            "run is not bound to exact target"
        );
        ensure!(
            (run.status == WorkflowRunStatus::Completed) == run.conclusion.is_some(),
            "completed runs require a conclusion and incomplete runs forbid one"
        );
    }
    Ok(())
}

fn validate_non_empty(label: &str, value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{label} must not be empty");
    Ok(())
}

fn validate_unique_non_empty(label: &str, values: &[String]) -> Result<()> {
    ensure!(
        values.len() <= MAX_ITEMS,
        "{label} exceeds {MAX_ITEMS} items"
    );
    let mut unique = BTreeSet::new();
    for value in values {
        validate_non_empty(label, value)?;
        ensure!(
            unique.insert(value.as_str()),
            "{label} must not contain duplicates"
        );
    }
    Ok(())
}

fn validate_sha(label: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} must be a lowercase 40-character hex SHA"
    );
    Ok(())
}
