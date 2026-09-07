use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::doctor_workflow::{
    DoctorWorkflowTriggerDiagnosis, DoctorWorkflowTriggerInput, WorkflowExpectedApp, WorkflowJob,
    WorkflowTargetKind, WorkflowTriggerCause, WorkflowTriggers, classify_workflow_trigger,
};

pub const PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA: &str = "stratadiff-pull-request-doctor-snapshot-v1";
pub const PULL_REQUEST_DOCTOR_REPORT_SCHEMA: &str = "stratadiff-pull-request-doctor-v1";
pub const PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA: &str =
    "stratadiff-pull-request-doctor-snapshot-v2";
pub const PULL_REQUEST_DOCTOR_REPORT_V2_SCHEMA: &str = "stratadiff-pull-request-doctor-v2";
pub const PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA: &str =
    "stratadiff-pull-request-doctor-snapshot-v3";
pub const PULL_REQUEST_DOCTOR_REPORT_V3_SCHEMA: &str = "stratadiff-pull-request-doctor-v3";

const MAX_JSON_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_COLLECTION_ITEMS: usize = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCollectionStatus {
    Complete,
    Partial,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCollectionSurface {
    Target,
    Requirements,
    CheckRuns,
    CommitStatuses,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct DoctorCollectionGap {
    pub surface: DoctorCollectionSurface,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorCollection {
    pub status: DoctorCollectionStatus,
    pub api_calls: u64,
    pub response_bytes: u64,
    pub gaps: Vec<DoctorCollectionGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorTarget {
    pub number: u64,
    pub url: String,
    pub base_ref: String,
    pub base_sha: String,
    pub head_sha: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorEvaluationTargetKind {
    PrHead,
    TestMerge,
    MergeGroup,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorEvaluationTargetResolution {
    Selected,
    Provisional,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorEvaluationTarget {
    pub kind: DoctorEvaluationTargetKind,
    pub resolution: DoctorEvaluationTargetResolution,
    pub sha: String,
    pub base_sha: Option<String>,
    pub queue_entry_id: Option<String>,
    pub queue_state: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorTargetV2 {
    pub number: u64,
    pub url: String,
    pub base_ref: String,
    pub base_sha: String,
    pub head_sha: String,
    pub evaluation: DoctorEvaluationTarget,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorPolicyKind {
    Ruleset,
    BranchProtection,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct DoctorPolicyRef {
    pub kind: DoctorPolicyKind,
    pub id: String,
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorRequirement {
    pub context: String,
    pub expected_app_id: Option<u64>,
    pub policies: Vec<DoctorPolicyRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorCheckRun {
    pub id: u64,
    pub url: String,
    pub name: String,
    pub app_id: Option<u64>,
    pub app_slug: Option<String>,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorCommitStatus {
    pub id: u64,
    pub url: String,
    pub context: String,
    pub creator_id: Option<u64>,
    pub creator_login: Option<String>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorSnapshot {
    pub schema: String,
    pub captured_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTarget,
    pub collection: DoctorCollection,
    pub requirements: Vec<DoctorRequirement>,
    pub check_runs: Vec<DoctorCheckRun>,
    pub statuses: Vec<DoctorCommitStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorSnapshotV2 {
    pub schema: String,
    pub captured_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTargetV2,
    pub signal_sha: String,
    pub collection: DoctorCollection,
    pub requirements: Vec<DoctorRequirement>,
    pub check_runs: Vec<DoctorCheckRun>,
    pub statuses: Vec<DoctorCommitStatus>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DoctorWorkflowCollectionStatus {
    NotApplicable,
    Complete,
    Partial,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowCollectionGap {
    pub requirement: DoctorRequirementKey,
    pub code: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowInventoryFile {
    pub blob_sha: String,
    pub jobs: Vec<WorkflowJob>,
    pub path: String,
    pub triggers: WorkflowTriggers,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowInventory {
    pub sha: String,
    pub files: Vec<DoctorWorkflowInventoryFile>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorWorkflowProbeKind {
    PullRequestHead,
    TestMerge,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowProbe {
    pub kind: DoctorWorkflowProbeKind,
    pub sha: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowCollection {
    pub status: DoctorWorkflowCollectionStatus,
    pub api_calls: u64,
    pub response_bytes: u64,
    pub inventory: Option<DoctorWorkflowInventory>,
    pub probes: Vec<DoctorWorkflowProbe>,
    pub gaps: Vec<DoctorWorkflowCollectionGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowProducer {
    pub source_sha: String,
    pub check_run_id: u64,
    pub check_run_api_url: String,
    pub check_run_url: String,
    pub check_name: String,
    pub app_id: u64,
    pub app_slug: String,
    pub check_suite_id: u64,
    pub workflow_run_id: u64,
    pub workflow_run_attempt: u64,
    pub workflow_run_url: String,
    pub workflow_run_path: String,
    pub workflow_job_id: u64,
    pub workflow_job_url: String,
    pub workflow_job_name: String,
    pub workflow_job_check_run_url: String,
    pub workflow_id: u64,
    pub workflow_path: String,
    pub workflow_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowTriggerInvestigation {
    pub requirement: DoctorRequirementKey,
    pub producer: Option<DoctorWorkflowProducer>,
    pub input: Option<DoctorWorkflowTriggerInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorSnapshotV3 {
    pub schema: String,
    pub captured_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTargetV2,
    pub signal_sha: String,
    pub collection: DoctorCollection,
    pub requirements: Vec<DoctorRequirement>,
    pub check_runs: Vec<DoctorCheckRun>,
    pub statuses: Vec<DoctorCommitStatus>,
    pub workflow_collection: DoctorWorkflowCollection,
    pub workflow_trigger_investigations: Vec<DoctorWorkflowTriggerInvestigation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct DoctorRequirementKey {
    pub context: String,
    pub expected_app_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorRequirementStatus {
    Satisfied,
    Pending,
    Failed,
    Missing,
    SourceMismatch,
    SourceUnknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorEvidenceKind {
    CheckRun,
    CommitStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct DoctorEvidence {
    pub kind: DoctorEvidenceKind,
    pub id: String,
    pub url: String,
    pub sha: String,
    pub context: String,
    pub app_id: Option<u64>,
    pub producer: Option<String>,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorRequirementDiagnosis {
    pub key: DoctorRequirementKey,
    pub status: DoctorRequirementStatus,
    pub explanation: String,
    pub policies: Vec<DoctorPolicyRef>,
    pub evidence: Vec<DoctorEvidence>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorVerdict {
    ChecksClear,
    ChecksBlocked,
    Inconclusive,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorSummary {
    pub requirements: u64,
    pub satisfied: u64,
    pub pending: u64,
    pub failed: u64,
    pub missing: u64,
    pub source_mismatch: u64,
    pub source_unknown: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorClaimBoundary {
    pub required_check_readiness_supported: bool,
    pub mergeability_supported: bool,
    pub review_requirements_supported: bool,
    pub compliance_supported: bool,
    pub code_safety_supported: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoctorActionCode {
    WaitForCheck,
    InspectFailedCheck,
    RestoreRequiredCheck,
    FixRequiredCheckSource,
    ResolveSourceIdentity,
    CompleteCollection,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorNextAction {
    pub code: DoctorActionCode,
    pub requirement: Option<DoctorRequirementKey>,
    pub title: String,
    pub rationale: String,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorReport {
    pub schema: String,
    pub tool_version: String,
    pub generated_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTarget,
    pub collection: DoctorCollection,
    pub claim_boundary: DoctorClaimBoundary,
    pub verdict: DoctorVerdict,
    pub summary: DoctorSummary,
    pub requirements: Vec<DoctorRequirementDiagnosis>,
    pub next_actions: Vec<DoctorNextAction>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorReportV2 {
    pub schema: String,
    pub tool_version: String,
    pub generated_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTargetV2,
    pub collection: DoctorCollection,
    pub claim_boundary: DoctorClaimBoundary,
    pub verdict: DoctorVerdict,
    pub summary: DoctorSummary,
    pub requirements: Vec<DoctorRequirementDiagnosis>,
    pub next_actions: Vec<DoctorNextAction>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorWorkflowTriggerReport {
    pub requirement: DoctorRequirementKey,
    pub producer: Option<DoctorWorkflowProducer>,
    pub diagnosis: Option<DoctorWorkflowTriggerDiagnosis>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestDoctorReportV3 {
    pub schema: String,
    pub tool_version: String,
    pub generated_at: String,
    pub provider_url: String,
    pub repository: String,
    pub target: DoctorTargetV2,
    pub collection: DoctorCollection,
    pub claim_boundary: DoctorClaimBoundary,
    pub verdict: DoctorVerdict,
    pub summary: DoctorSummary,
    pub requirements: Vec<DoctorRequirementDiagnosis>,
    pub next_actions: Vec<DoctorNextAction>,
    pub workflow_collection: DoctorWorkflowCollection,
    pub workflow_trigger_diagnoses: Vec<DoctorWorkflowTriggerReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignalState {
    Satisfied,
    Pending,
    Failed,
    Unknown,
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

fn valid_workflow_path(value: &str) -> bool {
    value
        .strip_prefix(".github/workflows/")
        .is_some_and(|name| {
            !name.is_empty()
                && !name.contains('/')
                && !matches!(name, "." | "..")
                && (name.ends_with(".yml") || name.ends_with(".yaml"))
        })
}

fn valid_repository(value: &str) -> bool {
    let mut parts = value.split('/');
    let valid_component = |component: &str| {
        !component.is_empty()
            && !matches!(component, "." | "..")
            && component.len() <= 100
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    };
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(owner), Some(repository), None)
            if valid_component(owner) && valid_component(repository)
    )
}

fn valid_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[bytes.len() - 1] != b'Z'
    {
        return false;
    }
    if ![0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
        .into_iter()
        .all(|index| bytes[index].is_ascii_digit())
    {
        return false;
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
    bytes.len() == 20
        || (bytes[19] == b'.'
            && bytes.len() > 21
            && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit))
}

fn valid_url(value: &str) -> bool {
    value.starts_with("https://")
        && value.len() <= 2_048
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && !value.contains(['<', '>', '"', '\''])
}

fn provider_hostname(provider_url: &str) -> Option<&str> {
    let hostname = provider_url.strip_prefix("https://")?;
    if hostname.is_empty()
        || hostname.contains(['/', '?', '#', '@'])
        || hostname.starts_with('.')
        || hostname.ends_with('.')
    {
        return None;
    }
    Some(hostname)
}

fn validate_url(value: &str, label: &str) -> Result<()> {
    ensure!(valid_url(value), "{label} must be a canonical HTTPS URL");
    Ok(())
}

fn validate_snapshot(snapshot: &PullRequestDoctorSnapshot) -> Result<()> {
    ensure!(
        snapshot.schema == PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA,
        "unsupported pull-request doctor snapshot schema"
    );
    bounded_nonempty(&snapshot.captured_at, 64, "captured_at")?;
    ensure!(
        valid_utc_timestamp(&snapshot.captured_at),
        "captured_at must be an RFC 3339 UTC timestamp"
    );
    validate_url(&snapshot.provider_url, "provider URL")?;
    ensure!(
        provider_hostname(&snapshot.provider_url).is_some(),
        "provider URL must be an HTTPS origin without credentials, path, query, or fragment"
    );
    ensure!(
        valid_repository(&snapshot.repository),
        "repository must be OWNER/REPO"
    );
    ensure!(
        snapshot.target.number > 0 && snapshot.target.number <= MAX_JSON_INTEGER,
        "pull request number must be a positive JSON-safe integer"
    );
    validate_url(&snapshot.target.url, "pull request URL")?;
    ensure!(
        snapshot.target.url
            == format!(
                "{}/{}/pull/{}",
                snapshot.provider_url, snapshot.repository, snapshot.target.number
            ),
        "pull request URL must identify the target on the configured provider"
    );
    bounded_nonempty(&snapshot.target.base_ref, 255, "base ref")?;
    ensure!(
        valid_sha(&snapshot.target.base_sha),
        "base SHA must be a lowercase full Git object ID"
    );
    ensure!(
        valid_sha(&snapshot.target.head_sha),
        "head SHA must be a lowercase full Git object ID"
    );
    match snapshot.collection.status {
        DoctorCollectionStatus::Complete => ensure!(
            snapshot.collection.gaps.is_empty(),
            "a complete doctor snapshot cannot contain collection gaps"
        ),
        DoctorCollectionStatus::Partial => ensure!(
            !snapshot.collection.gaps.is_empty(),
            "a partial doctor snapshot must explain at least one collection gap"
        ),
    }
    ensure!(
        snapshot.collection.api_calls <= MAX_JSON_INTEGER,
        "API call count must be a JSON-safe integer"
    );
    ensure!(
        snapshot.collection.response_bytes <= MAX_JSON_INTEGER,
        "response byte count must be a JSON-safe integer"
    );
    ensure!(
        snapshot.collection.gaps.len() <= MAX_COLLECTION_ITEMS,
        "too many collection gaps"
    );
    for gap in &snapshot.collection.gaps {
        bounded_nonempty(&gap.reason, 4_096, "collection gap reason")?;
    }

    let mut policy_identities: BTreeMap<(DoctorPolicyKind, String), DoctorPolicyRef> =
        BTreeMap::new();
    ensure!(
        snapshot.requirements.len() <= MAX_COLLECTION_ITEMS,
        "too many required checks"
    );
    for requirement in &snapshot.requirements {
        bounded_nonempty(&requirement.context, 255, "required check context")?;
        if let Some(app_id) = requirement.expected_app_id {
            ensure!(
                app_id > 0 && app_id <= MAX_JSON_INTEGER,
                "expected App ID must be a positive JSON-safe integer"
            );
        }
        ensure!(
            !requirement.policies.is_empty(),
            "every required check must identify at least one enforcing policy"
        );
        ensure!(
            requirement.policies.len() <= MAX_COLLECTION_ITEMS,
            "too many policies on one required check"
        );
        for policy in &requirement.policies {
            bounded_nonempty(&policy.id, 255, "policy ID")?;
            bounded_nonempty(&policy.name, 255, "policy name")?;
            validate_url(&policy.url, "policy URL")?;
            let identity = (policy.kind, policy.id.clone());
            if let Some(previous) = policy_identities.insert(identity, policy.clone()) {
                ensure!(
                    previous == *policy,
                    "one policy identity cannot have conflicting metadata"
                );
            }
        }
    }
    ensure!(
        policy_identities.len() <= MAX_COLLECTION_ITEMS,
        "too many distinct enforcing policies"
    );

    let mut check_ids = BTreeSet::new();
    ensure!(
        snapshot
            .check_runs
            .len()
            .checked_add(snapshot.statuses.len())
            .is_some_and(|count| count <= MAX_COLLECTION_ITEMS),
        "too many exact-head signals"
    );
    for check in &snapshot.check_runs {
        ensure!(
            check.id > 0 && check.id <= MAX_JSON_INTEGER,
            "check-run ID must be a positive JSON-safe integer"
        );
        ensure!(check_ids.insert(check.id), "duplicate check-run ID");
        validate_url(&check.url, "check-run URL")?;
        bounded_nonempty(&check.name, 255, "check-run name")?;
        bounded_nonempty(&check.status, 64, "check-run status")?;
        if let Some(app_id) = check.app_id {
            ensure!(
                app_id > 0 && app_id <= MAX_JSON_INTEGER,
                "check-run App ID must be a positive JSON-safe integer"
            );
        }
        if let Some(app_slug) = &check.app_slug {
            bounded_nonempty(app_slug, 255, "check-run App slug")?;
        }
        if let Some(conclusion) = &check.conclusion {
            bounded_nonempty(conclusion, 64, "check-run conclusion")?;
        }
    }

    let mut status_ids = BTreeSet::new();
    for status in &snapshot.statuses {
        ensure!(
            status.id > 0 && status.id <= MAX_JSON_INTEGER,
            "commit-status ID must be a positive JSON-safe integer"
        );
        ensure!(status_ids.insert(status.id), "duplicate commit-status ID");
        validate_url(&status.url, "commit-status URL")?;
        bounded_nonempty(&status.context, 255, "commit-status context")?;
        bounded_nonempty(&status.state, 64, "commit-status state")?;
        if let Some(creator_id) = status.creator_id {
            ensure!(
                creator_id > 0 && creator_id <= MAX_JSON_INTEGER,
                "commit-status creator ID must be a positive JSON-safe integer"
            );
        }
        if let Some(creator_login) = &status.creator_login {
            bounded_nonempty(creator_login, 255, "commit-status creator login")?;
        }
    }
    Ok(())
}

fn legacy_snapshot(snapshot: &PullRequestDoctorSnapshotV2) -> PullRequestDoctorSnapshot {
    PullRequestDoctorSnapshot {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA.to_owned(),
        captured_at: snapshot.captured_at.clone(),
        provider_url: snapshot.provider_url.clone(),
        repository: snapshot.repository.clone(),
        target: DoctorTarget {
            number: snapshot.target.number,
            url: snapshot.target.url.clone(),
            base_ref: snapshot.target.base_ref.clone(),
            base_sha: snapshot.target.base_sha.clone(),
            head_sha: snapshot.target.head_sha.clone(),
        },
        collection: snapshot.collection.clone(),
        requirements: snapshot.requirements.clone(),
        check_runs: snapshot.check_runs.clone(),
        statuses: snapshot.statuses.clone(),
    }
}

fn validate_evaluation_target(target: &DoctorTargetV2) -> Result<()> {
    let evaluation = &target.evaluation;
    ensure!(
        valid_sha(&evaluation.sha),
        "evaluation SHA must be a lowercase full Git object ID"
    );
    if let Some(base_sha) = &evaluation.base_sha {
        ensure!(
            valid_sha(base_sha),
            "evaluation base SHA must be a lowercase full Git object ID"
        );
    }
    if let Some(queue_entry_id) = &evaluation.queue_entry_id {
        bounded_nonempty(queue_entry_id, 255, "merge-queue entry ID")?;
    }
    if let Some(queue_state) = &evaluation.queue_state {
        bounded_nonempty(queue_state, 64, "merge-queue state")?;
    }

    match evaluation.kind {
        DoctorEvaluationTargetKind::PrHead => ensure!(
            evaluation.sha == target.head_sha
                && evaluation.base_sha.is_none()
                && evaluation.queue_entry_id.is_none()
                && evaluation.queue_state.is_none(),
            "pr_head evaluation must use the PR head SHA without candidate metadata"
        ),
        DoctorEvaluationTargetKind::TestMerge => {
            ensure!(
                evaluation.sha != target.head_sha && evaluation.sha != target.base_sha,
                "test_merge evaluation SHA must be distinct from the PR head and base"
            );
            ensure!(
                evaluation.base_sha.as_deref() == Some(target.base_sha.as_str())
                    && evaluation.queue_entry_id.is_none()
                    && evaluation.queue_state.is_none(),
                "test_merge evaluation must identify the PR base without merge-queue metadata"
            );
        }
        DoctorEvaluationTargetKind::MergeGroup => {
            ensure!(
                evaluation.sha != target.head_sha
                    && evaluation
                        .base_sha
                        .as_deref()
                        .is_some_and(|base_sha| base_sha != evaluation.sha),
                "merge_group evaluation must identify a distinct candidate and base SHA"
            );
            ensure!(
                evaluation.queue_entry_id.is_some() && evaluation.queue_state.is_some(),
                "merge_group evaluation must identify its queue entry and state"
            );
        }
    }
    Ok(())
}

fn validate_snapshot_v2(snapshot: &PullRequestDoctorSnapshotV2) -> Result<()> {
    ensure!(
        snapshot.schema == PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA,
        "unsupported pull-request doctor v2 snapshot schema"
    );
    validate_snapshot(&legacy_snapshot(snapshot))?;
    validate_evaluation_target(&snapshot.target)?;
    ensure!(
        valid_sha(&snapshot.signal_sha),
        "signal SHA must be a lowercase full Git object ID"
    );
    ensure!(
        snapshot.signal_sha == snapshot.target.evaluation.sha,
        "signal SHA must match the evaluation SHA"
    );
    Ok(())
}

fn check_state(check: &DoctorCheckRun) -> SignalState {
    match check.status.as_str() {
        "queued" | "in_progress" | "requested" | "pending" | "waiting" => SignalState::Pending,
        "completed" => match check.conclusion.as_deref() {
            Some("success" | "neutral" | "skipped") => SignalState::Satisfied,
            Some(
                "failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure"
                | "stale",
            ) => SignalState::Failed,
            _ => SignalState::Unknown,
        },
        _ => SignalState::Unknown,
    }
}

fn commit_status_state(status: &DoctorCommitStatus) -> SignalState {
    match status.state.as_str() {
        "success" => SignalState::Satisfied,
        "pending" => SignalState::Pending,
        "error" | "failure" => SignalState::Failed,
        _ => SignalState::Unknown,
    }
}

fn aggregate_signal_states(states: impl Iterator<Item = SignalState>) -> SignalState {
    let mut saw_satisfied = false;
    let mut saw_pending = false;
    let mut saw_unknown = false;
    for state in states {
        match state {
            SignalState::Failed => return SignalState::Failed,
            SignalState::Pending => saw_pending = true,
            SignalState::Unknown => saw_unknown = true,
            SignalState::Satisfied => saw_satisfied = true,
        }
    }
    if saw_pending {
        SignalState::Pending
    } else if saw_unknown {
        SignalState::Unknown
    } else if saw_satisfied {
        SignalState::Satisfied
    } else {
        SignalState::Unknown
    }
}

fn check_evidence(signal_sha: &str, check: &DoctorCheckRun) -> DoctorEvidence {
    DoctorEvidence {
        kind: DoctorEvidenceKind::CheckRun,
        id: check.id.to_string(),
        url: check.url.clone(),
        sha: signal_sha.to_owned(),
        context: check.name.clone(),
        app_id: check.app_id,
        producer: check.app_slug.clone(),
        status: check.status.clone(),
        conclusion: check.conclusion.clone(),
    }
}

fn status_evidence(signal_sha: &str, status: &DoctorCommitStatus) -> DoctorEvidence {
    DoctorEvidence {
        kind: DoctorEvidenceKind::CommitStatus,
        id: status.id.to_string(),
        url: status.url.clone(),
        sha: signal_sha.to_owned(),
        context: status.context.clone(),
        app_id: None,
        producer: status.creator_login.clone(),
        status: status.state.clone(),
        conclusion: None,
    }
}

fn status_from_signal(state: SignalState) -> DoctorRequirementStatus {
    match state {
        SignalState::Satisfied => DoctorRequirementStatus::Satisfied,
        SignalState::Pending => DoctorRequirementStatus::Pending,
        SignalState::Failed => DoctorRequirementStatus::Failed,
        SignalState::Unknown => DoctorRequirementStatus::SourceUnknown,
    }
}

fn signal_collection_incomplete(snapshot: &PullRequestDoctorSnapshot) -> bool {
    snapshot.collection.gaps.iter().any(|gap| {
        matches!(
            gap.surface,
            DoctorCollectionSurface::CheckRuns | DoctorCollectionSurface::CommitStatuses
        )
    })
}

fn diagnose_requirement(
    snapshot: &PullRequestDoctorSnapshot,
    signal_sha: &str,
    key: DoctorRequirementKey,
    policies: Vec<DoctorPolicyRef>,
) -> DoctorRequirementDiagnosis {
    let signal_collection_incomplete = signal_collection_incomplete(snapshot);
    let matching_checks = snapshot
        .check_runs
        .iter()
        .filter(|check| check.name == key.context)
        .collect::<Vec<_>>();
    let matching_statuses = snapshot
        .statuses
        .iter()
        .filter(|status| status.context == key.context)
        .collect::<Vec<_>>();
    let mut evidence = matching_checks
        .iter()
        .map(|check| check_evidence(signal_sha, check))
        .chain(
            matching_statuses
                .iter()
                .map(|status| status_evidence(signal_sha, status)),
        )
        .collect::<Vec<_>>();
    evidence.sort();
    evidence.dedup();

    let (status, explanation) = match key.expected_app_id {
        Some(expected_app_id) => {
            let matching_expected_app = matching_checks
                .iter()
                .copied()
                .filter(|check| check.app_id == Some(expected_app_id))
                .collect::<Vec<_>>();
            if !matching_expected_app.is_empty() {
                let signal = aggregate_signal_states(
                    matching_expected_app
                        .iter()
                        .map(|check| check_state(check))
                        .chain(
                            matching_statuses
                                .iter()
                                .map(|status| commit_status_state(status)),
                        ),
                );
                let status = if signal == SignalState::Satisfied
                    && (signal_collection_incomplete || !matching_statuses.is_empty())
                {
                    DoctorRequirementStatus::SourceUnknown
                } else {
                    status_from_signal(signal)
                };
                let explanation = match status {
                    DoctorRequirementStatus::Satisfied => format!(
                        "The exact head has a successful {context} Check Run from expected App {expected_app_id}, and every same-name legacy status also succeeded.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::Pending => format!(
                        "The expected-App Check Run or a same-name legacy {context} status is still pending on the exact head.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::Failed => format!(
                        "The expected-App Check Run or a same-name legacy {context} status failed on the exact head.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::SourceUnknown => format!(
                        "The expected-App Check Run is present, but a same-name legacy {context} status has no provable GitHub App identity or collection is incomplete.",
                        context = key.context
                    ),
                    _ => unreachable!("signal states map only to observed statuses"),
                };
                (status, explanation)
            } else if signal_collection_incomplete {
                (
                    DoctorRequirementStatus::SourceUnknown,
                    format!(
                        "Collection is partial, so absence of a {context} Check Run from expected App {expected_app_id} is not proof that it is missing.",
                        context = key.context
                    ),
                )
            } else if matching_checks.iter().any(|check| check.app_id.is_none())
                || !matching_statuses.is_empty()
            {
                (
                    DoctorRequirementStatus::SourceUnknown,
                    format!(
                        "The exact head exposes {context}, but at least one matching signal cannot prove GitHub App identity {expected_app_id}; legacy statuses never satisfy an App-bound requirement conclusively.",
                        context = key.context
                    ),
                )
            } else if !matching_checks.is_empty() {
                (
                    DoctorRequirementStatus::SourceMismatch,
                    format!(
                        "The exact head exposes {context}, but no Check Run came from required App {expected_app_id}.",
                        context = key.context
                    ),
                )
            } else {
                (
                    DoctorRequirementStatus::Missing,
                    format!(
                        "Complete collection found no {context} signal on the exact head.",
                        context = key.context
                    ),
                )
            }
        }
        None => {
            if matching_checks.is_empty() && matching_statuses.is_empty() {
                if signal_collection_incomplete {
                    (
                        DoctorRequirementStatus::SourceUnknown,
                        format!(
                            "Collection is partial, so absence of {context} is not proof that it is missing.",
                            context = key.context
                        ),
                    )
                } else {
                    (
                        DoctorRequirementStatus::Missing,
                        format!(
                            "Complete collection found no {context} signal on the exact head.",
                            context = key.context
                        ),
                    )
                }
            } else {
                let distinct_app_count = matching_checks
                    .iter()
                    .filter_map(|check| check.app_id)
                    .collect::<BTreeSet<_>>()
                    .len();
                let signal = aggregate_signal_states(
                    matching_checks
                        .iter()
                        .map(|check| check_state(check))
                        .chain(
                            matching_statuses
                                .iter()
                                .map(|status| commit_status_state(status)),
                        ),
                );
                let status = if matches!(signal, SignalState::Failed | SignalState::Pending) {
                    status_from_signal(signal)
                } else if distinct_app_count > 1
                    || (signal == SignalState::Satisfied && signal_collection_incomplete)
                {
                    DoctorRequirementStatus::SourceUnknown
                } else {
                    status_from_signal(signal)
                };
                let explanation = match status {
                    DoctorRequirementStatus::Satisfied => format!(
                        "The exact head has a successful {context} signal.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::Pending => format!(
                        "The exact head has a pending {context} signal.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::Failed => format!(
                        "At least one {context} signal on the exact head failed; conflicting legacy and Check Run signals are treated conservatively.",
                        context = key.context
                    ),
                    DoctorRequirementStatus::SourceUnknown => format!(
                        "The effective {context} producer is ambiguous, or a matching signal has an unsupported or incomplete state.",
                        context = key.context
                    ),
                    _ => unreachable!("signal states map only to observed statuses"),
                };
                (status, explanation)
            }
        }
    };

    DoctorRequirementDiagnosis {
        key,
        status,
        explanation,
        policies,
        evidence,
    }
}

fn status_count(
    diagnoses: &[DoctorRequirementDiagnosis],
    status: DoctorRequirementStatus,
) -> Result<u64> {
    u64::try_from(
        diagnoses
            .iter()
            .filter(|diagnosis| diagnosis.status == status)
            .count(),
    )
    .context("doctor requirement count exceeds u64")
}

fn checks_argv_for_sha(provider_url: &str, repository: &str, evaluation_sha: &str) -> Vec<String> {
    let hostname = provider_hostname(provider_url)
        .expect("the provider URL was validated before actions are built");
    vec![
        "gh".to_owned(),
        "api".to_owned(),
        "--hostname".to_owned(),
        hostname.to_owned(),
        format!(
            "repos/{}/commits/{}/check-runs?filter=latest&per_page=100",
            repository, evaluation_sha
        ),
    ]
}

fn checks_argv(snapshot: &PullRequestDoctorSnapshot) -> Vec<String> {
    checks_argv_for_sha(
        &snapshot.provider_url,
        &snapshot.repository,
        &snapshot.target.head_sha,
    )
}

fn doctor_argv(snapshot: &PullRequestDoctorSnapshot) -> Vec<String> {
    vec![
        "stratadiff".to_owned(),
        "doctor".to_owned(),
        snapshot.target.url.clone(),
        "--format".to_owned(),
        "json".to_owned(),
    ]
}

fn action_for(
    snapshot: &PullRequestDoctorSnapshot,
    diagnosis: &DoctorRequirementDiagnosis,
) -> Option<DoctorNextAction> {
    let (code, title, rationale) = match diagnosis.status {
        DoctorRequirementStatus::Satisfied => return None,
        DoctorRequirementStatus::Pending => (
            DoctorActionCode::WaitForCheck,
            format!("Wait for {} on the exact head", diagnosis.key.context),
            "The required producer has published the check, but it has not reached a terminal state."
                .to_owned(),
        ),
        DoctorRequirementStatus::Failed => (
            DoctorActionCode::InspectFailedCheck,
            format!("Inspect the failed {} signal", diagnosis.key.context),
            "The required signal is terminal and unsuccessful on the exact head.".to_owned(),
        ),
        DoctorRequirementStatus::Missing => (
            DoctorActionCode::RestoreRequiredCheck,
            format!("Restore the missing {} signal", diagnosis.key.context),
            "Complete collection found no matching signal; inspect the workflow trigger and producer configuration."
                .to_owned(),
        ),
        DoctorRequirementStatus::SourceMismatch => (
            DoctorActionCode::FixRequiredCheckSource,
            format!("Repair the producer binding for {}", diagnosis.key.context),
            "A same-name signal exists, but it came from a different GitHub App than the enforcing policy requires."
                .to_owned(),
        ),
        DoctorRequirementStatus::SourceUnknown => (
            DoctorActionCode::ResolveSourceIdentity,
            format!("Resolve the producer identity for {}", diagnosis.key.context),
            "The available evidence cannot prove whether the configured producer satisfied this requirement."
                .to_owned(),
        ),
    };
    Some(DoctorNextAction {
        code,
        requirement: Some(diagnosis.key.clone()),
        title,
        rationale,
        argv: checks_argv(snapshot),
    })
}

fn evaluate_pull_request_doctor_with_signal_sha(
    snapshot: &PullRequestDoctorSnapshot,
    signal_sha: &str,
) -> Result<PullRequestDoctorReport> {
    validate_snapshot(snapshot)?;

    let mut requirements: BTreeMap<DoctorRequirementKey, BTreeSet<DoctorPolicyRef>> =
        BTreeMap::new();
    for requirement in &snapshot.requirements {
        requirements
            .entry(DoctorRequirementKey {
                context: requirement.context.clone(),
                expected_app_id: requirement.expected_app_id,
            })
            .or_default()
            .extend(requirement.policies.iter().cloned());
    }
    let diagnoses = requirements
        .into_iter()
        .map(|(key, policies)| {
            diagnose_requirement(snapshot, signal_sha, key, policies.into_iter().collect())
        })
        .collect::<Vec<_>>();

    let summary = DoctorSummary {
        requirements: u64::try_from(diagnoses.len())
            .context("doctor requirement count exceeds u64")?,
        satisfied: status_count(&diagnoses, DoctorRequirementStatus::Satisfied)?,
        pending: status_count(&diagnoses, DoctorRequirementStatus::Pending)?,
        failed: status_count(&diagnoses, DoctorRequirementStatus::Failed)?,
        missing: status_count(&diagnoses, DoctorRequirementStatus::Missing)?,
        source_mismatch: status_count(&diagnoses, DoctorRequirementStatus::SourceMismatch)?,
        source_unknown: status_count(&diagnoses, DoctorRequirementStatus::SourceUnknown)?,
    };
    let has_blocker = diagnoses.iter().any(|diagnosis| {
        matches!(
            diagnosis.status,
            DoctorRequirementStatus::Pending
                | DoctorRequirementStatus::Failed
                | DoctorRequirementStatus::Missing
                | DoctorRequirementStatus::SourceMismatch
        )
    });
    let target_inconclusive = snapshot
        .collection
        .gaps
        .iter()
        .any(|gap| gap.surface == DoctorCollectionSurface::Target);
    let verdict = if target_inconclusive {
        DoctorVerdict::Inconclusive
    } else if has_blocker {
        DoctorVerdict::ChecksBlocked
    } else if snapshot.collection.status == DoctorCollectionStatus::Partial
        || summary.source_unknown > 0
    {
        DoctorVerdict::Inconclusive
    } else {
        DoctorVerdict::ChecksClear
    };

    let mut next_actions = if target_inconclusive {
        Vec::new()
    } else {
        diagnoses
            .iter()
            .filter_map(|diagnosis| action_for(snapshot, diagnosis))
            .collect::<Vec<_>>()
    };
    if target_inconclusive {
        next_actions.push(DoctorNextAction {
            code: DoctorActionCode::CompleteCollection,
            requirement: None,
            title: "Resolve the active GitHub check target before acting".to_owned(),
            rationale: "Doctor could not prove that the PR head is GitHub's active required-check target; retry after the target stabilizes or use candidate-aware diagnostics."
                .to_owned(),
            argv: doctor_argv(snapshot),
        });
    } else if snapshot.collection.status == DoctorCollectionStatus::Partial {
        next_actions.push(DoctorNextAction {
            code: DoctorActionCode::CompleteCollection,
            requirement: None,
            title: "Repeat the diagnosis with complete GitHub visibility".to_owned(),
            rationale: "One or more required GitHub surfaces were not collected completely; restore access and repeat the exact-head query."
                .to_owned(),
            argv: doctor_argv(snapshot),
        });
    }

    Ok(PullRequestDoctorReport {
        schema: PULL_REQUEST_DOCTOR_REPORT_SCHEMA.to_owned(),
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        generated_at: snapshot.captured_at.clone(),
        provider_url: snapshot.provider_url.clone(),
        repository: snapshot.repository.clone(),
        target: snapshot.target.clone(),
        collection: snapshot.collection.clone(),
        claim_boundary: DoctorClaimBoundary {
            required_check_readiness_supported: !target_inconclusive,
            mergeability_supported: false,
            review_requirements_supported: false,
            compliance_supported: false,
            code_safety_supported: false,
        },
        verdict,
        summary,
        requirements: diagnoses,
        next_actions,
    })
}

pub fn evaluate_pull_request_doctor(
    snapshot: &PullRequestDoctorSnapshot,
) -> Result<PullRequestDoctorReport> {
    evaluate_pull_request_doctor_with_signal_sha(snapshot, &snapshot.target.head_sha)
}

pub fn evaluate_pull_request_doctor_v2(
    snapshot: &PullRequestDoctorSnapshotV2,
) -> Result<PullRequestDoctorReportV2> {
    validate_snapshot_v2(snapshot)?;
    let legacy = legacy_snapshot(snapshot);
    let report = evaluate_pull_request_doctor_with_signal_sha(&legacy, &snapshot.signal_sha)?;
    let target_selected =
        snapshot.target.evaluation.resolution == DoctorEvaluationTargetResolution::Selected;
    let mut requirements = report.requirements;
    for diagnosis in &mut requirements {
        diagnosis.explanation = diagnosis
            .explanation
            .replace("exact head", "exact evaluation target");
    }
    let mut next_actions = report.next_actions;
    for action in &mut next_actions {
        action.title = action
            .title
            .replace("exact head", "exact evaluation target");
        action.rationale = action
            .rationale
            .replace("exact head", "exact evaluation target")
            .replace("PR head", "declared evaluation target")
            .replace("exact-head", "evaluation-target");
        if action.requirement.is_some() {
            action.argv = checks_argv_for_sha(
                &snapshot.provider_url,
                &snapshot.repository,
                &snapshot.signal_sha,
            );
        }
    }
    if !target_selected {
        next_actions = vec![DoctorNextAction {
            code: DoctorActionCode::CompleteCollection,
            requirement: None,
            title: "Resolve the provisional evaluation target".to_owned(),
            rationale: "Doctor observed signals on this SHA but did not prove that GitHub selected it as the active required-check target. Refresh candidate evidence before acting on the diagnosis."
                .to_owned(),
            argv: doctor_argv(&legacy),
        }];
    }

    let mut claim_boundary = report.claim_boundary;
    claim_boundary.required_check_readiness_supported = target_selected
        && snapshot.collection.status == DoctorCollectionStatus::Complete
        && claim_boundary.required_check_readiness_supported;
    let verdict = if target_selected {
        report.verdict
    } else {
        DoctorVerdict::Inconclusive
    };

    Ok(PullRequestDoctorReportV2 {
        schema: PULL_REQUEST_DOCTOR_REPORT_V2_SCHEMA.to_owned(),
        tool_version: report.tool_version,
        generated_at: report.generated_at,
        provider_url: report.provider_url,
        repository: report.repository,
        target: snapshot.target.clone(),
        collection: report.collection,
        claim_boundary,
        verdict,
        summary: report.summary,
        requirements,
        next_actions,
    })
}

fn snapshot_v2_from_v3(snapshot: &PullRequestDoctorSnapshotV3) -> PullRequestDoctorSnapshotV2 {
    PullRequestDoctorSnapshotV2 {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA.to_owned(),
        captured_at: snapshot.captured_at.clone(),
        provider_url: snapshot.provider_url.clone(),
        repository: snapshot.repository.clone(),
        target: snapshot.target.clone(),
        signal_sha: snapshot.signal_sha.clone(),
        collection: snapshot.collection.clone(),
        requirements: snapshot.requirements.clone(),
        check_runs: snapshot.check_runs.clone(),
        statuses: snapshot.statuses.clone(),
    }
}

fn validate_workflow_collection(
    snapshot: &PullRequestDoctorSnapshotV3,
    report: &PullRequestDoctorReportV2,
) -> Result<()> {
    ensure!(
        snapshot.schema == PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA,
        "unsupported pull-request doctor v3 snapshot schema"
    );
    ensure!(
        snapshot.workflow_collection.api_calls <= MAX_JSON_INTEGER,
        "workflow API call count must be a JSON-safe integer"
    );
    ensure!(
        snapshot.workflow_collection.response_bytes <= MAX_JSON_INTEGER,
        "workflow response byte count must be a JSON-safe integer"
    );
    ensure!(
        snapshot.workflow_collection.api_calls <= snapshot.collection.api_calls
            && snapshot.workflow_collection.response_bytes <= snapshot.collection.response_bytes,
        "workflow collection budget must be a subset of the cumulative doctor budget"
    );
    ensure!(
        snapshot.workflow_collection.gaps.len() <= MAX_COLLECTION_ITEMS,
        "too many workflow collection gaps"
    );
    ensure!(
        snapshot.workflow_trigger_investigations.len() <= MAX_COLLECTION_ITEMS,
        "too many workflow trigger investigations"
    );
    let mut inventory_files = BTreeMap::new();
    if let Some(inventory) = &snapshot.workflow_collection.inventory {
        ensure!(
            inventory.sha == snapshot.signal_sha && valid_sha(&inventory.sha),
            "workflow inventory must be bound to the exact evaluation SHA"
        );
        ensure!(
            inventory.files.len() <= MAX_COLLECTION_ITEMS,
            "too many workflow inventory files"
        );
        let mut previous_path = None;
        for file in &inventory.files {
            ensure!(
                valid_sha(&file.blob_sha),
                "workflow inventory blob SHA must be a lowercase full Git object ID"
            );
            ensure!(
                valid_workflow_path(&file.path),
                "workflow inventory path is not a direct workflow file"
            );
            if let Some(previous) = previous_path {
                ensure!(
                    previous < file.path.as_str(),
                    "workflow inventory files must be unique and sorted"
                );
            }
            previous_path = Some(file.path.as_str());
            ensure!(
                file.jobs.len() <= MAX_COLLECTION_ITEMS,
                "too many jobs in a workflow inventory file"
            );
            let mut job_ids = BTreeSet::new();
            for job in &file.jobs {
                bounded_nonempty(&job.id, 255, "workflow inventory job ID")?;
                bounded_nonempty(&job.name, 255, "workflow inventory job name")?;
                bounded_nonempty(&job.condition, 255, "workflow inventory job condition")?;
                ensure!(
                    job_ids.insert(job.id.as_str()),
                    "workflow inventory job IDs must be unique per file"
                );
            }
            ensure!(
                inventory_files.insert(file.path.as_str(), file).is_none(),
                "duplicate workflow inventory path"
            );
        }
    }
    match snapshot.workflow_collection.status {
        DoctorWorkflowCollectionStatus::NotApplicable => ensure!(
            snapshot.workflow_collection.api_calls == 0
                && snapshot.workflow_collection.response_bytes == 0
                && snapshot.workflow_collection.inventory.is_none()
                && snapshot.workflow_collection.probes.is_empty()
                && snapshot.workflow_collection.gaps.is_empty()
                && snapshot.workflow_trigger_investigations.is_empty(),
            "a not-applicable workflow collection cannot contain observations"
        ),
        DoctorWorkflowCollectionStatus::Complete => ensure!(
            snapshot.workflow_collection.api_calls > 0
                && snapshot.workflow_collection.inventory.is_some()
                && snapshot.workflow_collection.gaps.is_empty()
                && !snapshot.workflow_trigger_investigations.is_empty()
                && snapshot
                    .workflow_trigger_investigations
                    .iter()
                    .all(|investigation| investigation.input.is_some()),
            "a complete workflow collection requires classified inputs without gaps"
        ),
        DoctorWorkflowCollectionStatus::Partial => ensure!(
            !snapshot.workflow_collection.gaps.is_empty()
                && !snapshot.workflow_trigger_investigations.is_empty(),
            "a partial workflow collection must contain investigations and gaps"
        ),
    }

    let requirement_statuses = report
        .requirements
        .iter()
        .map(|diagnosis| (diagnosis.key.clone(), diagnosis.status))
        .collect::<BTreeMap<_, _>>();
    let eligible_keys = if snapshot.target.evaluation.kind == DoctorEvaluationTargetKind::MergeGroup
    {
        report
            .requirements
            .iter()
            .filter(|diagnosis| {
                diagnosis.status == DoctorRequirementStatus::Missing
                    && diagnosis.key.expected_app_id.is_some()
            })
            .map(|diagnosis| diagnosis.key.clone())
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    let mut probed_source_shas = BTreeSet::new();
    let mut probe_kinds = BTreeSet::new();
    for probe in &snapshot.workflow_collection.probes {
        ensure!(
            valid_sha(&probe.sha) && probe.sha != snapshot.signal_sha,
            "workflow producer probe SHA must be a distinct lowercase full Git object ID"
        );
        ensure!(
            probed_source_shas.insert(probe.sha.as_str()),
            "duplicate workflow producer probe SHA"
        );
        ensure!(
            probe_kinds.insert(probe.kind),
            "duplicate workflow producer probe kind"
        );
        match probe.kind {
            DoctorWorkflowProbeKind::PullRequestHead => ensure!(
                probe.sha == snapshot.target.head_sha,
                "pull-request-head producer probe must use the declared head SHA"
            ),
            DoctorWorkflowProbeKind::TestMerge => ensure!(
                probe.sha != snapshot.target.head_sha
                    && probe.sha != snapshot.target.base_sha
                    && probe.sha != snapshot.signal_sha,
                "test-merge producer probe must identify a distinct candidate"
            ),
        }
    }
    if !eligible_keys.is_empty()
        && snapshot.workflow_collection.status != DoctorWorkflowCollectionStatus::Partial
    {
        ensure!(
            probe_kinds.contains(&DoctorWorkflowProbeKind::PullRequestHead),
            "complete workflow collection must probe the pull-request head"
        );
    }
    let mut investigation_keys = BTreeSet::new();
    for investigation in &snapshot.workflow_trigger_investigations {
        ensure!(
            investigation_keys.insert(investigation.requirement.clone()),
            "duplicate workflow trigger investigation"
        );
        ensure!(
            investigation.requirement.expected_app_id.is_some(),
            "workflow trigger investigation must be pinned to an App"
        );
        ensure!(
            requirement_statuses.get(&investigation.requirement)
                == Some(&DoctorRequirementStatus::Missing),
            "workflow trigger investigation must identify a missing required check"
        );
        ensure!(
            snapshot.target.evaluation.kind == DoctorEvaluationTargetKind::MergeGroup,
            "workflow trigger investigation currently supports only merge-group targets"
        );

        if let Some(producer) = &investigation.producer {
            ensure!(
                valid_sha(&producer.source_sha),
                "workflow producer source SHA must be a lowercase full Git object ID"
            );
            ensure!(
                probed_source_shas.contains(producer.source_sha.as_str()),
                "workflow producer evidence must come from a declared probe SHA"
            );
            ensure!(
                producer.check_run_id > 0
                    && producer.check_run_id <= MAX_JSON_INTEGER
                    && producer.check_suite_id > 0
                    && producer.check_suite_id <= MAX_JSON_INTEGER
                    && producer.workflow_run_id > 0
                    && producer.workflow_run_id <= MAX_JSON_INTEGER
                    && producer.workflow_run_attempt > 0
                    && producer.workflow_run_attempt <= MAX_JSON_INTEGER
                    && producer.workflow_job_id > 0
                    && producer.workflow_job_id <= MAX_JSON_INTEGER
                    && producer.workflow_id > 0
                    && producer.workflow_id <= MAX_JSON_INTEGER,
                "workflow producer IDs must be positive JSON-safe integers"
            );
            validate_url(
                &producer.check_run_api_url,
                "workflow producer check-run API URL",
            )?;
            validate_url(&producer.check_run_url, "workflow producer check-run URL")?;
            validate_url(&producer.workflow_run_url, "workflow producer run URL")?;
            validate_url(&producer.workflow_job_url, "workflow producer job URL")?;
            validate_url(
                &producer.workflow_job_check_run_url,
                "workflow producer job check-run URL",
            )?;
            validate_url(&producer.workflow_url, "workflow producer URL")?;
            bounded_nonempty(&producer.check_name, 255, "workflow producer check name")?;
            bounded_nonempty(
                &producer.workflow_job_name,
                255,
                "workflow producer job name",
            )?;
            bounded_nonempty(&producer.app_slug, 255, "workflow producer App slug")?;
            bounded_nonempty(
                &producer.workflow_run_path,
                1_024,
                "workflow producer run path",
            )?;
            ensure!(
                producer.check_name == investigation.requirement.context
                    && producer.workflow_job_name == investigation.requirement.context
                    && producer.workflow_job_check_run_url == producer.check_run_api_url
                    && producer.app_id
                        == investigation
                            .requirement
                            .expected_app_id
                            .context("workflow investigation is missing its expected App ID")?
                    && producer.app_slug == "github-actions",
                "workflow producer must match the required context and GitHub Actions App"
            );
            bounded_nonempty(&producer.workflow_path, 1_024, "workflow producer path")?;
            ensure!(
                valid_workflow_path(&producer.workflow_path),
                "workflow producer path must identify a GitHub Actions workflow"
            );
            ensure!(
                producer.workflow_run_path == producer.workflow_path
                    || producer
                        .workflow_run_path
                        .strip_prefix(&producer.workflow_path)
                        .is_some_and(|suffix| suffix.starts_with('@') && suffix.len() > 1),
                "workflow producer run path must resolve to its canonical workflow path"
            );
        }

        let has_keyed_gap = snapshot
            .workflow_collection
            .gaps
            .iter()
            .any(|gap| gap.requirement == investigation.requirement);
        if let Some(input) = &investigation.input {
            ensure!(
                !has_keyed_gap,
                "a classified workflow investigation cannot retain a blocking evidence gap"
            );
            ensure!(
                investigation.producer.is_some(),
                "classified workflow input must have an exact producer binding"
            );
            ensure!(
                input.expected_app == WorkflowExpectedApp::GithubActions
                    && input.required_context == investigation.requirement.context
                    && input.target.kind == WorkflowTargetKind::MergeGroup
                    && input.target.sha == snapshot.signal_sha
                    && input.pull_request.number == snapshot.target.number
                    && input.pull_request.base_ref == snapshot.target.base_ref
                    && input.pull_request.base_sha == snapshot.target.base_sha
                    && input.pull_request.head_sha == snapshot.target.head_sha,
                "workflow classifier input is not bound to its required check and pull request"
            );
            ensure!(
                !input.changed_files.complete
                    && !input.changed_files.github_filter_file_limit_reached
                    && input.changed_files.paths.is_empty()
                    && input.changed_files.total == 0
                    && input.collection_gaps.is_empty()
                    && input.historical_check_names
                        == vec![investigation.requirement.context.clone()]
                    && input.last_activity == "not_applicable"
                    && input.provider_capability
                        == crate::doctor_workflow::WorkflowProviderCapability::NotApplicable
                    && input.pull_request.mergeable_state
                        == crate::doctor_workflow::WorkflowMergeableState::Unknown,
                "live merge-group classifier input contains unsupported phase-one evidence"
            );
            let producer = investigation
                .producer
                .as_ref()
                .context("classified workflow input is missing its producer")?;
            ensure!(
                input.workflows.len() == 1
                    && input.workflows[0].path == producer.workflow_path
                    && input.workflows[0].state == crate::doctor_workflow::WorkflowState::Active
                    && input.workflows[0].syntax == crate::doctor_workflow::WorkflowSyntax::Valid
                    && input.workflows[0]
                        .jobs
                        .iter()
                        .all(|job| job.name_static && !job.reusable)
                    && input.workflows[0]
                        .jobs
                        .iter()
                        .filter(|job| job.name == investigation.requirement.context)
                        .count()
                        == 1
                    && input.runs.iter().all(|run| {
                        run.head_sha == snapshot.signal_sha
                            && run.workflow_path == producer.workflow_path
                    }),
                "workflow classifier input must contain exactly its bound producer"
            );
            let inventory = snapshot
                .workflow_collection
                .inventory
                .as_ref()
                .context("classified workflow input is missing its exact-SHA inventory")?;
            let inventory_file = inventory_files
                .get(producer.workflow_path.as_str())
                .context("bound producer is absent from the exact-SHA workflow inventory")?;
            ensure!(
                inventory_file.jobs == input.workflows[0].jobs
                    && inventory_file.triggers == input.workflows[0].triggers,
                "bound producer definition differs from the exact-SHA workflow inventory"
            );
            ensure!(
                inventory
                    .files
                    .iter()
                    .all(|file| file.jobs.iter().all(|job| job.name_static && !job.reusable)),
                "classified workflow input requires a fully resolved producer inventory"
            );
            let matching_inventory_jobs = inventory
                .files
                .iter()
                .flat_map(|file| {
                    file.jobs.iter().filter_map(move |job| {
                        (job.name == investigation.requirement.context)
                            .then_some((file.path.as_str(), job.id.as_str()))
                    })
                })
                .collect::<Vec<_>>();
            ensure!(
                matching_inventory_jobs.len() == 1
                    && matching_inventory_jobs[0].0 == producer.workflow_path,
                "classified workflow producer is not unique in the exact-SHA inventory"
            );
            classify_workflow_trigger(input)?;
        } else {
            ensure!(
                has_keyed_gap,
                "an unclassified workflow investigation must identify its evidence gap"
            );
        }
    }
    ensure!(
        investigation_keys == eligible_keys,
        "workflow investigations must cover every pinned missing merge-group requirement exactly once"
    );

    let mut gap_identities = BTreeSet::new();
    for gap in &snapshot.workflow_collection.gaps {
        ensure!(
            investigation_keys.contains(&gap.requirement),
            "workflow collection gap does not identify an investigation"
        );
        bounded_nonempty(&gap.code, 128, "workflow collection gap code")?;
        ensure!(
            gap.code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "workflow collection gap code must use lowercase snake case"
        );
        bounded_nonempty(&gap.reason, 4_096, "workflow collection gap reason")?;
        ensure!(
            gap_identities.insert((
                gap.requirement.clone(),
                gap.code.clone(),
                gap.reason.clone()
            )),
            "duplicate workflow collection gap"
        );
    }
    Ok(())
}

pub fn evaluate_pull_request_doctor_v3(
    snapshot: &PullRequestDoctorSnapshotV3,
) -> Result<PullRequestDoctorReportV3> {
    let report = evaluate_pull_request_doctor_v2(&snapshot_v2_from_v3(snapshot))?;
    validate_workflow_collection(snapshot, &report)?;
    let workflow_trigger_diagnoses = snapshot
        .workflow_trigger_investigations
        .iter()
        .map(|investigation| {
            let diagnosis = investigation
                .input
                .as_ref()
                .map(classify_workflow_trigger)
                .transpose()?;
            ensure!(
                diagnosis.as_ref().is_none_or(|diagnosis| matches!(
                    diagnosis.cause_code,
                    WorkflowTriggerCause::ForkApprovalRequired
                        | WorkflowTriggerCause::MergeGroupTriggerMissing
                        | WorkflowTriggerCause::None
                        | WorkflowTriggerCause::WorkflowTriggerUnknown
                )),
                "live workflow diagnosis produced a cause outside the v3 evidence contract"
            );
            Ok(DoctorWorkflowTriggerReport {
                requirement: investigation.requirement.clone(),
                producer: investigation.producer.clone(),
                diagnosis,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(PullRequestDoctorReportV3 {
        schema: PULL_REQUEST_DOCTOR_REPORT_V3_SCHEMA.to_owned(),
        tool_version: report.tool_version,
        generated_at: report.generated_at,
        provider_url: report.provider_url,
        repository: report.repository,
        target: report.target,
        collection: report.collection,
        claim_boundary: report.claim_boundary,
        verdict: report.verdict,
        summary: report.summary,
        requirements: report.requirements,
        next_actions: report.next_actions,
        workflow_collection: snapshot.workflow_collection.clone(),
        workflow_trigger_diagnoses,
    })
}

fn markdown_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '`' => escaped.push_str("&#96;"),
            '|' => escaped.push_str("&#124;"),
            '\\' => escaped.push_str("&#92;"),
            '*' => escaped.push_str("&#42;"),
            '_' => escaped.push_str("&#95;"),
            '[' => escaped.push_str("&#91;"),
            ']' => escaped.push_str("&#93;"),
            '(' => escaped.push_str("&#40;"),
            ')' => escaped.push_str("&#41;"),
            '#' => escaped.push_str("&#35;"),
            '!' => escaped.push_str("&#33;"),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            character if character.is_control() => escaped.extend(character.escape_unicode()),
            character => escaped.push(character),
        }
    }
    escaped
}

fn markdown_code(value: &str) -> String {
    format!("<code>{}</code>", markdown_text(value))
}

fn requirement_status(status: DoctorRequirementStatus) -> &'static str {
    match status {
        DoctorRequirementStatus::Satisfied => "satisfied",
        DoctorRequirementStatus::Pending => "pending",
        DoctorRequirementStatus::Failed => "failed",
        DoctorRequirementStatus::Missing => "missing",
        DoctorRequirementStatus::SourceMismatch => "source_mismatch",
        DoctorRequirementStatus::SourceUnknown => "source_unknown",
    }
}

fn verdict_name(verdict: DoctorVerdict) -> &'static str {
    match verdict {
        DoctorVerdict::ChecksClear => "checks_clear",
        DoctorVerdict::ChecksBlocked => "checks_blocked",
        DoctorVerdict::Inconclusive => "inconclusive",
    }
}

fn policy_kind(kind: DoctorPolicyKind) -> &'static str {
    match kind {
        DoctorPolicyKind::Ruleset => "ruleset",
        DoctorPolicyKind::BranchProtection => "branch_protection",
    }
}

fn evidence_kind(kind: DoctorEvidenceKind) -> &'static str {
    match kind {
        DoctorEvidenceKind::CheckRun => "check_run",
        DoctorEvidenceKind::CommitStatus => "commit_status",
    }
}

fn collection_surface(surface: DoctorCollectionSurface) -> &'static str {
    match surface {
        DoctorCollectionSurface::Target => "target",
        DoctorCollectionSurface::Requirements => "requirements",
        DoctorCollectionSurface::CheckRuns => "check_runs",
        DoctorCollectionSurface::CommitStatuses => "commit_statuses",
    }
}

fn evaluation_kind(kind: DoctorEvaluationTargetKind) -> &'static str {
    match kind {
        DoctorEvaluationTargetKind::PrHead => "pr_head",
        DoctorEvaluationTargetKind::TestMerge => "test_merge",
        DoctorEvaluationTargetKind::MergeGroup => "merge_group",
    }
}

fn evaluation_resolution(resolution: DoctorEvaluationTargetResolution) -> &'static str {
    match resolution {
        DoctorEvaluationTargetResolution::Selected => "selected",
        DoctorEvaluationTargetResolution::Provisional => "provisional",
    }
}

fn render_doctor_sections(
    output: &mut String,
    requirements: &[DoctorRequirementDiagnosis],
    next_actions: &[DoctorNextAction],
    collection: &DoctorCollection,
    clear_target: &str,
) {
    output.push_str("## Required checks\n\n");
    output.push_str("| Requirement | Expected App | Status | Evidence |\n");
    output.push_str("|---|---:|---|---:|\n");
    for diagnosis in requirements {
        let expected_app = diagnosis
            .key
            .expected_app_id
            .map(|app_id| app_id.to_string())
            .unwrap_or_else(|| "unbound".to_owned());
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            markdown_code(&diagnosis.key.context),
            markdown_code(&expected_app),
            markdown_code(requirement_status(diagnosis.status)),
            diagnosis.evidence.len()
        ));
    }
    if requirements.is_empty() {
        output.push_str("| _No required checks_ | — | — | 0 |\n");
    }

    output.push_str("\n## Diagnosis\n\n");
    for diagnosis in requirements {
        output.push_str(&format!(
            "### {} · {}\n\n{}\n\n",
            markdown_code(&diagnosis.key.context),
            markdown_code(requirement_status(diagnosis.status)),
            markdown_text(&diagnosis.explanation)
        ));
        for policy in &diagnosis.policies {
            output.push_str(&format!(
                "- Policy: [{}](<{}>) {} ({})\n",
                markdown_text(&policy.name),
                policy.url,
                markdown_code(policy_kind(policy.kind)),
                markdown_code(&policy.id)
            ));
        }
        if diagnosis.evidence.is_empty() {
            output.push_str("- Evidence: no matching signal was observed.\n");
        } else {
            for evidence in &diagnosis.evidence {
                let producer = evidence.producer.as_deref().unwrap_or("unknown");
                let conclusion = evidence.conclusion.as_deref().unwrap_or("none");
                output.push_str(&format!(
                    "- Evidence: [{} {}](<{}>) on {} — producer {}, state {}, conclusion {}\n",
                    markdown_code(evidence_kind(evidence.kind)),
                    markdown_code(&evidence.id),
                    evidence.url,
                    markdown_code(&evidence.sha),
                    markdown_code(producer),
                    markdown_code(&evidence.status),
                    markdown_code(conclusion)
                ));
            }
        }
    }
    if requirements.is_empty() {
        output.push_str("No required checks were reported by the collected policy surfaces.\n");
    }

    output.push_str("\n## Next actions\n\n");
    if next_actions.is_empty() {
        output.push_str(&format!(
            "No check-recovery action is needed for {clear_target}.\n"
        ));
    } else {
        for action in next_actions {
            let argv = serde_json::to_string(&action.argv)
                .expect("a string argv is always JSON serializable");
            output.push_str(&format!(
                "- {} — {}\n  - argv: {}\n",
                markdown_text(&action.title),
                markdown_text(&action.rationale),
                markdown_code(&argv)
            ));
        }
    }

    output.push_str("\n## Collection gaps\n\n");
    if collection.gaps.is_empty() {
        output.push_str("Collection was complete.\n");
    } else {
        for gap in &collection.gaps {
            output.push_str(&format!(
                "- {}: {}\n",
                markdown_code(collection_surface(gap.surface)),
                markdown_text(&gap.reason)
            ));
        }
    }
}

pub fn render_pull_request_doctor_markdown(report: &PullRequestDoctorReport) -> String {
    let mut output = String::new();
    output.push_str("# StrataDiff PR Required-Check Doctor\n\n");
    output.push_str(&format!(
        "- Pull request: [#{}](<{}>)\n",
        report.target.number, report.target.url
    ));
    output.push_str(&format!(
        "- Repository: {}\n- Exact head: {}\n- Base: {} at {}\n- Verdict: {}\n\n",
        markdown_code(&report.repository),
        markdown_code(&report.target.head_sha),
        markdown_code(&report.target.base_ref),
        markdown_code(&report.target.base_sha),
        markdown_code(verdict_name(report.verdict))
    ));
    if report.claim_boundary.required_check_readiness_supported {
        output.push_str("The verdict covers required-check readiness for this exact head SHA after confirming that GitHub's test-merge commit had no status signals and no visible rule selected an unsupported target. It does not evaluate reviews, conflicts, deployment policy, or code safety.\n\n");
    } else {
        output.push_str("The observed signals are bound to this exact head SHA, but Doctor could not prove that it is GitHub's active check target. The global verdict is therefore inconclusive and does not evaluate mergeability, reviews, conflicts, deployment policy, or code safety.\n\n");
    }
    render_doctor_sections(
        &mut output,
        &report.requirements,
        &report.next_actions,
        &report.collection,
        "this exact head",
    );
    output
}

fn support_name(supported: bool) -> &'static str {
    if supported {
        "supported"
    } else {
        "not supported"
    }
}

pub fn render_pull_request_doctor_v2_markdown(report: &PullRequestDoctorReportV2) -> String {
    let evaluation = &report.target.evaluation;
    let mut output = String::new();
    output.push_str("# StrataDiff PR Required-Check Doctor\n\n");
    output.push_str(&format!(
        "- Pull request: [#{}](<{}>)\n",
        report.target.number, report.target.url
    ));
    output.push_str(&format!(
        "- Repository: {}\n- Evaluation target: {} at {}\n- Resolution: {}\n- PR head: {}\n- PR base: {} at {}\n",
        markdown_code(&report.repository),
        markdown_code(evaluation_kind(evaluation.kind)),
        markdown_code(&evaluation.sha),
        markdown_code(evaluation_resolution(evaluation.resolution)),
        markdown_code(&report.target.head_sha),
        markdown_code(&report.target.base_ref),
        markdown_code(&report.target.base_sha),
    ));
    if let Some(base_sha) = &evaluation.base_sha {
        output.push_str(&format!("- Evaluation base: {}\n", markdown_code(base_sha)));
    }
    if let (Some(entry_id), Some(state)) = (&evaluation.queue_entry_id, &evaluation.queue_state) {
        output.push_str(&format!(
            "- Merge-queue entry: {} ({})\n",
            markdown_code(entry_id),
            markdown_code(state)
        ));
    }
    output.push_str(&format!(
        "- Verdict: {}\n\n## Claim boundary\n\n- Required-check readiness: {}\n- Mergeability: {}\n- Review requirements: {}\n- Compliance: {}\n- Code safety: {}\n\n",
        markdown_code(verdict_name(report.verdict)),
        support_name(report.claim_boundary.required_check_readiness_supported),
        support_name(report.claim_boundary.mergeability_supported),
        support_name(report.claim_boundary.review_requirements_supported),
        support_name(report.claim_boundary.compliance_supported),
        support_name(report.claim_boundary.code_safety_supported),
    ));
    if evaluation.resolution == DoctorEvaluationTargetResolution::Provisional {
        output.push_str(&format!(
            "This {} evaluation target is provisional: Doctor observed this SHA but has not proved that GitHub selected it as the active required-check target. The global verdict is therefore inconclusive. It does not evaluate mergeability, reviews, conflicts, deployment policy, compliance, or code safety.\n\n",
            markdown_code(evaluation_kind(evaluation.kind)),
        ));
    } else if report.claim_boundary.required_check_readiness_supported {
        output.push_str(&format!(
            "The verdict covers required-check readiness only for the declared {} evaluation target at the exact SHA shown above. It does not evaluate mergeability, reviews, conflicts, deployment policy, compliance, or code safety.\n\n",
            markdown_code(evaluation_kind(evaluation.kind)),
        ));
    } else {
        output.push_str(&format!(
            "The observed signals are bound to the declared {} evaluation target at the exact SHA shown above, but Doctor could not complete a required-check readiness claim. It does not evaluate mergeability, reviews, conflicts, deployment policy, compliance, or code safety.\n\n",
            markdown_code(evaluation_kind(evaluation.kind)),
        ));
    }
    render_doctor_sections(
        &mut output,
        &report.requirements,
        &report.next_actions,
        &report.collection,
        "this exact evaluation target",
    );
    output
}

fn workflow_cause_name(cause: crate::doctor_workflow::WorkflowTriggerCause) -> &'static str {
    use crate::doctor_workflow::WorkflowTriggerCause;
    match cause {
        WorkflowTriggerCause::DuplicateJobNameAmbiguous => "duplicate_job_name_ambiguous",
        WorkflowTriggerCause::ForkApprovalPossible => "fork_approval_possible",
        WorkflowTriggerCause::ForkApprovalRequired => "fork_approval_required",
        WorkflowTriggerCause::MergeGroupTriggerMissing => "merge_group_trigger_missing",
        WorkflowTriggerCause::None => "none",
        WorkflowTriggerCause::ProviderDidNotEmitMergeGroupStatus => {
            "provider_did_not_emit_merge_group_status"
        }
        WorkflowTriggerCause::ProviderRuntimeDeliveryGap => "provider_runtime_delivery_gap",
        WorkflowTriggerCause::PullRequestMergeConflict => "pull_request_merge_conflict",
        WorkflowTriggerCause::RequiredContextNotProduced => "required_context_not_produced",
        WorkflowTriggerCause::WorkflowActivityExcludesSynchronize => {
            "workflow_activity_excludes_synchronize"
        }
        WorkflowTriggerCause::WorkflowBranchFilterExcluded => "workflow_branch_filter_excluded",
        WorkflowTriggerCause::WorkflowDefinitionInvalid => "workflow_definition_invalid",
        WorkflowTriggerCause::WorkflowDisabled => "workflow_disabled",
        WorkflowTriggerCause::WorkflowPathFilterExcluded => "workflow_path_filter_excluded",
        WorkflowTriggerCause::WorkflowTriggerUnknown => "workflow_trigger_unknown",
    }
}

fn workflow_confidence_name(
    confidence: crate::doctor_workflow::WorkflowTriggerConfidence,
) -> &'static str {
    use crate::doctor_workflow::WorkflowTriggerConfidence;
    match confidence {
        WorkflowTriggerConfidence::Certain => "certain",
        WorkflowTriggerConfidence::High => "high",
        WorkflowTriggerConfidence::Uncertain => "uncertain",
    }
}

fn workflow_collection_status_name(status: DoctorWorkflowCollectionStatus) -> &'static str {
    match status {
        DoctorWorkflowCollectionStatus::NotApplicable => "not_applicable",
        DoctorWorkflowCollectionStatus::Complete => "complete",
        DoctorWorkflowCollectionStatus::Partial => "partial",
    }
}

pub fn render_pull_request_doctor_v3_markdown(report: &PullRequestDoctorReportV3) -> String {
    let base = PullRequestDoctorReportV2 {
        schema: PULL_REQUEST_DOCTOR_REPORT_V2_SCHEMA.to_owned(),
        tool_version: report.tool_version.clone(),
        generated_at: report.generated_at.clone(),
        provider_url: report.provider_url.clone(),
        repository: report.repository.clone(),
        target: report.target.clone(),
        collection: report.collection.clone(),
        claim_boundary: report.claim_boundary.clone(),
        verdict: report.verdict,
        summary: report.summary.clone(),
        requirements: report.requirements.clone(),
        next_actions: report.next_actions.clone(),
    };
    let mut output = render_pull_request_doctor_v2_markdown(&base);
    if report.workflow_collection.status == DoctorWorkflowCollectionStatus::NotApplicable {
        return output;
    }

    let (overview, details) = output
        .split_once("\n## Claim boundary\n\n")
        .expect("the v2 Doctor renderer always emits its claim boundary");
    let mut prioritized = String::with_capacity(output.len() + 1_024);
    prioritized.push_str(overview);
    prioritized.push_str("\n\n## Answer\n\n");
    for trigger in &report.workflow_trigger_diagnoses {
        let context = markdown_code(&trigger.requirement.context);
        match trigger
            .diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.cause_code)
        {
            Some(WorkflowTriggerCause::MergeGroupTriggerMissing) => {
                let producer = trigger
                    .producer
                    .as_ref()
                    .expect("a classified workflow diagnosis always has a producer");
                prioritized.push_str(&format!(
                    "- {context}: {} is the unique static workflow-job producer in the exact-SHA inventory, but it does not subscribe to {}. Add that trigger to enable the workflow for merge-queue candidates, then verify that the required check appears.\n",
                    markdown_code(&producer.workflow_path),
                    markdown_code("merge_group"),
                ));
            }
            Some(WorkflowTriggerCause::ForkApprovalRequired) => {
                prioritized.push_str(&format!(
                    "- {context}: the exact-candidate workflow run is waiting for approval because the pull-request head comes from a fork. Review and approve that run in GitHub, then run Doctor again.\n"
                ));
            }
            Some(WorkflowTriggerCause::None) => {
                prioritized.push_str(&format!(
                    "- {context}: the producer has an exact-candidate {} run, so its workflow trigger is not the proven blocker.\n",
                    markdown_code("merge_group"),
                ));
            }
            Some(WorkflowTriggerCause::WorkflowTriggerUnknown) => {
                prioritized.push_str(&format!(
                    "- {context}: the producer is identified, but the available exact-candidate evidence does not prove why the required check is absent.\n"
                ));
            }
            Some(cause) => {
                prioritized.push_str(&format!(
                    "- {context}: Doctor classified the cause as {}.\n",
                    markdown_code(workflow_cause_name(cause)),
                ));
            }
            None => {
                let gap_codes = report
                    .workflow_collection
                    .gaps
                    .iter()
                    .filter(|gap| gap.requirement == trigger.requirement)
                    .map(|gap| markdown_code(&gap.code))
                    .collect::<Vec<_>>();
                if gap_codes.is_empty() {
                    prioritized.push_str(&format!(
                        "- {context}: the required evidence is incomplete, so Doctor did not guess a root cause.\n"
                    ));
                } else {
                    prioritized.push_str(&format!(
                        "- {context}: Doctor did not guess a root cause because evidence is incomplete ({}).\n",
                        gap_codes.join(", "),
                    ));
                }
            }
        }
    }
    prioritized.push_str("\n## Claim boundary\n\n");
    prioritized.push_str(details);
    output = prioritized;

    output.push_str("\n## Workflow trigger diagnoses\n\n");
    output.push_str(&format!(
        "- Collection: {} ({} API calls, {} response bytes)\n",
        markdown_code(workflow_collection_status_name(
            report.workflow_collection.status
        )),
        report.workflow_collection.api_calls,
        report.workflow_collection.response_bytes,
    ));
    for trigger in &report.workflow_trigger_diagnoses {
        output.push_str(&format!(
            "\n### {}\n\n",
            markdown_code(&trigger.requirement.context)
        ));
        if let Some(producer) = &trigger.producer {
            output.push_str(&format!(
                "- Producer: {} (workflow ID {}, check suite {})\n- Producer evidence SHA: {}\n- Check run: [#{}](<{}>)\n",
                markdown_code(&producer.workflow_path),
                producer.workflow_id,
                producer.check_suite_id,
                markdown_code(&producer.source_sha),
                producer.check_run_id,
                producer.check_run_url,
            ));
        } else {
            output.push_str("- Producer: not proven\n");
        }
        if let Some(diagnosis) = &trigger.diagnosis {
            output.push_str(&format!(
                "- Cause: {}\n- Confidence: {}\n- Evidence: {}\n- Repair action: {}{}\n",
                markdown_code(workflow_cause_name(diagnosis.cause_code)),
                markdown_code(workflow_confidence_name(diagnosis.confidence)),
                diagnosis
                    .evidence
                    .iter()
                    .map(|item| markdown_code(item))
                    .collect::<Vec<_>>()
                    .join(", "),
                markdown_code(&diagnosis.fix.action_code),
                if diagnosis.fix.requires_human_edit {
                    " (human edit required)"
                } else {
                    ""
                },
            ));
        } else {
            output.push_str("- Cause: not classified because required evidence was unavailable\n");
        }
        for gap in report
            .workflow_collection
            .gaps
            .iter()
            .filter(|gap| gap.requirement == trigger.requirement)
        {
            output.push_str(&format!(
                "- Evidence gap {}: {}\n",
                markdown_code(&gap.code),
                markdown_text(&gap.reason),
            ));
        }
    }
    output
}
