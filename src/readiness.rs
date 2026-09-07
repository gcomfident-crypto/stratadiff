use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MERGE_READINESS_SNAPSHOT_SCHEMA: &str = "stratadiff-merge-readiness-snapshot-v1";
pub const MERGE_READINESS_AUDIT_SCHEMA: &str = "stratadiff-merge-readiness-audit-v1";
pub const GITHUB_ACTIONS_APP_ID: u64 = 15_368;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionStatus {
    Complete,
    Partial,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CollectionSurface {
    Repository,
    EffectiveRules,
    Rulesets,
    Workflows,
    WorkflowDefinitions,
    PullRequests,
    CheckRuns,
    CommitStatuses,
    WorkflowRuns,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CollectionGap {
    pub surface: CollectionSurface,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCollection {
    pub status: CollectionStatus,
    pub api_calls: u64,
    pub response_bytes: u64,
    pub gaps: Vec<CollectionGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepositorySnapshot {
    pub database_id: u64,
    pub name_with_owner: String,
    pub url: String,
    pub default_branch: String,
    pub default_branch_head_sha: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct RulesetRef {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub source_type: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheckSnapshot {
    pub context: String,
    pub integration_id: Option<u64>,
    pub rulesets: Vec<RulesetRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BranchPolicySnapshot {
    pub branch_protected: bool,
    pub effective_rule_count: u64,
    pub pull_request_rule: bool,
    pub merge_queue_rule: bool,
    pub required_checks: Vec<RequiredCheckSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TriggerSnapshot {
    pub paths: Vec<String>,
    pub paths_ignore: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub pull_request: Option<TriggerSnapshot>,
    pub pull_request_target: Option<TriggerSnapshot>,
    pub merge_group: Option<TriggerSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSnapshot {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub state: String,
    pub url: String,
    pub definition: Option<WorkflowDefinition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Merged,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckRunSnapshot {
    pub id: u64,
    pub url: String,
    pub name: String,
    pub app_id: Option<u64>,
    pub app_slug: Option<String>,
    pub check_suite_id: Option<u64>,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommitStatusSnapshot {
    pub id: u64,
    pub url: String,
    pub context: String,
    pub creator_id: Option<u64>,
    pub creator_login: Option<String>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRunSnapshot {
    pub id: u64,
    pub url: String,
    pub workflow_id: u64,
    pub path: String,
    pub event: String,
    pub check_suite_id: u64,
    pub run_attempt: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestSnapshot {
    pub number: u64,
    pub url: String,
    pub state: PullRequestState,
    pub head_sha: String,
    pub collection: CollectionStatus,
    pub check_runs: Vec<CheckRunSnapshot>,
    pub statuses: Vec<CommitStatusSnapshot>,
    pub workflow_runs: Vec<WorkflowRunSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeReadinessSnapshot {
    pub schema: String,
    pub captured_at: String,
    pub repository: RepositorySnapshot,
    pub collection: SnapshotCollection,
    pub branch_policy: BranchPolicySnapshot,
    pub workflows: Vec<WorkflowSnapshot>,
    pub pull_requests: Vec<PullRequestSnapshot>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FindingRule {
    DefaultBranchUnprotected,
    RequiredCheckSourceUnpinned,
    RequiredCheckSourceMismatch,
    RequiredCheckSourceAmbiguous,
    MergeGroupTriggerMissing,
    PathFilteredRequiredWorkflow,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    High,
    Medium,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Repository,
    Ruleset,
    Workflow,
    PullRequest,
    CheckRun,
    WorkflowRun,
    CommitStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub url: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditFinding {
    pub rule: FindingRule,
    pub severity: FindingSeverity,
    pub title: String,
    pub explanation: String,
    pub contexts: Vec<String>,
    pub workflow_paths: Vec<String>,
    pub rulesets: Vec<RulesetRef>,
    pub evidence: Vec<EvidenceRef>,
    pub remediation: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum UnknownCode {
    CollectionIncomplete,
    RequiredCheckNeverObserved,
    RequiredCheckMissingOnSampledHead,
    RequiredCheckSourceUnresolved,
    WorkflowMappingUnavailable,
    WorkflowDefinitionUnavailable,
    ExternalMergeGroupSupportUnobserved,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditUnknown {
    pub code: UnknownCode,
    pub context: Option<String>,
    pub workflow_path: Option<String>,
    pub reason: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    ActionRequired,
    NoObservedRisk,
    Inconclusive,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditScope {
    pub provider_url: String,
    pub repository: String,
    pub default_branch: String,
    pub default_branch_head_sha: String,
    pub sampled_pull_requests: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditPrivacy {
    pub repository_source_collected: bool,
    pub workflow_definitions_collected: bool,
    pub pull_request_text_collected: bool,
    pub check_run_output_text_collected: bool,
    pub commit_status_description_collected: bool,
    pub review_text_collected: bool,
    pub commit_messages_collected: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditClaimBoundary {
    pub current_configuration_snapshot_supported: bool,
    pub historical_configuration_at_merge_supported: bool,
    pub merge_safety_supported: bool,
    pub workflow_runtime_semantics_supported: bool,
    pub cost_savings_supported: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditSummary {
    pub verdict: AuditVerdict,
    pub findings: u64,
    pub high_findings: u64,
    pub medium_findings: u64,
    pub unknowns: u64,
    pub required_checks: u64,
    pub sampled_pull_requests: u64,
    pub fully_observed_pull_requests: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeReadinessAudit {
    pub schema: String,
    pub tool_version: String,
    pub generated_at: String,
    pub scope: AuditScope,
    pub collection: SnapshotCollection,
    pub privacy: AuditPrivacy,
    pub claim_boundary: AuditClaimBoundary,
    pub summary: AuditSummary,
    pub findings: Vec<AuditFinding>,
    pub unknowns: Vec<AuditUnknown>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Producer {
    CheckApp(u64),
    CommitStatus(u64),
}

#[derive(Default)]
struct ContextEvidence {
    producers: BTreeSet<Producer>,
    evidence: BTreeSet<EvidenceRef>,
    github_actions_runs: BTreeSet<(u64, String)>,
    github_actions_unmapped: BTreeSet<EvidenceRef>,
}

fn bounded_nonempty(value: &str, maximum: usize, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= maximum,
        "{label} must contain between 1 and {maximum} bytes"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} must not contain control characters"
    );
    Ok(())
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() {
            return false;
        }
    }
    let number = |tens: usize, ones: usize| {
        u32::from(bytes[tens] - b'0') * 10 + u32::from(bytes[ones] - b'0')
    };
    let year = u32::from(bytes[0] - b'0') * 1_000
        + u32::from(bytes[1] - b'0') * 100
        + u32::from(bytes[2] - b'0') * 10
        + u32::from(bytes[3] - b'0');
    let month = number(5, 6);
    let day = number(8, 9);
    let hour = number(11, 12);
    let minute = number(14, 15);
    let second = number(17, 18);
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let maximum_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    if !(1..=maximum_day).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if bytes[19] == b'Z' {
        return bytes.len() == 20;
    }
    bytes[19] == b'.'
        && bytes.len() > 21
        && bytes[bytes.len() - 1] == b'Z'
        && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
}

fn is_terminal_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

fn valid_url(value: &str) -> bool {
    value.starts_with("https://")
        && !value.chars().any(is_terminal_unsafe)
        && !value.chars().any(char::is_whitespace)
}

fn validate_url(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() <= 2_048 && valid_url(value),
        "{label} must be a canonical HTTPS URL"
    );
    Ok(())
}

fn valid_repository(value: &str) -> bool {
    let mut parts = value.split('/');
    let valid_component = |component: &str| {
        !component.is_empty()
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    };
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(owner), Some(name), None) if valid_component(owner) && valid_component(name)
    )
}

fn validate_ruleset(ruleset: &RulesetRef) -> Result<()> {
    ensure!(ruleset.id > 0, "ruleset ID must be positive");
    bounded_nonempty(&ruleset.name, 255, "ruleset name")?;
    validate_url(&ruleset.url, "ruleset URL")?;
    bounded_nonempty(&ruleset.source_type, 64, "ruleset source type")?;
    bounded_nonempty(&ruleset.source, 255, "ruleset source")
}

fn validate_snapshot(snapshot: &MergeReadinessSnapshot) -> Result<()> {
    ensure!(
        snapshot.schema == MERGE_READINESS_SNAPSHOT_SCHEMA,
        "unsupported merge-readiness snapshot schema"
    );
    bounded_nonempty(&snapshot.captured_at, 64, "snapshot captured_at")?;
    ensure!(
        valid_utc_timestamp(&snapshot.captured_at),
        "snapshot captured_at must be an RFC 3339 UTC timestamp"
    );
    ensure!(
        snapshot.repository.database_id > 0,
        "repository database ID must be positive"
    );
    bounded_nonempty(
        &snapshot.repository.name_with_owner,
        202,
        "repository name_with_owner",
    )?;
    ensure!(
        valid_repository(&snapshot.repository.name_with_owner),
        "repository name_with_owner must be OWNER/REPO"
    );
    validate_url(&snapshot.repository.url, "repository URL")?;
    bounded_nonempty(&snapshot.repository.default_branch, 255, "default branch")?;
    ensure!(
        valid_sha(&snapshot.repository.default_branch_head_sha),
        "default branch head must be a lowercase full Git object ID"
    );
    ensure!(
        snapshot
            .repository
            .url
            .ends_with(&format!("/{}", snapshot.repository.name_with_owner)),
        "repository URL must end with name_with_owner"
    );
    match snapshot.collection.status {
        CollectionStatus::Complete => ensure!(
            snapshot.collection.gaps.is_empty(),
            "a complete snapshot cannot contain collection gaps"
        ),
        CollectionStatus::Partial => ensure!(
            !snapshot.collection.gaps.is_empty(),
            "a partial snapshot must explain at least one collection gap"
        ),
    }
    for gap in &snapshot.collection.gaps {
        bounded_nonempty(&gap.reason, 4_096, "collection gap reason")?;
    }
    let mut workflow_ids = BTreeSet::new();
    let mut workflow_paths = BTreeSet::new();
    for workflow in &snapshot.workflows {
        ensure!(workflow.id > 0, "workflow ID must be positive");
        ensure!(workflow_ids.insert(workflow.id), "duplicate workflow ID");
        ensure!(
            workflow_paths.insert(workflow.path.clone()),
            "duplicate workflow path"
        );
        bounded_nonempty(&workflow.name, 255, "workflow name")?;
        bounded_nonempty(&workflow.path, 1_024, "workflow path")?;
        bounded_nonempty(&workflow.state, 64, "workflow state")?;
        validate_url(&workflow.url, "workflow URL")?;
        if snapshot.collection.status == CollectionStatus::Complete && workflow.state == "active" {
            ensure!(
                workflow.definition.is_some(),
                "a complete snapshot must include every active workflow definition"
            );
        }
    }
    for check in &snapshot.branch_policy.required_checks {
        bounded_nonempty(&check.context, 255, "required check context")?;
        ensure!(
            !check.rulesets.is_empty(),
            "required check must name a ruleset"
        );
        for ruleset in &check.rulesets {
            validate_ruleset(ruleset)?;
        }
        if let Some(integration_id) = check.integration_id {
            ensure!(integration_id > 0, "integration ID must be positive");
        }
    }
    let mut pull_numbers = BTreeSet::new();
    for pull_request in &snapshot.pull_requests {
        ensure!(
            pull_request.number > 0,
            "pull request number must be positive"
        );
        ensure!(
            pull_numbers.insert(pull_request.number),
            "duplicate pull request number"
        );
        validate_url(&pull_request.url, "pull request URL")?;
        ensure!(
            valid_sha(&pull_request.head_sha),
            "pull request head must be a lowercase full Git object ID"
        );
        ensure!(
            snapshot.collection.status == CollectionStatus::Partial
                || pull_request.collection == CollectionStatus::Complete,
            "a complete snapshot cannot contain a partial pull request"
        );
        for check_run in &pull_request.check_runs {
            ensure!(check_run.id > 0, "check run ID must be positive");
            validate_url(&check_run.url, "check run URL")?;
            bounded_nonempty(&check_run.name, 255, "check run name")?;
            bounded_nonempty(&check_run.status, 64, "check run status")?;
            if let Some(app_id) = check_run.app_id {
                ensure!(app_id > 0, "check run App ID must be positive");
            }
        }
        for status in &pull_request.statuses {
            ensure!(status.id > 0, "commit status ID must be positive");
            validate_url(&status.url, "commit status URL")?;
            bounded_nonempty(&status.context, 255, "commit status context")?;
            bounded_nonempty(&status.state, 64, "commit status state")?;
            if let Some(creator_id) = status.creator_id {
                ensure!(creator_id > 0, "commit-status creator ID must be positive");
            }
        }
        for run in &pull_request.workflow_runs {
            ensure!(run.id > 0, "workflow run ID must be positive");
            ensure!(
                run.workflow_id > 0,
                "workflow run workflow ID must be positive"
            );
            ensure!(
                run.check_suite_id > 0,
                "workflow run check-suite ID must be positive"
            );
            ensure!(run.run_attempt > 0, "workflow run attempt must be positive");
            validate_url(&run.url, "workflow run URL")?;
            bounded_nonempty(&run.path, 1_024, "workflow run path")?;
            bounded_nonempty(&run.event, 64, "workflow run event")?;
        }
    }
    Ok(())
}

fn repository_evidence(snapshot: &MergeReadinessSnapshot, description: String) -> EvidenceRef {
    EvidenceRef {
        kind: EvidenceKind::Repository,
        url: snapshot.repository.url.clone(),
        description,
    }
}

fn ruleset_union(checks: &[&RequiredCheckSnapshot]) -> Vec<RulesetRef> {
    let mut rulesets = BTreeSet::new();
    for check in checks {
        rulesets.extend(check.rulesets.iter().cloned());
    }
    rulesets.into_iter().collect()
}

fn ruleset_evidence(rulesets: &[RulesetRef]) -> Vec<EvidenceRef> {
    rulesets
        .iter()
        .map(|ruleset| EvidenceRef {
            kind: EvidenceKind::Ruleset,
            url: ruleset.url.clone(),
            description: format!("ruleset {} requires this context", ruleset.name),
        })
        .collect()
}

fn context_evidence(
    snapshot: &MergeReadinessSnapshot,
    contexts: &BTreeSet<String>,
) -> BTreeMap<String, ContextEvidence> {
    let mut observations = contexts
        .iter()
        .map(|context| (context.clone(), ContextEvidence::default()))
        .collect::<BTreeMap<_, _>>();
    for pull_request in &snapshot.pull_requests {
        let workflow_runs = pull_request
            .workflow_runs
            .iter()
            .map(|run| (run.check_suite_id, run))
            .collect::<BTreeMap<_, _>>();
        for check_run in &pull_request.check_runs {
            let Some(context) = observations.get_mut(&check_run.name) else {
                continue;
            };
            let evidence = EvidenceRef {
                kind: EvidenceKind::CheckRun,
                url: check_run.url.clone(),
                description: format!(
                    "pull request #{} head {} published check {}",
                    pull_request.number, pull_request.head_sha, check_run.name
                ),
            };
            context.evidence.insert(evidence.clone());
            if let Some(app_id) = check_run.app_id {
                context.producers.insert(Producer::CheckApp(app_id));
                if app_id == GITHUB_ACTIONS_APP_ID {
                    match check_run
                        .check_suite_id
                        .and_then(|check_suite_id| workflow_runs.get(&check_suite_id))
                    {
                        Some(run) => {
                            context
                                .github_actions_runs
                                .insert((run.workflow_id, run.path.clone()));
                        }
                        None => {
                            context.github_actions_unmapped.insert(evidence);
                        }
                    }
                }
            }
        }
        for status in &pull_request.statuses {
            let Some(context) = observations.get_mut(&status.context) else {
                continue;
            };
            let evidence = EvidenceRef {
                kind: EvidenceKind::CommitStatus,
                url: status.url.clone(),
                description: format!(
                    "pull request #{} head {} published legacy status {}",
                    pull_request.number, pull_request.head_sha, status.context
                ),
            };
            context.evidence.insert(evidence);
            if let Some(creator_id) = status.creator_id {
                context.producers.insert(Producer::CommitStatus(creator_id));
            }
        }
    }
    observations
}

fn sort_findings(findings: &mut [AuditFinding]) {
    findings.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.rule.cmp(&right.rule))
            .then_with(|| left.contexts.cmp(&right.contexts))
            .then_with(|| left.workflow_paths.cmp(&right.workflow_paths))
    });
}

fn sort_unknowns(unknowns: &mut [AuditUnknown]) {
    unknowns.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.context.cmp(&right.context))
            .then_with(|| left.workflow_path.cmp(&right.workflow_path))
    });
}

pub fn evaluate_merge_readiness(snapshot: &MergeReadinessSnapshot) -> Result<MergeReadinessAudit> {
    validate_snapshot(snapshot)?;
    let mut findings = Vec::new();
    let mut unknowns = Vec::new();

    if snapshot.collection.status == CollectionStatus::Partial {
        unknowns.push(AuditUnknown {
            code: UnknownCode::CollectionIncomplete,
            context: None,
            workflow_path: None,
            reason: "One or more GitHub surfaces were not collected completely; absence of another finding is not a clean result.".to_owned(),
            evidence: vec![repository_evidence(
                snapshot,
                "repository whose readiness snapshot is incomplete".to_owned(),
            )],
        });
    }

    let branch_policy_observed = !snapshot.collection.gaps.iter().any(|gap| {
        matches!(
            gap.surface,
            CollectionSurface::Repository | CollectionSurface::EffectiveRules
        )
    });
    if branch_policy_observed
        && !snapshot.branch_policy.branch_protected
        && !snapshot.branch_policy.pull_request_rule
        && snapshot.branch_policy.required_checks.is_empty()
    {
        findings.push(AuditFinding {
            rule: FindingRule::DefaultBranchUnprotected,
            severity: FindingSeverity::High,
            title: "The default branch has no observed pull-request or required-check gate"
                .to_owned(),
            explanation: format!(
                "GitHub reported no classic branch protection, pull-request rule, or required check for {}. Other effective rules do not establish pull-request or required-check enforcement.",
                snapshot.repository.default_branch
            ),
            contexts: Vec::new(),
            workflow_paths: Vec::new(),
            rulesets: Vec::new(),
            evidence: vec![repository_evidence(
                snapshot,
                format!(
                    "default branch {} at {} has no observed pull-request or required-check enforcement",
                    snapshot.repository.default_branch,
                    snapshot.repository.default_branch_head_sha
                ),
            )],
            remediation: "Protect the default branch with classic branch protection or an active pull-request or required-check rule, then capture a new snapshot.".to_owned(),
        });
    }

    let mut required_by_context: BTreeMap<String, Vec<&RequiredCheckSnapshot>> = BTreeMap::new();
    for check in &snapshot.branch_policy.required_checks {
        required_by_context
            .entry(check.context.clone())
            .or_default()
            .push(check);
    }
    let contexts = required_by_context.keys().cloned().collect::<BTreeSet<_>>();
    let observed_by_context = context_evidence(snapshot, &contexts);
    let workflows_by_id = snapshot
        .workflows
        .iter()
        .map(|workflow| (workflow.id, workflow))
        .collect::<BTreeMap<_, _>>();

    for (context_name, requirements) in required_by_context {
        let rulesets = ruleset_union(&requirements);
        let mut expected_app_ids = BTreeSet::new();
        let mut has_unpinned_requirement = false;
        for requirement in &requirements {
            match requirement.integration_id {
                Some(id) => {
                    expected_app_ids.insert(id);
                }
                None => has_unpinned_requirement = true,
            }
        }
        if has_unpinned_requirement {
            findings.push(AuditFinding {
                rule: FindingRule::RequiredCheckSourceUnpinned,
                severity: FindingSeverity::High,
                title: format!("Required check {context_name} is not pinned to an App"),
                explanation: "At least one effective required-check rule names the context without an integration_id, so the policy does not identify the expected publisher.".to_owned(),
                contexts: vec![context_name.clone()],
                workflow_paths: Vec::new(),
                rulesets: rulesets.clone(),
                evidence: ruleset_evidence(&rulesets),
                remediation: "Select the expected GitHub App as the required-check source in the ruleset and verify the new integration_id.".to_owned(),
            });
        }

        let observed = observed_by_context
            .get(&context_name)
            .expect("every required context has an observation bucket");
        let has_observed_evidence = !observed.evidence.is_empty();
        if has_observed_evidence {
            for pull_request in &snapshot.pull_requests {
                let context_observed = pull_request
                    .check_runs
                    .iter()
                    .any(|check_run| check_run.name == context_name)
                    || pull_request
                        .statuses
                        .iter()
                        .any(|status| status.context == context_name);
                if pull_request.collection == CollectionStatus::Complete && !context_observed {
                    unknowns.push(AuditUnknown {
                        code: UnknownCode::RequiredCheckMissingOnSampledHead,
                        context: Some(context_name.clone()),
                        workflow_path: None,
                        reason: format!(
                            "Required context {context_name} was absent from the fully collected exact head of pull request #{}.",
                            pull_request.number
                        ),
                        evidence: vec![EvidenceRef {
                            kind: EvidenceKind::PullRequest,
                            url: pull_request.url.clone(),
                            description: format!(
                                "pull request #{} head {} had no Check Run or commit status named {}",
                                pull_request.number, pull_request.head_sha, context_name
                            ),
                        }],
                    });
                }
            }
        }
        let actions_can_satisfy =
            has_unpinned_requirement || expected_app_ids.contains(&GITHUB_ACTIONS_APP_ID);
        if (has_unpinned_requirement && observed.producers.len() > 1)
            || (actions_can_satisfy && observed.github_actions_runs.len() > 1)
        {
            let workflow_paths = observed
                .github_actions_runs
                .iter()
                .map(|(_, path)| path.clone())
                .collect::<Vec<_>>();
            let mut evidence = ruleset_evidence(&rulesets)
                .into_iter()
                .collect::<BTreeSet<_>>();
            evidence.extend(observed.evidence.iter().cloned());
            findings.push(AuditFinding {
                rule: FindingRule::RequiredCheckSourceAmbiguous,
                severity: FindingSeverity::Medium,
                title: format!("Required check {context_name} has ambiguous publisher identity"),
                explanation: format!(
                    "The sampled exact heads contain {} publisher identities and {} eligible GitHub Actions workflow paths for this context, while at least one effective rule cannot distinguish every observed source.",
                    observed.producers.len(),
                    observed.github_actions_runs.len()
                ),
                contexts: vec![context_name.clone()],
                workflow_paths,
                rulesets: rulesets.clone(),
                evidence: evidence.into_iter().collect(),
                remediation: "Give every workflow job a unique context and pin each required context to its expected App source.".to_owned(),
            });
        }

        if !expected_app_ids.is_empty() {
            let mut mismatched_evidence = Vec::new();
            for pull_request in &snapshot.pull_requests {
                let matching = pull_request
                    .check_runs
                    .iter()
                    .filter(|check_run| check_run.name == context_name)
                    .collect::<Vec<_>>();
                let matching_statuses = pull_request
                    .statuses
                    .iter()
                    .filter(|status| status.context == context_name)
                    .collect::<Vec<_>>();
                let observed_expected_app_ids = matching
                    .iter()
                    .filter_map(|check_run| check_run.app_id)
                    .filter(|app_id| expected_app_ids.contains(app_id))
                    .collect::<BTreeSet<_>>();
                let missing_app_ids = expected_app_ids
                    .difference(&observed_expected_app_ids)
                    .copied()
                    .collect::<Vec<_>>();
                if missing_app_ids.is_empty()
                    || (matching.is_empty() && matching_statuses.is_empty())
                {
                    continue;
                }
                let source_unknown = pull_request.collection == CollectionStatus::Partial
                    || !matching_statuses.is_empty()
                    || matching.iter().any(|check_run| check_run.app_id.is_none());
                if source_unknown {
                    let reason = if pull_request.collection == CollectionStatus::Partial {
                        format!(
                            "Pull request #{} was collected partially, so its complete publisher set for this context is unknown.",
                            pull_request.number
                        )
                    } else {
                        format!(
                            "Pull request #{} exposes this context through evidence whose GitHub App identity is not available in the snapshot.",
                            pull_request.number
                        )
                    };
                    unknowns.push(AuditUnknown {
                        code: UnknownCode::RequiredCheckSourceUnresolved,
                        context: Some(context_name.clone()),
                        workflow_path: None,
                        reason,
                        evidence: vec![EvidenceRef {
                            kind: EvidenceKind::PullRequest,
                            url: pull_request.url.clone(),
                            description: format!(
                                "pull request #{} head {} has unresolved publisher identity for {}",
                                pull_request.number, pull_request.head_sha, context_name
                            ),
                        }],
                    });
                } else {
                    mismatched_evidence.push(EvidenceRef {
                        kind: EvidenceKind::PullRequest,
                        url: pull_request.url.clone(),
                        description: format!(
                            "pull request #{} head {} had context {} but was missing configured App IDs {:?}",
                            pull_request.number,
                            pull_request.head_sha,
                            context_name,
                            missing_app_ids
                        ),
                    });
                }
            }
            if !mismatched_evidence.is_empty() {
                findings.push(AuditFinding {
                    rule: FindingRule::RequiredCheckSourceMismatch,
                    severity: FindingSeverity::High,
                    title: format!(
                        "Required check {context_name} was observed without every configured source"
                    ),
                    explanation: format!(
                        "One or more sampled exact heads exposed this context without a matching Check Run from every configured App ID {:?}.",
                        expected_app_ids
                    ),
                    contexts: vec![context_name.clone()],
                    workflow_paths: Vec::new(),
                    rulesets: rulesets.clone(),
                    evidence: mismatched_evidence,
                    remediation: "Repair the required-check source or the publisher configuration; do not remove source binding to make the check pass.".to_owned(),
                });
            }
        }

        if !has_observed_evidence {
            unknowns.push(AuditUnknown {
                code: UnknownCode::RequiredCheckNeverObserved,
                context: Some(context_name.clone()),
                workflow_path: None,
                reason: "No matching Check Run or commit status was observed on the sampled exact heads.".to_owned(),
                evidence: ruleset_evidence(&rulesets),
            });
        }

        let has_external_pinned_requirement = expected_app_ids
            .iter()
            .any(|app_id| *app_id != GITHUB_ACTIONS_APP_ID);
        if has_external_pinned_requirement && snapshot.branch_policy.merge_queue_rule {
            unknowns.push(AuditUnknown {
                code: UnknownCode::ExternalMergeGroupSupportUnobserved,
                context: Some(context_name.clone()),
                workflow_path: None,
                reason: "The snapshot cannot prove that every configured external App publishes the required context on merge-group candidate SHAs.".to_owned(),
                evidence: ruleset_evidence(&rulesets),
            });
        }
        if !actions_can_satisfy {
            continue;
        }
        if !has_observed_evidence {
            continue;
        }
        if observed.github_actions_runs.is_empty() {
            unknowns.push(AuditUnknown {
                code: UnknownCode::WorkflowMappingUnavailable,
                context: Some(context_name.clone()),
                workflow_path: None,
                reason: "The required context could be satisfied by GitHub Actions, but no sampled Check Run could be joined to an Actions workflow run by check-suite ID.".to_owned(),
                evidence: observed.evidence.iter().cloned().collect(),
            });
            continue;
        }
        if !observed.github_actions_unmapped.is_empty() {
            unknowns.push(AuditUnknown {
                code: UnknownCode::WorkflowMappingUnavailable,
                context: Some(context_name.clone()),
                workflow_path: None,
                reason: "At least one matching GitHub Actions Check Run could not be joined to a workflow run by check-suite ID.".to_owned(),
                evidence: observed.github_actions_unmapped.iter().cloned().collect(),
            });
        }

        for (workflow_id, workflow_path) in &observed.github_actions_runs {
            let Some(workflow) = workflows_by_id.get(workflow_id) else {
                unknowns.push(AuditUnknown {
                    code: UnknownCode::WorkflowDefinitionUnavailable,
                    context: Some(context_name.clone()),
                    workflow_path: Some(workflow_path.clone()),
                    reason: "The mapped workflow is absent from the current workflow inventory."
                        .to_owned(),
                    evidence: observed.evidence.iter().cloned().collect(),
                });
                continue;
            };
            let Some(definition) = &workflow.definition else {
                unknowns.push(AuditUnknown {
                    code: UnknownCode::WorkflowDefinitionUnavailable,
                    context: Some(context_name.clone()),
                    workflow_path: Some(workflow.path.clone()),
                    reason: "The mapped workflow definition was not collected, so its triggers cannot be audited.".to_owned(),
                    evidence: vec![EvidenceRef {
                        kind: EvidenceKind::Workflow,
                        url: workflow.url.clone(),
                        description: format!("workflow {} at {}", workflow.name, workflow.path),
                    }],
                });
                continue;
            };
            if snapshot.branch_policy.merge_queue_rule && definition.merge_group.is_none() {
                findings.push(AuditFinding {
                    rule: FindingRule::MergeGroupTriggerMissing,
                    severity: FindingSeverity::High,
                    title: format!("Workflow {} cannot run for merge-group candidates", workflow.path),
                    explanation: format!(
                        "The default branch uses a merge queue and required context {context_name} maps to this workflow, but its current definition has no merge_group trigger."
                    ),
                    contexts: vec![context_name.clone()],
                    workflow_paths: vec![workflow.path.clone()],
                    rulesets: rulesets.clone(),
                    evidence: vec![EvidenceRef {
                        kind: EvidenceKind::Workflow,
                        url: workflow.url.clone(),
                        description: format!("current workflow definition for {}", workflow.path),
                    }],
                    remediation: "Add a merge_group trigger and verify the required context on a real queue candidate before enforcing it.".to_owned(),
                });
            }
            let filtered_triggers = [
                ("pull_request", definition.pull_request.as_ref()),
                (
                    "pull_request_target",
                    definition.pull_request_target.as_ref(),
                ),
                ("merge_group", definition.merge_group.as_ref()),
            ]
            .into_iter()
            .filter_map(|(name, trigger)| {
                trigger
                    .filter(|trigger| !trigger.paths.is_empty() || !trigger.paths_ignore.is_empty())
                    .map(|_| name)
            })
            .collect::<Vec<_>>();
            if !filtered_triggers.is_empty() {
                findings.push(AuditFinding {
                    rule: FindingRule::PathFilteredRequiredWorkflow,
                    severity: FindingSeverity::Medium,
                    title: format!("Required workflow {} has path-filtered triggers", workflow.path),
                    explanation: format!(
                        "Required context {context_name} maps to this workflow, whose {} trigger(s) use paths or paths-ignore. A skipped required workflow can leave a pull request or queue candidate waiting indefinitely.",
                        filtered_triggers.join(", ")
                    ),
                    contexts: vec![context_name.clone()],
                    workflow_paths: vec![workflow.path.clone()],
                    rulesets: rulesets.clone(),
                    evidence: vec![EvidenceRef {
                        kind: EvidenceKind::Workflow,
                        url: workflow.url.clone(),
                        description: format!("current workflow definition for {}", workflow.path),
                    }],
                    remediation: "Remove path filters from the required workflow trigger, or publish the same required context from an always-running aggregator job.".to_owned(),
                });
            }
        }
    }

    sort_findings(&mut findings);
    findings.dedup();
    sort_unknowns(&mut unknowns);
    unknowns.dedup();
    let high_findings = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::High)
        .count();
    let medium_findings = findings.len() - high_findings;
    let verdict = if !findings.is_empty() {
        AuditVerdict::ActionRequired
    } else if snapshot.collection.status == CollectionStatus::Partial || !unknowns.is_empty() {
        AuditVerdict::Inconclusive
    } else {
        AuditVerdict::NoObservedRisk
    };
    let provider_url = snapshot
        .repository
        .url
        .strip_suffix(&format!("/{}", snapshot.repository.name_with_owner))
        .expect("repository URL suffix was validated")
        .to_owned();
    Ok(MergeReadinessAudit {
        schema: MERGE_READINESS_AUDIT_SCHEMA.to_owned(),
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        generated_at: snapshot.captured_at.clone(),
        scope: AuditScope {
            provider_url,
            repository: snapshot.repository.name_with_owner.clone(),
            default_branch: snapshot.repository.default_branch.clone(),
            default_branch_head_sha: snapshot.repository.default_branch_head_sha.clone(),
            sampled_pull_requests: u64::try_from(snapshot.pull_requests.len())?,
        },
        collection: snapshot.collection.clone(),
        privacy: AuditPrivacy {
            repository_source_collected: false,
            workflow_definitions_collected: true,
            pull_request_text_collected: true,
            check_run_output_text_collected: true,
            commit_status_description_collected: true,
            review_text_collected: false,
            commit_messages_collected: true,
        },
        claim_boundary: AuditClaimBoundary {
            current_configuration_snapshot_supported: true,
            historical_configuration_at_merge_supported: false,
            merge_safety_supported: false,
            workflow_runtime_semantics_supported: false,
            cost_savings_supported: false,
        },
        summary: AuditSummary {
            verdict,
            findings: u64::try_from(findings.len())?,
            high_findings: u64::try_from(high_findings)?,
            medium_findings: u64::try_from(medium_findings)?,
            unknowns: u64::try_from(unknowns.len())?,
            required_checks: u64::try_from(snapshot.branch_policy.required_checks.len())?,
            sampled_pull_requests: u64::try_from(snapshot.pull_requests.len())?,
            fully_observed_pull_requests: u64::try_from(
                snapshot
                    .pull_requests
                    .iter()
                    .filter(|pull_request| pull_request.collection == CollectionStatus::Complete)
                    .count(),
            )?,
        },
        findings,
        unknowns,
    })
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if is_terminal_unsafe(character) {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('&', "&amp;")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn code(value: &str) -> String {
    format!("`{}`", safe_text(value).replace('`', "'"))
}

fn finding_rule(rule: FindingRule) -> &'static str {
    match rule {
        FindingRule::DefaultBranchUnprotected => "default_branch_unprotected",
        FindingRule::RequiredCheckSourceUnpinned => "required_check_source_unpinned",
        FindingRule::RequiredCheckSourceMismatch => "required_check_source_mismatch",
        FindingRule::RequiredCheckSourceAmbiguous => "required_check_source_ambiguous",
        FindingRule::MergeGroupTriggerMissing => "merge_group_trigger_missing",
        FindingRule::PathFilteredRequiredWorkflow => "path_filtered_required_workflow",
    }
}

fn unknown_code(code: UnknownCode) -> &'static str {
    match code {
        UnknownCode::CollectionIncomplete => "collection_incomplete",
        UnknownCode::RequiredCheckNeverObserved => "required_check_never_observed",
        UnknownCode::RequiredCheckMissingOnSampledHead => "required_check_missing_on_sampled_head",
        UnknownCode::RequiredCheckSourceUnresolved => "required_check_source_unresolved",
        UnknownCode::WorkflowMappingUnavailable => "workflow_mapping_unavailable",
        UnknownCode::WorkflowDefinitionUnavailable => "workflow_definition_unavailable",
        UnknownCode::ExternalMergeGroupSupportUnobserved => {
            "external_merge_group_support_unobserved"
        }
    }
}

fn collection_surface(surface: &CollectionSurface) -> &'static str {
    match surface {
        CollectionSurface::Repository => "repository",
        CollectionSurface::EffectiveRules => "effective_rules",
        CollectionSurface::Rulesets => "rulesets",
        CollectionSurface::Workflows => "workflows",
        CollectionSurface::WorkflowDefinitions => "workflow_definitions",
        CollectionSurface::PullRequests => "pull_requests",
        CollectionSurface::CheckRuns => "check_runs",
        CollectionSurface::CommitStatuses => "commit_statuses",
        CollectionSurface::WorkflowRuns => "workflow_runs",
    }
}

pub fn render_merge_readiness_markdown(report: &MergeReadinessAudit) -> String {
    let mut output = String::new();
    output.push_str("# StrataDiff Merge Readiness Audit\n\n");
    output.push_str(&format!(
        "- Repository: [{}](<{}>)\n",
        safe_text(&report.scope.repository),
        report.scope.provider_url.to_owned() + "/" + &report.scope.repository
    ));
    output.push_str(&format!(
        "- Default branch: {} at {}\n",
        code(&report.scope.default_branch),
        code(&report.scope.default_branch_head_sha)
    ));
    output.push_str(&format!(
        "- Collection: {}\n- Verdict: {}\n",
        code(match report.collection.status {
            CollectionStatus::Complete => "complete",
            CollectionStatus::Partial => "partial",
        }),
        code(match report.summary.verdict {
            AuditVerdict::ActionRequired => "action_required",
            AuditVerdict::NoObservedRisk => "no_observed_risk",
            AuditVerdict::Inconclusive => "inconclusive",
        })
    ));
    output.push_str(&format!(
        "- Findings: {} high, {} medium; unknowns: {}\n\n",
        report.summary.high_findings, report.summary.medium_findings, report.summary.unknowns
    ));
    output.push_str("This report describes a bounded current-configuration snapshot. It does not prove historical merge policy or code safety.\n\n");
    output.push_str("## Findings\n\n");
    if report.findings.is_empty() {
        output.push_str("No actionable risk was observed in the supported checks.\n");
    } else {
        for finding in &report.findings {
            let severity = match finding.severity {
                FindingSeverity::High => "HIGH",
                FindingSeverity::Medium => "MEDIUM",
            };
            output.push_str(&format!(
                "### {severity} · {} · {}\n\n{}\n\n",
                code(finding_rule(finding.rule)),
                safe_text(&finding.title),
                safe_text(&finding.explanation)
            ));
            if !finding.contexts.is_empty() {
                output.push_str(&format!(
                    "- Contexts: {}\n",
                    finding
                        .contexts
                        .iter()
                        .map(|value| code(value))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !finding.workflow_paths.is_empty() {
                output.push_str(&format!(
                    "- Workflows: {}\n",
                    finding
                        .workflow_paths
                        .iter()
                        .map(|value| code(value))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for evidence in &finding.evidence {
                output.push_str(&format!(
                    "- Evidence: [{}](<{}>) — {}\n",
                    code(match evidence.kind {
                        EvidenceKind::Repository => "repository",
                        EvidenceKind::Ruleset => "ruleset",
                        EvidenceKind::Workflow => "workflow",
                        EvidenceKind::PullRequest => "pull_request",
                        EvidenceKind::CheckRun => "check_run",
                        EvidenceKind::WorkflowRun => "workflow_run",
                        EvidenceKind::CommitStatus => "commit_status",
                    }),
                    evidence.url,
                    safe_text(&evidence.description)
                ));
            }
            output.push_str(&format!(
                "- Remediation: {}\n\n",
                safe_text(&finding.remediation)
            ));
        }
    }
    output.push_str("\n## Unknowns\n\n");
    if report.unknowns.is_empty() {
        output.push_str("No unsupported or incomplete observation was recorded.\n");
    } else {
        for unknown in &report.unknowns {
            output.push_str(&format!(
                "- {}: {}",
                code(unknown_code(unknown.code)),
                safe_text(&unknown.reason)
            ));
            if let Some(context) = &unknown.context {
                output.push_str(&format!(" Context: {}.", code(context)));
            }
            if let Some(workflow_path) = &unknown.workflow_path {
                output.push_str(&format!(" Workflow: {}.", code(workflow_path)));
            }
            output.push('\n');
        }
    }
    output.push_str("\n## Collection gaps\n\n");
    if report.collection.gaps.is_empty() {
        output.push_str("Collection completed within the configured bounds.\n");
    } else {
        for gap in &report.collection.gaps {
            output.push_str(&format!(
                "- {}: {}\n",
                code(collection_surface(&gap.surface)),
                safe_text(&gap.reason)
            ));
        }
    }
    output
}
