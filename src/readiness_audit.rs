use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_yaml_ng::Value as YamlValue;

use crate::doctor::{
    DoctorCheckRun, DoctorCollection, DoctorCollectionGap, DoctorCollectionStatus,
    DoctorCollectionSurface, DoctorCommitStatus, DoctorEvaluationTarget,
    DoctorEvaluationTargetKind, DoctorEvaluationTargetResolution, DoctorPolicyKind,
    DoctorPolicyRef, DoctorRequirement, DoctorRequirementKey, DoctorRequirementStatus,
    DoctorTargetV2, DoctorWorkflowCollection, DoctorWorkflowCollectionGap,
    DoctorWorkflowCollectionStatus, DoctorWorkflowInventory, DoctorWorkflowInventoryFile,
    DoctorWorkflowProbe, DoctorWorkflowProbeKind, DoctorWorkflowProducer,
    DoctorWorkflowTriggerInvestigation, PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA,
    PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA, PullRequestDoctorSnapshotV2,
    PullRequestDoctorSnapshotV3, evaluate_pull_request_doctor_v2,
};
use crate::doctor_candidate::{
    CandidateKind, CandidateQueueEntry, CandidateSelectionInput, CandidateSelectionStatus,
    CandidateSignalCollectionStatus, CandidateSignalSurface, CandidateSignals,
    CandidateTargetIdentity, select_candidate,
};
use crate::doctor_workflow::{
    DoctorWorkflowTriggerInput, MergeGroupWorkflowTrigger, PullRequestWorkflowTrigger,
    WorkflowChangedFiles, WorkflowDefinition as DoctorWorkflowDefinition, WorkflowExpectedApp,
    WorkflowJob, WorkflowMergeableState, WorkflowProviderCapability, WorkflowRunConclusion,
    WorkflowRunEvent, WorkflowRunObservation, WorkflowRunStatus, WorkflowState, WorkflowSyntax,
    WorkflowTarget, WorkflowTargetKind, WorkflowTriggers,
};
use crate::ownership::github_provider_hostname;
use crate::readiness::{
    BranchPolicySnapshot, CheckRunSnapshot, CollectionGap, CollectionStatus, CollectionSurface,
    CommitStatusSnapshot, MERGE_READINESS_SNAPSHOT_SCHEMA, MergeReadinessSnapshot,
    PullRequestSnapshot, PullRequestState, RepositorySnapshot, RequiredCheckSnapshot, RulesetRef,
    SnapshotCollection, TriggerSnapshot, WorkflowDefinition, WorkflowRunSnapshot, WorkflowSnapshot,
};

pub const MAX_READINESS_PULL_REQUESTS: usize = 25;
pub const MAX_READINESS_API_REQUESTS: usize = 1_000;
pub const MAX_READINESS_API_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_READINESS_API_TOTAL_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_WORKFLOW_DEFINITION_BYTES: usize = 512 * 1024;

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10;
const MAX_LINK_HEADER_BYTES: usize = 16 * 1024;
const MAX_WORKFLOW_TRIGGER_PATTERNS: usize = 1_000;
const MAX_WORKFLOW_TRIGGER_PATTERN_BYTES: usize = 4 * 1024;
const MAX_DOCTOR_WORKFLOW_DIRECTORY_ENTRIES: usize = 100;
const DOCTOR_CANDIDATE_QUERY: &str = concat!(
    "query StrataDiffPullRequestCandidate($owner:String!,$name:String!,$number:Int!){",
    "repository(owner:$owner,name:$name){nameWithOwner url pullRequest(number:$number){",
    "number url state baseRefName baseRefOid headRefOid mergeable mergeStateStatus ",
    "isMergeQueueEnabled isInMergeQueue potentialMergeCommit{oid} mergeQueueEntry{",
    "id state position baseCommit{oid} headCommit{oid} pullRequest{number headRefOid}}}}}"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubReadinessApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub link_header: Option<String>,
}

pub trait GithubReadinessApi {
    fn get(&mut self, endpoint: &str) -> Result<GithubReadinessApiResponse>;
}

pub trait GithubPullRequestDoctorApi: GithubReadinessApi {
    fn graphql(&mut self, query: &str, variables: &JsonValue)
    -> Result<GithubReadinessApiResponse>;
}

#[derive(Clone, Debug)]
pub struct MergeReadinessCollection<'a> {
    pub provider_url: &'a str,
    pub repository: &'a str,
    pub captured_at: &'a str,
    pub pull_request_limit: usize,
}

#[derive(Clone, Debug)]
pub struct PullRequestDoctorCollection<'a> {
    pub provider_url: &'a str,
    pub repository: &'a str,
    pub captured_at: &'a str,
    pub pull_request_number: u64,
}

#[derive(Debug)]
struct ApiBudget {
    requests: usize,
    response_bytes: usize,
}

impl ApiBudget {
    fn new() -> Self {
        Self {
            requests: 0,
            response_bytes: 0,
        }
    }

    fn get<A: GithubReadinessApi>(
        &mut self,
        api: &mut A,
        endpoint: &str,
    ) -> Result<GithubReadinessApiResponse> {
        self.requests = self
            .requests
            .checked_add(1)
            .context("GitHub readiness API request count overflow")?;
        ensure!(
            self.requests <= MAX_READINESS_API_REQUESTS,
            "GitHub readiness API request limit exceeded: observed {}, limit {MAX_READINESS_API_REQUESTS}",
            self.requests
        );
        let response = api
            .get(endpoint)
            .with_context(|| format!("GitHub readiness request failed for {endpoint}"))?;
        ensure!(
            response.body.len() <= MAX_READINESS_API_RESPONSE_BYTES,
            "GitHub readiness response bytes limit exceeded for {endpoint}: observed {}, limit {MAX_READINESS_API_RESPONSE_BYTES}",
            response.body.len()
        );
        if let Some(link) = &response.link_header {
            ensure!(
                link.len() <= MAX_LINK_HEADER_BYTES,
                "GitHub readiness Link header bytes limit exceeded for {endpoint}"
            );
        }
        self.response_bytes = self
            .response_bytes
            .checked_add(response.body.len())
            .context("GitHub readiness response byte count overflow")?;
        ensure!(
            self.response_bytes <= MAX_READINESS_API_TOTAL_BYTES,
            "GitHub readiness total response bytes limit exceeded: observed {}, limit {MAX_READINESS_API_TOTAL_BYTES}",
            self.response_bytes
        );
        Ok(response)
    }

    fn graphql<A: GithubPullRequestDoctorApi>(
        &mut self,
        api: &mut A,
        query: &str,
        variables: &JsonValue,
    ) -> Result<GithubReadinessApiResponse> {
        self.requests = self
            .requests
            .checked_add(1)
            .context("GitHub readiness API request count overflow")?;
        ensure!(
            self.requests <= MAX_READINESS_API_REQUESTS,
            "GitHub readiness API request limit exceeded: observed {}, limit {MAX_READINESS_API_REQUESTS}",
            self.requests
        );
        let response = api
            .graphql(query, variables)
            .context("GitHub pull-request candidate GraphQL request failed")?;
        ensure!(
            response.body.len() <= MAX_READINESS_API_RESPONSE_BYTES,
            "GitHub readiness response bytes limit exceeded for GraphQL: observed {}, limit {MAX_READINESS_API_RESPONSE_BYTES}",
            response.body.len()
        );
        self.response_bytes = self
            .response_bytes
            .checked_add(response.body.len())
            .context("GitHub readiness response byte count overflow")?;
        ensure!(
            self.response_bytes <= MAX_READINESS_API_TOTAL_BYTES,
            "GitHub readiness total response bytes limit exceeded: observed {}, limit {MAX_READINESS_API_TOTAL_BYTES}",
            self.response_bytes
        );
        Ok(response)
    }
}

#[derive(Debug, Deserialize)]
struct ApiRepository {
    id: u64,
    full_name: String,
    html_url: String,
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct ApiGitRef {
    object: ApiGitObject,
}

#[derive(Debug, Deserialize)]
struct ApiGitObject {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct ApiBranchSummary {
    name: String,
    protected: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiRulesetSummary {
    id: u64,
    name: String,
    source_type: String,
    source: String,
    #[serde(rename = "_links")]
    links: ApiRulesetLinks,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiRulesetLinks {
    html: ApiLink,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiLink {
    href: String,
}

#[derive(Debug, Deserialize)]
struct ApiEffectiveRule {
    #[serde(rename = "type")]
    kind: String,
    parameters: Option<JsonValue>,
    ruleset_source_type: String,
    ruleset_source: String,
    ruleset_id: u64,
}

#[derive(Debug, Deserialize)]
struct ApiRequiredChecksParameters {
    required_status_checks: Vec<ApiRequiredCheck>,
}

#[derive(Debug, Deserialize)]
struct ApiRequiredCheck {
    context: String,
    integration_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiWorkflowList {
    total_count: u64,
    workflows: Vec<ApiWorkflow>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiWorkflow {
    id: u64,
    name: String,
    path: String,
    state: String,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct ApiContent {
    #[serde(rename = "type")]
    kind: String,
    encoding: String,
    content: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorWorkflowDirectoryEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct ApiDoctorWorkflowContent {
    #[serde(rename = "type")]
    kind: String,
    encoding: String,
    content: String,
    path: String,
    sha: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiPullRequest {
    number: u64,
    html_url: String,
    state: String,
    merged_at: Option<String>,
    base: ApiPullBase,
    head: ApiPullHead,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiPullBase {
    #[serde(rename = "ref")]
    reference: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiPullHead {
    sha: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorPullRequest {
    number: u64,
    html_url: String,
    state: String,
    merge_commit_sha: Option<String>,
    base: ApiDoctorPullRef,
    head: ApiDoctorPullHead,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorPullRef {
    #[serde(rename = "ref")]
    reference: String,
    sha: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorPullHead {
    #[serde(rename = "ref")]
    reference: Option<String>,
    sha: String,
    repo: Option<ApiDoctorPullRepository>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorPullRepository {
    full_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorGraphqlEnvelope {
    data: Option<ApiDoctorGraphqlData>,
    #[serde(default)]
    errors: Vec<ApiDoctorGraphqlError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorGraphqlError {
    message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorGraphqlData {
    repository: Option<ApiDoctorGraphqlRepository>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ApiDoctorGraphqlRepository {
    name_with_owner: String,
    url: String,
    pull_request: Option<ApiDoctorGraphqlPullRequest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ApiDoctorGraphqlPullRequest {
    number: u64,
    url: String,
    state: String,
    base_ref_name: String,
    base_ref_oid: String,
    head_ref_oid: String,
    mergeable: String,
    merge_state_status: String,
    is_merge_queue_enabled: bool,
    is_in_merge_queue: bool,
    potential_merge_commit: Option<ApiDoctorGraphqlCommit>,
    merge_queue_entry: Option<ApiDoctorGraphqlQueueEntry>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ApiDoctorGraphqlCommit {
    oid: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ApiDoctorGraphqlQueueEntry {
    id: String,
    state: String,
    position: u64,
    base_commit: ApiDoctorGraphqlCommit,
    head_commit: ApiDoctorGraphqlCommit,
    pull_request: ApiDoctorGraphqlQueuePullRequest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ApiDoctorGraphqlQueuePullRequest {
    number: u64,
    head_ref_oid: String,
}

#[derive(Debug, Deserialize)]
struct ApiBranchProtection {
    required_status_checks: Option<ApiClassicRequiredStatusChecks>,
}

#[derive(Debug, Deserialize)]
struct ApiClassicRequiredStatusChecks {
    #[serde(default)]
    contexts: Vec<String>,
    #[serde(default)]
    checks: Vec<ApiClassicRequiredCheck>,
}

#[derive(Debug, Deserialize)]
struct ApiClassicRequiredCheck {
    context: String,
    app_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorCheckRunList {
    total_count: u64,
    check_runs: Vec<ApiDoctorCheckRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorCheckRun {
    id: u64,
    html_url: String,
    name: String,
    head_sha: String,
    app: Option<ApiApp>,
    status: String,
    conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorProducerCheckRunList {
    total_count: u64,
    check_runs: Vec<ApiDoctorProducerCheckRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorProducerCheckRun {
    id: u64,
    url: String,
    html_url: String,
    name: String,
    head_sha: String,
    app: Option<ApiApp>,
    check_suite: Option<ApiCheckSuite>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorWorkflowRunList {
    total_count: u64,
    workflow_runs: Vec<ApiDoctorWorkflowRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorWorkflowRun {
    id: u64,
    html_url: String,
    workflow_id: u64,
    path: String,
    event: String,
    head_sha: String,
    check_suite_id: u64,
    run_attempt: u64,
    status: String,
    conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorWorkflowJobList {
    total_count: u64,
    jobs: Vec<ApiDoctorWorkflowJob>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDoctorWorkflowJob {
    id: u64,
    html_url: String,
    name: String,
    check_run_url: String,
    run_attempt: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiCheckRunList {
    total_count: u64,
    check_runs: Vec<ApiCheckRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiCheckRun {
    id: u64,
    html_url: String,
    name: String,
    app: Option<ApiApp>,
    check_suite: Option<ApiCheckSuite>,
    status: String,
    conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiApp {
    id: u64,
    slug: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiCheckSuite {
    id: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiCommitStatus {
    id: u64,
    url: String,
    context: String,
    creator: Option<ApiStatusCreator>,
    state: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiStatusCreator {
    id: u64,
    login: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiWorkflowRunList {
    total_count: u64,
    workflow_runs: Vec<ApiWorkflowRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiWorkflowRun {
    id: u64,
    html_url: String,
    workflow_id: u64,
    path: String,
    event: String,
    check_suite_id: u64,
    run_attempt: u64,
}

fn parse_repository(value: &str) -> Result<(String, String)> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    ensure!(
        parts.next().is_none()
            && valid_repository_component(owner)
            && valid_repository_component(name),
        "repository must use canonical OWNER/REPO form"
    );
    Ok((owner.to_owned(), name.to_owned()))
}

fn valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_provider_url(value: &str) -> Result<()> {
    github_provider_hostname(value)?;
    Ok(())
}

fn encode_path_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

fn encode_repository_path(path: &str) -> String {
    path.split('/')
        .map(encode_path_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8], endpoint: &str) -> Result<T> {
    serde_json::from_slice(body)
        .with_context(|| format!("failed to decode GitHub response for {endpoint}"))
}

fn add_gap(gaps: &mut Vec<CollectionGap>, surface: CollectionSurface, reason: String) {
    gaps.push(CollectionGap { surface, reason });
}

fn successful(response: &GithubReadinessApiResponse) -> bool {
    (200..300).contains(&response.status)
}

fn incomplete_page(observed: usize, total: u64) -> bool {
    u64::try_from(observed).is_ok_and(|count| count < total)
}

fn has_next_page(link_header: Option<&str>) -> bool {
    link_header.is_some_and(|header| {
        header
            .split(',')
            .any(|link| link.split(';').any(|part| part.trim() == "rel=\"next\""))
    })
}

fn trigger_patterns(value: Option<&YamlValue>, label: &str) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let patterns = match value {
        YamlValue::String(pattern) => vec![pattern.clone()],
        YamlValue::Sequence(patterns) => patterns
            .iter()
            .map(|pattern| {
                pattern
                    .as_str()
                    .map(str::to_owned)
                    .with_context(|| format!("{label} contains a non-string pattern"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("{label} must be a string or string sequence"),
    };
    ensure!(
        patterns.len() <= MAX_WORKFLOW_TRIGGER_PATTERNS,
        "{label} pattern count exceeds {MAX_WORKFLOW_TRIGGER_PATTERNS}"
    );
    for pattern in &patterns {
        ensure!(!pattern.is_empty(), "{label} contains an empty pattern");
        ensure!(
            pattern.len() <= MAX_WORKFLOW_TRIGGER_PATTERN_BYTES,
            "{label} pattern exceeds {MAX_WORKFLOW_TRIGGER_PATTERN_BYTES} bytes"
        );
        ensure!(
            !pattern.chars().any(char::is_control),
            "{label} pattern contains a control character"
        );
    }
    Ok(patterns)
}

fn parse_trigger(value: &YamlValue, event: &str) -> Result<TriggerSnapshot> {
    match value {
        YamlValue::Null => Ok(TriggerSnapshot {
            paths: Vec::new(),
            paths_ignore: Vec::new(),
        }),
        YamlValue::Mapping(parameters) => Ok(TriggerSnapshot {
            paths: trigger_patterns(
                parameters.get(YamlValue::String("paths".to_owned())),
                &format!("{event}.paths"),
            )?,
            paths_ignore: trigger_patterns(
                parameters.get(YamlValue::String("paths-ignore".to_owned())),
                &format!("{event}.paths-ignore"),
            )?,
        }),
        _ => bail!("workflow trigger {event} must be null or a mapping"),
    }
}

pub fn parse_workflow_definition(bytes: &[u8]) -> Result<WorkflowDefinition> {
    ensure!(
        bytes.len() <= MAX_WORKFLOW_DEFINITION_BYTES,
        "workflow definition bytes limit exceeded: observed {}, limit {MAX_WORKFLOW_DEFINITION_BYTES}",
        bytes.len()
    );
    let root: YamlValue =
        serde_yaml_ng::from_slice(bytes).context("failed to decode workflow YAML")?;
    let mapping = root
        .as_mapping()
        .context("workflow YAML root must be a mapping")?;
    let triggers = mapping
        .get(YamlValue::String("on".to_owned()))
        .context("workflow YAML is missing its on trigger")?;
    let mut definition = WorkflowDefinition {
        pull_request: None,
        pull_request_target: None,
        merge_group: None,
    };
    match triggers {
        YamlValue::String(event) => set_scalar_trigger(&mut definition, event)?,
        YamlValue::Sequence(events) => {
            for event in events {
                let event = event
                    .as_str()
                    .context("workflow on sequence contains a non-string event")?;
                set_scalar_trigger(&mut definition, event)?;
            }
        }
        YamlValue::Mapping(events) => {
            for (event, parameters) in events {
                let event = event
                    .as_str()
                    .context("workflow on mapping contains a non-string event")?;
                match event {
                    "pull_request" => {
                        definition.pull_request = Some(parse_trigger(parameters, event)?);
                    }
                    "pull_request_target" => {
                        definition.pull_request_target = Some(parse_trigger(parameters, event)?);
                    }
                    "merge_group" => {
                        definition.merge_group = Some(parse_trigger(parameters, event)?);
                    }
                    _ => {}
                }
            }
        }
        _ => bail!("workflow on trigger must be a string, sequence, or mapping"),
    }
    Ok(definition)
}

fn doctor_pull_request_trigger(value: &YamlValue) -> Result<PullRequestWorkflowTrigger> {
    match value {
        YamlValue::Null => Ok(PullRequestWorkflowTrigger {
            branches: Vec::new(),
            branches_ignore: Vec::new(),
            paths: Vec::new(),
            paths_ignore: Vec::new(),
            types: Vec::new(),
        }),
        YamlValue::Mapping(parameters) => Ok(PullRequestWorkflowTrigger {
            branches: trigger_patterns(
                parameters.get(YamlValue::String("branches".to_owned())),
                "pull_request.branches",
            )?,
            branches_ignore: trigger_patterns(
                parameters.get(YamlValue::String("branches-ignore".to_owned())),
                "pull_request.branches-ignore",
            )?,
            paths: trigger_patterns(
                parameters.get(YamlValue::String("paths".to_owned())),
                "pull_request.paths",
            )?,
            paths_ignore: trigger_patterns(
                parameters.get(YamlValue::String("paths-ignore".to_owned())),
                "pull_request.paths-ignore",
            )?,
            types: trigger_patterns(
                parameters.get(YamlValue::String("types".to_owned())),
                "pull_request.types",
            )?,
        }),
        _ => bail!("workflow trigger pull_request must be null or a mapping"),
    }
}

fn doctor_merge_group_trigger(value: &YamlValue) -> Result<MergeGroupWorkflowTrigger> {
    match value {
        YamlValue::Null => Ok(MergeGroupWorkflowTrigger { types: Vec::new() }),
        YamlValue::Mapping(parameters) => Ok(MergeGroupWorkflowTrigger {
            types: trigger_patterns(
                parameters.get(YamlValue::String("types".to_owned())),
                "merge_group.types",
            )?,
        }),
        _ => bail!("workflow trigger merge_group must be null or a mapping"),
    }
}

fn set_doctor_scalar_trigger(triggers: &mut WorkflowTriggers, event: &str) {
    match event {
        "pull_request" => {
            triggers.pull_request = Some(PullRequestWorkflowTrigger {
                branches: Vec::new(),
                branches_ignore: Vec::new(),
                paths: Vec::new(),
                paths_ignore: Vec::new(),
                types: Vec::new(),
            });
        }
        "merge_group" => {
            triggers.merge_group = Some(MergeGroupWorkflowTrigger { types: Vec::new() });
        }
        _ => {}
    }
}

fn doctor_workflow_triggers(value: &YamlValue) -> Result<WorkflowTriggers> {
    let mut triggers = WorkflowTriggers {
        merge_group: None,
        pull_request: None,
    };
    match value {
        YamlValue::String(event) => set_doctor_scalar_trigger(&mut triggers, event),
        YamlValue::Sequence(events) => {
            for event in events {
                let event = event
                    .as_str()
                    .context("workflow on sequence contains a non-string event")?;
                set_doctor_scalar_trigger(&mut triggers, event);
            }
        }
        YamlValue::Mapping(events) => {
            for (event, parameters) in events {
                let event = event
                    .as_str()
                    .context("workflow on mapping contains a non-string event")?;
                match event {
                    "pull_request" => {
                        triggers.pull_request = Some(doctor_pull_request_trigger(parameters)?);
                    }
                    "merge_group" => {
                        triggers.merge_group = Some(doctor_merge_group_trigger(parameters)?);
                    }
                    _ => {}
                }
            }
        }
        _ => bail!("workflow on trigger must be a string, sequence, or mapping"),
    }
    Ok(triggers)
}

fn doctor_job_condition(job: &serde_yaml_ng::Mapping) -> Result<String> {
    let Some(condition) = job.get(YamlValue::String("if".to_owned())) else {
        return Ok("always".to_owned());
    };
    match condition {
        YamlValue::String(_) | YamlValue::Bool(_) | YamlValue::Number(_) => {
            Ok("conditional".to_owned())
        }
        _ => bail!("workflow job if condition must be a scalar"),
    }
}

pub fn parse_doctor_workflow_definition(
    bytes: &[u8],
    path: &str,
    state: WorkflowState,
) -> Result<DoctorWorkflowDefinition> {
    ensure!(
        bytes.len() <= MAX_WORKFLOW_DEFINITION_BYTES,
        "workflow definition bytes limit exceeded: observed {}, limit {MAX_WORKFLOW_DEFINITION_BYTES}",
        bytes.len()
    );
    let root: YamlValue =
        serde_yaml_ng::from_slice(bytes).context("failed to decode workflow YAML")?;
    let mapping = root
        .as_mapping()
        .context("workflow YAML root must be a mapping")?;
    let on = mapping
        .get(YamlValue::String("on".to_owned()))
        .context("workflow YAML is missing its on trigger")?;
    let jobs = mapping
        .get(YamlValue::String("jobs".to_owned()))
        .and_then(YamlValue::as_mapping)
        .context("workflow YAML jobs must be a mapping")?;
    ensure!(
        jobs.len() <= MAX_WORKFLOW_TRIGGER_PATTERNS,
        "too many workflow jobs"
    );

    let mut parsed_jobs = Vec::new();
    for (id, job) in jobs {
        let id = id
            .as_str()
            .context("workflow job ID must be a string")?
            .to_owned();
        ensure!(
            !id.is_empty() && id.len() <= 255 && !id.chars().any(char::is_control),
            "workflow job ID is malformed"
        );
        let job = job
            .as_mapping()
            .context("workflow job definition must be a mapping")?;
        let name = match job.get(YamlValue::String("name".to_owned())) {
            Some(value) => value
                .as_str()
                .context("workflow job name must be a string")?
                .to_owned(),
            None => id.clone(),
        };
        ensure!(
            !name.is_empty() && name.len() <= 255 && !name.chars().any(char::is_control),
            "workflow job name is malformed"
        );
        let has_matrix = job
            .get(YamlValue::String("strategy".to_owned()))
            .and_then(YamlValue::as_mapping)
            .is_some_and(|strategy| strategy.contains_key(YamlValue::String("matrix".to_owned())));
        let name_static = !name.contains("${{") && !has_matrix;
        parsed_jobs.push(WorkflowJob {
            condition: doctor_job_condition(job)?,
            id,
            name: if name_static {
                name
            } else {
                "<dynamic>".to_owned()
            },
            name_static,
            reusable: job.contains_key(YamlValue::String("uses".to_owned())),
        });
    }
    parsed_jobs.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(DoctorWorkflowDefinition {
        jobs: parsed_jobs,
        path: path.to_owned(),
        state,
        syntax: WorkflowSyntax::Valid,
        triggers: doctor_workflow_triggers(on)?,
    })
}

fn set_scalar_trigger(definition: &mut WorkflowDefinition, event: &str) -> Result<()> {
    let trigger = TriggerSnapshot {
        paths: Vec::new(),
        paths_ignore: Vec::new(),
    };
    match event {
        "pull_request" => definition.pull_request = Some(trigger),
        "pull_request_target" => definition.pull_request_target = Some(trigger),
        "merge_group" => definition.merge_group = Some(trigger),
        _ => {}
    }
    Ok(())
}

fn required_checks_from_rules(
    rules: &[ApiEffectiveRule],
    summaries: &BTreeMap<u64, ApiRulesetSummary>,
    repository_url: &str,
) -> Result<BranchPolicySnapshot> {
    let mut required_checks = Vec::new();
    for rule in rules {
        if rule.kind != "required_status_checks" {
            continue;
        }
        let parameters = rule
            .parameters
            .clone()
            .context("required_status_checks rule is missing parameters")?;
        let parameters: ApiRequiredChecksParameters = serde_json::from_value(parameters)
            .context("failed to decode required_status_checks parameters")?;
        let ruleset = summaries
            .get(&rule.ruleset_id)
            .map(|summary| RulesetRef {
                id: summary.id,
                name: summary.name.clone(),
                url: summary.links.html.href.clone(),
                source_type: summary.source_type.clone(),
                source: summary.source.clone(),
            })
            .unwrap_or_else(|| RulesetRef {
                id: rule.ruleset_id,
                name: format!("ruleset {}", rule.ruleset_id),
                url: format!("{repository_url}/rules/{}", rule.ruleset_id),
                source_type: rule.ruleset_source_type.clone(),
                source: rule.ruleset_source.clone(),
            });
        for check in parameters.required_status_checks {
            required_checks.push(RequiredCheckSnapshot {
                context: check.context,
                integration_id: check.integration_id,
                rulesets: vec![ruleset.clone()],
            });
        }
    }
    required_checks.sort_by(|left, right| {
        left.context
            .cmp(&right.context)
            .then_with(|| left.integration_id.cmp(&right.integration_id))
            .then_with(|| left.rulesets.cmp(&right.rulesets))
    });
    Ok(BranchPolicySnapshot {
        branch_protected: false,
        effective_rule_count: u64::try_from(rules.len())?,
        pull_request_rule: rules.iter().any(|rule| rule.kind == "pull_request"),
        merge_queue_rule: rules.iter().any(|rule| rule.kind == "merge_queue"),
        required_checks,
    })
}

fn collect_rulesets<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    gaps: &mut Vec<CollectionGap>,
) -> Result<BTreeMap<u64, ApiRulesetSummary>> {
    let mut summaries = BTreeMap::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/rulesets?includes_parents=true&per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_gap(
                gaps,
                CollectionSurface::Rulesets,
                format!("GitHub returned HTTP {} for {endpoint}", response.status),
            );
            break;
        }
        let batch: Vec<ApiRulesetSummary> = parse_json(&response.body, &endpoint)?;
        let batch_len = batch.len();
        for summary in batch {
            ensure!(
                summaries.insert(summary.id, summary).is_none(),
                "GitHub returned a duplicate ruleset ID"
            );
        }
        if batch_len < PAGE_SIZE {
            return Ok(summaries);
        }
        if page == MAX_PAGES {
            add_gap(
                gaps,
                CollectionSurface::Rulesets,
                format!("ruleset pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    Ok(summaries)
}

fn collect_effective_rules<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    branch: &str,
    gaps: &mut Vec<CollectionGap>,
) -> Result<Vec<ApiEffectiveRule>> {
    let endpoint = format!(
        "repos/{repository_path}/rules/branches/{}",
        encode_path_component(branch)
    );
    let response = budget.get(api, &endpoint)?;
    if !successful(&response) {
        add_gap(
            gaps,
            CollectionSurface::EffectiveRules,
            format!("GitHub returned HTTP {} for {endpoint}", response.status),
        );
        return Ok(Vec::new());
    }
    parse_json(&response.body, &endpoint)
}

fn collect_branch_protection<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    branch: &str,
    gaps: &mut Vec<CollectionGap>,
) -> Result<bool> {
    let mut github_reports_protected = false;
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/branches?protected=true&per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_gap(
                gaps,
                CollectionSurface::Repository,
                format!(
                    "GitHub returned HTTP {} while listing protected branches",
                    response.status
                ),
            );
            return Ok(false);
        }
        let branches: Vec<ApiBranchSummary> = parse_json(&response.body, &endpoint)?;
        let page_len = branches.len();
        if branches
            .iter()
            .any(|candidate| candidate.name == branch && candidate.protected)
        {
            github_reports_protected = true;
            break;
        }
        if page_len < PAGE_SIZE {
            break;
        }
        if page == MAX_PAGES {
            add_gap(
                gaps,
                CollectionSurface::Repository,
                format!("protected-branch pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    if !github_reports_protected {
        return Ok(false);
    }

    let endpoint = format!(
        "repos/{repository_path}/branches/{}/protection",
        encode_path_component(branch)
    );
    let response = budget.get(api, &endpoint)?;
    match response.status {
        200 => {
            add_gap(
                gaps,
                CollectionSurface::EffectiveRules,
                "classic branch protection is active, but this snapshot does not yet encode its required status checks".to_owned(),
            );
            Ok(true)
        }
        404 => {
            add_gap(
                gaps,
                CollectionSurface::EffectiveRules,
                "GitHub marks the default branch protected, but classic branch protection was absent or unreadable".to_owned(),
            );
            Ok(false)
        }
        status => {
            add_gap(
                gaps,
                CollectionSurface::EffectiveRules,
                format!("GitHub returned HTTP {status} while checking classic branch protection"),
            );
            Ok(false)
        }
    }
}

fn collect_pull_candidates<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    branch: &str,
    limit: usize,
    gaps: &mut Vec<CollectionGap>,
) -> Result<Vec<ApiPullRequest>> {
    let mut pulls = Vec::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/pulls?state=all&sort=updated&direction=desc&base={}&per_page={PAGE_SIZE}&page={page}",
            encode_path_component(branch)
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_gap(
                gaps,
                CollectionSurface::PullRequests,
                format!("GitHub returned HTTP {} for {endpoint}", response.status),
            );
            return Ok(pulls);
        }
        let batch: Vec<ApiPullRequest> = parse_json(&response.body, &endpoint)?;
        ensure!(
            batch.iter().all(|pull| pull.base.reference == branch),
            "GitHub returned a pull request targeting a different base branch"
        );
        let batch_len = batch.len();
        pulls.extend(batch.into_iter().filter(|pull| {
            pull.state == "open" || (pull.state == "closed" && pull.merged_at.is_some())
        }));
        pulls.truncate(limit);
        if pulls.len() == limit || batch_len < PAGE_SIZE {
            return Ok(pulls);
        }
        if page == MAX_PAGES {
            add_gap(
                gaps,
                CollectionSurface::PullRequests,
                format!("pull-request candidate pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    Ok(pulls)
}

fn collect_check_runs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    head_sha: &str,
) -> Result<(Vec<CheckRunSnapshot>, Option<String>)> {
    let mut check_runs = Vec::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/commits/{head_sha}/check-runs?filter=all&per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                check_runs,
                Some(format!(
                    "GitHub returned HTTP {} for {endpoint}",
                    response.status
                )),
            ));
        }
        let page_body: ApiCheckRunList = parse_json(&response.body, &endpoint)?;
        let page_len = page_body.check_runs.len();
        check_runs.extend(
            page_body
                .check_runs
                .into_iter()
                .map(|check| CheckRunSnapshot {
                    id: check.id,
                    url: check.html_url,
                    name: check.name,
                    app_id: check.app.as_ref().map(|app| app.id),
                    app_slug: check.app.map(|app| app.slug),
                    check_suite_id: check.check_suite.map(|suite| suite.id),
                    status: check.status,
                    conclusion: check.conclusion,
                }),
        );
        if !incomplete_page(check_runs.len(), page_body.total_count) {
            return Ok((check_runs, None));
        }
        if page_len < PAGE_SIZE {
            return Ok((
                check_runs,
                Some("check-run response declared more results than it returned".to_owned()),
            ));
        }
        if page == MAX_PAGES {
            return Ok((
                check_runs,
                Some(format!("check-run pagination exceeded {MAX_PAGES} pages")),
            ));
        }
    }
    unreachable!("bounded page loop always returns")
}

fn collect_statuses<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    head_sha: &str,
) -> Result<(Vec<CommitStatusSnapshot>, Option<String>)> {
    let mut statuses = Vec::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/commits/{head_sha}/statuses?per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                statuses,
                Some(format!(
                    "GitHub returned HTTP {} for {endpoint}",
                    response.status
                )),
            ));
        }
        let batch: Vec<ApiCommitStatus> = parse_json(&response.body, &endpoint)?;
        let batch_len = batch.len();
        statuses.extend(batch.into_iter().map(|status| CommitStatusSnapshot {
            id: status.id,
            url: status.url,
            context: status.context,
            creator_id: status.creator.as_ref().map(|creator| creator.id),
            creator_login: status.creator.map(|creator| creator.login),
            state: status.state,
        }));
        if batch_len < PAGE_SIZE || !has_next_page(response.link_header.as_deref()) {
            return Ok((statuses, None));
        }
        if page == MAX_PAGES {
            return Ok((
                statuses,
                Some(format!(
                    "commit-status pagination exceeded {MAX_PAGES} pages"
                )),
            ));
        }
    }
    unreachable!("bounded page loop always returns")
}

fn collect_workflow_runs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    head_sha: &str,
) -> Result<(Vec<WorkflowRunSnapshot>, Option<String>)> {
    let mut runs = Vec::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/actions/runs?head_sha={head_sha}&per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                runs,
                Some(format!(
                    "GitHub returned HTTP {} for {endpoint}",
                    response.status
                )),
            ));
        }
        let page_body: ApiWorkflowRunList = parse_json(&response.body, &endpoint)?;
        let page_len = page_body.workflow_runs.len();
        runs.extend(
            page_body
                .workflow_runs
                .into_iter()
                .map(|run| WorkflowRunSnapshot {
                    id: run.id,
                    url: run.html_url,
                    workflow_id: run.workflow_id,
                    path: run.path,
                    event: run.event,
                    check_suite_id: run.check_suite_id,
                    run_attempt: run.run_attempt,
                }),
        );
        if !incomplete_page(runs.len(), page_body.total_count) {
            return Ok((runs, None));
        }
        if page_len < PAGE_SIZE {
            return Ok((
                runs,
                Some("workflow-run response declared more results than it returned".to_owned()),
            ));
        }
        if page == MAX_PAGES {
            return Ok((
                runs,
                Some(format!(
                    "workflow-run pagination exceeded {MAX_PAGES} pages"
                )),
            ));
        }
    }
    unreachable!("bounded page loop always returns")
}

fn collect_workflow_inventory<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    gaps: &mut Vec<CollectionGap>,
) -> Result<BTreeMap<u64, ApiWorkflow>> {
    let mut workflows = BTreeMap::new();
    let mut expected_total = None;
    for page in 1..=MAX_PAGES {
        let endpoint =
            format!("repos/{repository_path}/actions/workflows?per_page={PAGE_SIZE}&page={page}");
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_gap(
                gaps,
                CollectionSurface::Workflows,
                format!("GitHub returned HTTP {} for {endpoint}", response.status),
            );
            break;
        }
        let page_body: ApiWorkflowList = parse_json(&response.body, &endpoint)?;
        if let Some(total) = expected_total {
            ensure!(
                total == page_body.total_count,
                "workflow total_count changed during collection"
            );
        } else {
            expected_total = Some(page_body.total_count);
        }
        let page_len = page_body.workflows.len();
        for workflow in page_body.workflows {
            ensure!(
                workflows.insert(workflow.id, workflow).is_none(),
                "GitHub returned a duplicate workflow ID"
            );
        }
        if u64::try_from(workflows.len())? >= page_body.total_count {
            return Ok(workflows);
        }
        if page_len < PAGE_SIZE {
            add_gap(
                gaps,
                CollectionSurface::Workflows,
                "workflow response declared more results than it returned".to_owned(),
            );
            break;
        }
        if page == MAX_PAGES {
            add_gap(
                gaps,
                CollectionSurface::Workflows,
                format!("workflow pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    Ok(workflows)
}

fn decode_workflow_content(content: ApiContent) -> Result<Vec<u8>> {
    ensure!(content.kind == "file", "workflow content is not a file");
    ensure!(
        content.encoding == "base64",
        "workflow content is not base64 encoded"
    );
    let compact = content
        .content
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let bytes = STANDARD
        .decode(compact)
        .context("failed to decode workflow content base64")?;
    ensure!(
        bytes.len() <= MAX_WORKFLOW_DEFINITION_BYTES,
        "workflow definition bytes limit exceeded: observed {}, limit {MAX_WORKFLOW_DEFINITION_BYTES}",
        bytes.len()
    );
    Ok(bytes)
}

fn relevant_workflow_ids(
    required_contexts: &BTreeSet<String>,
    pulls: &[PullRequestSnapshot],
) -> BTreeSet<u64> {
    let mut ids = BTreeSet::new();
    for pull in pulls {
        let runs = pull
            .workflow_runs
            .iter()
            .map(|run| (run.check_suite_id, run.workflow_id))
            .collect::<BTreeMap<_, _>>();
        for check in &pull.check_runs {
            if !required_contexts.contains(&check.name) {
                continue;
            }
            let Some(suite_id) = check.check_suite_id else {
                continue;
            };
            if let Some(workflow_id) = runs.get(&suite_id) {
                ids.insert(*workflow_id);
            }
        }
    }
    ids
}

fn collect_relevant_workflows<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    default_head: &str,
    relevant_ids: &BTreeSet<u64>,
    gaps: &mut Vec<CollectionGap>,
) -> Result<Vec<WorkflowSnapshot>> {
    if relevant_ids.is_empty() {
        return Ok(Vec::new());
    }
    let inventory = collect_workflow_inventory(api, budget, repository_path, gaps)?;
    let mut workflows = Vec::new();
    for workflow_id in relevant_ids {
        let Some(workflow) = inventory.get(workflow_id) else {
            continue;
        };
        let mut definition = None;
        if workflow.state == "active" {
            let endpoint = format!(
                "repos/{repository_path}/contents/{}?ref={default_head}",
                encode_repository_path(&workflow.path)
            );
            let response = budget.get(api, &endpoint)?;
            if successful(&response) {
                let content: ApiContent = parse_json(&response.body, &endpoint)?;
                match decode_workflow_content(content).and_then(|bytes| {
                    parse_workflow_definition(&bytes)
                        .with_context(|| format!("failed to parse {}", workflow.path))
                }) {
                    Ok(parsed) => definition = Some(parsed),
                    Err(error) => add_gap(
                        gaps,
                        CollectionSurface::WorkflowDefinitions,
                        format!("{}: {error:#}", workflow.path),
                    ),
                }
            } else {
                add_gap(
                    gaps,
                    CollectionSurface::WorkflowDefinitions,
                    format!(
                        "GitHub returned HTTP {} for workflow {}",
                        response.status, workflow.path
                    ),
                );
            }
        }
        workflows.push(WorkflowSnapshot {
            id: workflow.id,
            name: workflow.name.clone(),
            path: workflow.path.clone(),
            state: workflow.state.clone(),
            url: workflow.html_url.clone(),
            definition,
        });
    }
    workflows.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(workflows)
}

fn collect_pull_observations<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    branch: &str,
    limit: usize,
    gaps: &mut Vec<CollectionGap>,
) -> Result<Vec<PullRequestSnapshot>> {
    let candidates = collect_pull_candidates(api, budget, repository_path, branch, limit, gaps)?;
    let mut pulls = Vec::new();
    for candidate in candidates {
        let (check_runs, check_gap) =
            collect_check_runs(api, budget, repository_path, &candidate.head.sha)?;
        let (statuses, status_gap) =
            collect_statuses(api, budget, repository_path, &candidate.head.sha)?;
        let (workflow_runs, workflow_gap) =
            collect_workflow_runs(api, budget, repository_path, &candidate.head.sha)?;
        let collection = if check_gap.is_none() && status_gap.is_none() && workflow_gap.is_none() {
            CollectionStatus::Complete
        } else {
            CollectionStatus::Partial
        };
        if let Some(reason) = check_gap {
            add_gap(
                gaps,
                CollectionSurface::CheckRuns,
                format!("pull request #{}: {reason}", candidate.number),
            );
        }
        if let Some(reason) = status_gap {
            add_gap(
                gaps,
                CollectionSurface::CommitStatuses,
                format!("pull request #{}: {reason}", candidate.number),
            );
        }
        if let Some(reason) = workflow_gap {
            add_gap(
                gaps,
                CollectionSurface::WorkflowRuns,
                format!("pull request #{}: {reason}", candidate.number),
            );
        }
        let state = match (candidate.state.as_str(), candidate.merged_at.is_some()) {
            ("open", false) => PullRequestState::Open,
            ("closed", true) => PullRequestState::Merged,
            _ => bail!("selected pull request has an inconsistent state"),
        };
        pulls.push(PullRequestSnapshot {
            number: candidate.number,
            url: candidate.html_url,
            state,
            head_sha: candidate.head.sha.to_ascii_lowercase(),
            collection,
            check_runs,
            statuses,
            workflow_runs,
        });
    }
    Ok(pulls)
}

pub fn collect_merge_readiness_snapshot<A: GithubReadinessApi>(
    request: MergeReadinessCollection<'_>,
    api: &mut A,
) -> Result<MergeReadinessSnapshot> {
    validate_provider_url(request.provider_url)?;
    ensure!(
        !request.captured_at.is_empty(),
        "captured_at must not be empty"
    );
    ensure!(
        (1..=MAX_READINESS_PULL_REQUESTS).contains(&request.pull_request_limit),
        "pull-request limit must be between 1 and {MAX_READINESS_PULL_REQUESTS}"
    );
    let (owner, name) = parse_repository(request.repository)?;
    let repository_path = format!(
        "{}/{}",
        encode_path_component(&owner),
        encode_path_component(&name)
    );
    let mut budget = ApiBudget::new();
    let mut gaps = Vec::new();

    let repository_endpoint = format!("repos/{repository_path}");
    let repository_response = budget.get(api, &repository_endpoint)?;
    ensure!(
        successful(&repository_response),
        "GitHub returned HTTP {} for {repository_endpoint}",
        repository_response.status
    );
    let repository: ApiRepository = parse_json(&repository_response.body, &repository_endpoint)?;
    ensure!(
        repository
            .full_name
            .eq_ignore_ascii_case(request.repository),
        "GitHub repository identity does not match the requested repository"
    );
    ensure!(
        repository.html_url == format!("{}/{}", request.provider_url, repository.full_name),
        "GitHub repository URL does not match the requested provider"
    );

    let reference_endpoint = format!(
        "repos/{repository_path}/git/ref/heads/{}",
        encode_path_component(&repository.default_branch)
    );
    let reference_response = budget.get(api, &reference_endpoint)?;
    ensure!(
        successful(&reference_response),
        "GitHub returned HTTP {} for {reference_endpoint}",
        reference_response.status
    );
    let reference: ApiGitRef = parse_json(&reference_response.body, &reference_endpoint)?;

    let ruleset_summaries = collect_rulesets(api, &mut budget, &repository_path, &mut gaps)?;
    let effective_rules = collect_effective_rules(
        api,
        &mut budget,
        &repository_path,
        &repository.default_branch,
        &mut gaps,
    )?;
    let branch_protected = collect_branch_protection(
        api,
        &mut budget,
        &repository_path,
        &repository.default_branch,
        &mut gaps,
    )?;
    let mut branch_policy =
        required_checks_from_rules(&effective_rules, &ruleset_summaries, &repository.html_url)?;
    branch_policy.branch_protected = branch_protected;

    let pull_requests = if branch_policy.required_checks.is_empty() {
        Vec::new()
    } else {
        collect_pull_observations(
            api,
            &mut budget,
            &repository_path,
            &repository.default_branch,
            request.pull_request_limit,
            &mut gaps,
        )?
    };
    let required_contexts = branch_policy
        .required_checks
        .iter()
        .map(|check| check.context.clone())
        .collect::<BTreeSet<_>>();
    let relevant_ids = relevant_workflow_ids(&required_contexts, &pull_requests);
    let workflows = collect_relevant_workflows(
        api,
        &mut budget,
        &repository_path,
        &reference.object.sha,
        &relevant_ids,
        &mut gaps,
    )?;

    gaps.sort_by(|left, right| {
        left.surface
            .cmp(&right.surface)
            .then_with(|| left.reason.cmp(&right.reason))
    });
    gaps.dedup();
    let collection = SnapshotCollection {
        status: if gaps.is_empty() {
            CollectionStatus::Complete
        } else {
            CollectionStatus::Partial
        },
        api_calls: u64::try_from(budget.requests)?,
        response_bytes: u64::try_from(budget.response_bytes)?,
        gaps,
    };
    Ok(MergeReadinessSnapshot {
        schema: MERGE_READINESS_SNAPSHOT_SCHEMA.to_owned(),
        captured_at: request.captured_at.to_owned(),
        repository: RepositorySnapshot {
            database_id: repository.id,
            name_with_owner: repository.full_name,
            url: repository.html_url,
            default_branch: repository.default_branch,
            default_branch_head_sha: reference.object.sha.to_ascii_lowercase(),
        },
        collection,
        branch_policy,
        workflows,
        pull_requests,
    })
}

fn add_doctor_gap(
    gaps: &mut Vec<DoctorCollectionGap>,
    surface: DoctorCollectionSurface,
    reason: String,
) {
    gaps.push(DoctorCollectionGap { surface, reason });
}

fn doctor_surface_rank(surface: &DoctorCollectionSurface) -> u8 {
    match surface {
        DoctorCollectionSurface::Target => 0,
        DoctorCollectionSurface::Requirements => 1,
        DoctorCollectionSurface::CheckRuns => 2,
        DoctorCollectionSurface::CommitStatuses => 3,
    }
}

fn sort_doctor_gaps(gaps: &mut Vec<DoctorCollectionGap>) {
    gaps.sort_by(|left, right| {
        doctor_surface_rank(&left.surface)
            .cmp(&doctor_surface_rank(&right.surface))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    gaps.dedup_by(|left, right| left.surface == right.surface && left.reason == right.reason);
}

fn doctor_policy_kind_rank(kind: &DoctorPolicyKind) -> u8 {
    match kind {
        DoctorPolicyKind::Ruleset => 0,
        DoctorPolicyKind::BranchProtection => 1,
    }
}

fn validate_doctor_pull(
    pull: &ApiDoctorPullRequest,
    expected_number: u64,
    expected_url: &str,
) -> Result<()> {
    ensure!(
        pull.number == expected_number,
        "GitHub pull request number does not match the requested pull request"
    );
    ensure!(
        pull.html_url == expected_url,
        "GitHub pull request URL does not match the requested provider and repository"
    );
    ensure!(
        pull.state == "open",
        "pull-request doctor requires an open pull request"
    );
    ensure!(
        !pull.base.reference.is_empty()
            && pull.base.reference.len() <= 255
            && !pull.base.reference.chars().any(char::is_control),
        "GitHub pull request base ref is malformed"
    );
    ensure!(
        valid_doctor_sha(&pull.base.sha),
        "GitHub pull request base SHA must be a lowercase full Git object ID"
    );
    ensure!(
        valid_doctor_sha(&pull.head.sha),
        "GitHub pull request head SHA must be a lowercase full Git object ID"
    );
    if let Some(head_ref) = &pull.head.reference {
        ensure!(
            !head_ref.is_empty()
                && head_ref.len() <= 255
                && !head_ref.chars().any(char::is_control),
            "GitHub pull request head ref is malformed"
        );
    }
    if let Some(merge_commit_sha) = &pull.merge_commit_sha {
        ensure!(
            valid_doctor_sha(merge_commit_sha),
            "GitHub pull request test-merge SHA must be a lowercase full Git object ID"
        );
    }
    Ok(())
}

fn valid_doctor_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn collect_doctor_candidate_observation<A: GithubPullRequestDoctorApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    owner: &str,
    name: &str,
    repository: &ApiRepository,
    pull: &ApiDoctorPullRequest,
    gaps: &mut Vec<DoctorCollectionGap>,
) -> Result<Option<ApiDoctorGraphqlPullRequest>> {
    let variables = serde_json::json!({
        "owner": owner,
        "name": name,
        "number": pull.number,
    });
    let response = budget.graphql(api, DOCTOR_CANDIDATE_QUERY, &variables)?;
    if !successful(&response) {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Target,
            format!(
                "GitHub returned HTTP {} while reading the pull-request evaluation candidate",
                response.status
            ),
        );
        return Ok(None);
    }
    let envelope: ApiDoctorGraphqlEnvelope = parse_json(&response.body, "graphql")?;
    if !envelope.errors.is_empty() {
        for error in &envelope.errors {
            ensure!(
                !error.message.is_empty()
                    && error.message.len() <= 1_024
                    && !error.message.chars().any(char::is_control),
                "GitHub GraphQL returned a malformed error message"
            );
        }
        let summary = envelope
            .errors
            .iter()
            .take(3)
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Target,
            format!(
                "GitHub GraphQL could not expose the pull-request evaluation candidate ({} error(s)): {summary}",
                envelope.errors.len()
            ),
        );
        return Ok(None);
    }
    let Some(data) = envelope.data else {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Target,
            "GitHub GraphQL returned no data for the pull-request evaluation candidate".to_owned(),
        );
        return Ok(None);
    };
    let Some(graphql_repository) = data.repository else {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Target,
            "GitHub GraphQL did not expose the requested repository".to_owned(),
        );
        return Ok(None);
    };
    ensure!(
        graphql_repository
            .name_with_owner
            .eq_ignore_ascii_case(&repository.full_name)
            && graphql_repository.url == repository.html_url,
        "GraphQL repository identity does not match the REST repository; retry the doctor command"
    );
    let Some(graphql_pull) = graphql_repository.pull_request else {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Target,
            "GitHub GraphQL did not expose the requested pull request".to_owned(),
        );
        return Ok(None);
    };
    ensure!(
        graphql_pull.number == pull.number
            && graphql_pull.url == pull.html_url
            && graphql_pull.state == "OPEN"
            && graphql_pull.base_ref_name == pull.base.reference
            && graphql_pull
                .base_ref_oid
                .eq_ignore_ascii_case(&pull.base.sha)
            && graphql_pull
                .head_ref_oid
                .eq_ignore_ascii_case(&pull.head.sha),
        "GraphQL pull-request identity does not match REST; retry the doctor command"
    );
    ensure!(
        matches!(
            graphql_pull.mergeable.as_str(),
            "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
        ),
        "GitHub GraphQL returned an unknown mergeable state"
    );
    ensure!(
        !graphql_pull.merge_state_status.is_empty()
            && graphql_pull.merge_state_status.len() <= 64
            && !graphql_pull
                .merge_state_status
                .chars()
                .any(char::is_control),
        "GitHub GraphQL returned a malformed merge-state status"
    );
    if let Some(candidate) = &graphql_pull.potential_merge_commit {
        ensure!(
            valid_doctor_sha(&candidate.oid),
            "GitHub GraphQL test-merge SHA must be a lowercase full Git object ID"
        );
    }
    if let (Some(rest), Some(graphql)) = (
        pull.merge_commit_sha.as_deref(),
        graphql_pull
            .potential_merge_commit
            .as_ref()
            .map(|candidate| candidate.oid.as_str()),
    ) {
        ensure!(
            rest.eq_ignore_ascii_case(graphql),
            "REST and GraphQL test-merge candidates disagree; retry the doctor command"
        );
    }
    ensure!(
        graphql_pull.is_in_merge_queue || graphql_pull.merge_queue_entry.is_none(),
        "GitHub GraphQL returned a merge-queue entry without active queue membership; retry the doctor command"
    );
    ensure!(
        !graphql_pull.is_in_merge_queue || graphql_pull.is_merge_queue_enabled,
        "GitHub GraphQL reported a queued pull request without an enabled merge queue"
    );
    if let Some(entry) = &graphql_pull.merge_queue_entry {
        ensure!(
            !entry.id.is_empty()
                && entry.id.len() <= 255
                && !entry.id.chars().any(char::is_control),
            "GitHub GraphQL returned a malformed merge-queue entry ID"
        );
        ensure!(
            matches!(
                entry.state.as_str(),
                "QUEUED" | "AWAITING_CHECKS" | "MERGEABLE" | "UNMERGEABLE" | "LOCKED"
            ),
            "GitHub GraphQL returned an unknown merge-queue entry state"
        );
        ensure!(
            entry.position > 0
                && entry.position <= i32::MAX as u64
                && valid_doctor_sha(&entry.base_commit.oid)
                && valid_doctor_sha(&entry.head_commit.oid),
            "GitHub GraphQL returned malformed merge-queue candidate metadata"
        );
        ensure!(
            entry.pull_request.number == pull.number
                && entry
                    .pull_request
                    .head_ref_oid
                    .eq_ignore_ascii_case(&pull.head.sha),
            "merge-queue entry is bound to a different pull request; retry the doctor command"
        );
        ensure!(
            !entry.head_commit.oid.eq_ignore_ascii_case(&pull.head.sha)
                && !entry
                    .head_commit
                    .oid
                    .eq_ignore_ascii_case(&entry.base_commit.oid),
            "merge-queue candidate does not identify a distinct synthetic commit"
        );
    }
    Ok(Some(graphql_pull))
}

fn doctor_ruleset_policy(rule: &ApiEffectiveRule, repository_url: &str) -> DoctorPolicyRef {
    DoctorPolicyRef {
        kind: DoctorPolicyKind::Ruleset,
        id: rule.ruleset_id.to_string(),
        name: format!("ruleset {}", rule.ruleset_id),
        url: format!("{repository_url}/rules/{}", rule.ruleset_id),
    }
}

fn doctor_branch_protection_policy(branch: &str, repository_url: &str) -> DoctorPolicyRef {
    DoctorPolicyRef {
        kind: DoctorPolicyKind::BranchProtection,
        id: branch.to_owned(),
        name: format!("Branch protection for {branch}"),
        url: format!("{repository_url}/settings/branches"),
    }
}

fn add_doctor_requirement(
    requirements: &mut Vec<DoctorRequirement>,
    context: String,
    expected_app_id: Option<u64>,
    policy: DoctorPolicyRef,
) -> Result<()> {
    ensure!(
        !context.is_empty(),
        "required check context must not be empty"
    );
    if let Some(requirement) = requirements.iter_mut().find(|requirement| {
        requirement.context == context && requirement.expected_app_id == expected_app_id
    }) {
        if !requirement.policies.iter().any(|candidate| {
            candidate.kind == policy.kind
                && candidate.id == policy.id
                && candidate.name == policy.name
                && candidate.url == policy.url
        }) {
            requirement.policies.push(policy);
        }
    } else {
        requirements.push(DoctorRequirement {
            context,
            expected_app_id,
            policies: vec![policy],
        });
    }
    Ok(())
}

fn sort_doctor_requirements(requirements: &mut [DoctorRequirement]) {
    for requirement in requirements.iter_mut() {
        requirement.policies.sort_by(|left, right| {
            doctor_policy_kind_rank(&left.kind)
                .cmp(&doctor_policy_kind_rank(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.url.cmp(&right.url))
        });
        requirement.policies.dedup_by(|left, right| {
            left.kind == right.kind
                && left.id == right.id
                && left.name == right.name
                && left.url == right.url
        });
    }
    requirements.sort_by(|left, right| {
        left.context
            .cmp(&right.context)
            .then_with(|| left.expected_app_id.cmp(&right.expected_app_id))
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DoctorTargetRules {
    merge_queue: bool,
    required_workflows: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DoctorPolicySnapshot {
    target_rules: DoctorTargetRules,
    requirements: Vec<DoctorRequirement>,
    visibility_gaps: Vec<DoctorCollectionGap>,
}

fn candidate_target_identity(
    pull: &ApiDoctorPullRequest,
    graphql: Option<&ApiDoctorGraphqlPullRequest>,
    target_rules: DoctorTargetRules,
) -> CandidateTargetIdentity {
    let potential_merge_sha = graphql
        .and_then(|observation| observation.potential_merge_commit.as_ref())
        .map(|candidate| candidate.oid.to_ascii_lowercase())
        .or_else(|| {
            pull.merge_commit_sha
                .as_ref()
                .map(|sha| sha.to_ascii_lowercase())
        })
        .filter(|sha| !sha.eq_ignore_ascii_case(&pull.head.sha));
    let queue_entry = graphql
        .and_then(|observation| observation.merge_queue_entry.as_ref())
        .map(|entry| CandidateQueueEntry {
            id: entry.id.clone(),
            state: entry.state.clone(),
            position: entry.position,
            base_sha: entry.base_commit.oid.to_ascii_lowercase(),
            head_sha: entry.head_commit.oid.to_ascii_lowercase(),
        });
    CandidateTargetIdentity {
        state: pull.state.clone(),
        base_sha: pull.base.sha.to_ascii_lowercase(),
        head_sha: pull.head.sha.to_ascii_lowercase(),
        is_merge_queue_enabled: graphql
            .is_some_and(|observation| observation.is_merge_queue_enabled)
            || target_rules.merge_queue,
        is_in_merge_queue: graphql.is_some_and(|observation| observation.is_in_merge_queue),
        potential_merge_sha,
        queue_entry,
    }
}

fn candidate_signal_surface(complete: bool, observed: usize) -> Result<CandidateSignalSurface> {
    Ok(CandidateSignalSurface {
        collection: if complete {
            CandidateSignalCollectionStatus::Complete
        } else {
            CandidateSignalCollectionStatus::Gap
        },
        success: 0,
        other: u64::try_from(observed).context("candidate signal count exceeds u64")?,
    })
}

fn candidate_signals(
    sha: &str,
    check_runs: &[DoctorCheckRun],
    statuses: &[DoctorCommitStatus],
    gaps: &[DoctorCollectionGap],
) -> Result<CandidateSignals> {
    Ok(CandidateSignals {
        sha: sha.to_ascii_lowercase(),
        check_runs: candidate_signal_surface(
            !gaps
                .iter()
                .any(|gap| gap.surface == DoctorCollectionSurface::CheckRuns),
            check_runs.len(),
        )?,
        commit_statuses: candidate_signal_surface(
            !gaps
                .iter()
                .any(|gap| gap.surface == DoctorCollectionSurface::CommitStatuses),
            statuses.len(),
        )?,
    })
}

fn collect_doctor_effective_requirements<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    repository_url: &str,
    branch: &str,
    gaps: &mut Vec<DoctorCollectionGap>,
    requirements: &mut Vec<DoctorRequirement>,
) -> Result<DoctorTargetRules> {
    let mut target_rules = DoctorTargetRules::default();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/rules/branches/{}?per_page={PAGE_SIZE}&page={page}",
            encode_path_component(branch)
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::Requirements,
                format!(
                    "GitHub returned HTTP {} while reading effective rules for base branch {branch}",
                    response.status
                ),
            );
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::Target,
                "effective rules were not visible, so Doctor could not exclude merge-queue or required-workflow targets"
                    .to_owned(),
            );
            break;
        }

        let rules: Vec<ApiEffectiveRule> = parse_json(&response.body, &endpoint)?;
        let rule_count = rules.len();
        let has_next = has_next_page(response.link_header.as_deref());
        ensure!(
            rule_count <= PAGE_SIZE,
            "GitHub returned more effective rules than the requested page size"
        );
        for rule in &rules {
            match rule.kind.as_str() {
                "merge_queue" => target_rules.merge_queue = true,
                "workflows" => target_rules.required_workflows = true,
                _ => {}
            }
        }
        for rule in rules
            .iter()
            .filter(|rule| rule.kind == "required_status_checks")
        {
            let parameters = rule
                .parameters
                .clone()
                .context("required_status_checks rule is missing parameters")?;
            let parameters: ApiRequiredChecksParameters = serde_json::from_value(parameters)
                .context("failed to decode required_status_checks parameters")?;
            let policy = doctor_ruleset_policy(rule, repository_url);
            for check in parameters.required_status_checks {
                add_doctor_requirement(
                    requirements,
                    check.context,
                    check.integration_id,
                    policy.clone(),
                )?;
            }
        }
        if has_next && rule_count == 0 {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::Requirements,
                "effective-rule pagination returned an empty page with a next link".to_owned(),
            );
            break;
        }
        if !has_next {
            break;
        }
        if page == MAX_PAGES {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::Requirements,
                format!("effective-rule pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    Ok(target_rules)
}

fn collect_doctor_classic_requirements<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    repository_url: &str,
    branch: &str,
    gaps: &mut Vec<DoctorCollectionGap>,
    requirements: &mut Vec<DoctorRequirement>,
) -> Result<()> {
    let branch_endpoint = format!(
        "repos/{repository_path}/branches/{}",
        encode_path_component(branch)
    );
    let branch_response = budget.get(api, &branch_endpoint)?;
    let branch_protected = if successful(&branch_response) {
        let summary: ApiBranchSummary = parse_json(&branch_response.body, &branch_endpoint)?;
        ensure!(
            summary.name == branch,
            "GitHub branch identity does not match the pull request base ref"
        );
        Some(summary.protected)
    } else {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Requirements,
            format!(
                "GitHub returned HTTP {} while reading base branch {branch}",
                branch_response.status
            ),
        );
        None
    };

    if branch_protected == Some(false) {
        return Ok(());
    }

    let protection_endpoint = format!(
        "repos/{repository_path}/branches/{}/protection",
        encode_path_component(branch)
    );
    let protection_response = budget.get(api, &protection_endpoint)?;
    if !successful(&protection_response) {
        add_doctor_gap(
            gaps,
            DoctorCollectionSurface::Requirements,
            format!(
                "GitHub returned HTTP {} while reading classic branch protection for base branch {branch}; the policy may be absent or not visible to this token",
                protection_response.status
            ),
        );
        return Ok(());
    }
    let protection: ApiBranchProtection =
        parse_json(&protection_response.body, &protection_endpoint)?;
    let Some(required) = protection.required_status_checks else {
        return Ok(());
    };
    let policy = doctor_branch_protection_policy(branch, repository_url);
    let mut contexts_with_checks = BTreeSet::new();
    for check in required.checks {
        let expected_app_id = match check.app_id {
            None | Some(-1) => None,
            Some(value) => Some(
                u64::try_from(value)
                    .context("classic branch protection check has a negative app_id")?,
            ),
        };
        contexts_with_checks.insert(check.context.clone());
        add_doctor_requirement(requirements, check.context, expected_app_id, policy.clone())?;
    }
    for context in required.contexts {
        if !contexts_with_checks.contains(&context) {
            add_doctor_requirement(requirements, context, None, policy.clone())?;
        }
    }
    Ok(())
}

fn collect_doctor_policy_snapshot<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    repository_url: &str,
    branch: &str,
) -> Result<DoctorPolicySnapshot> {
    let mut visibility_gaps = Vec::new();
    let mut requirements = Vec::new();
    let target_rules = collect_doctor_effective_requirements(
        api,
        budget,
        repository_path,
        repository_url,
        branch,
        &mut visibility_gaps,
        &mut requirements,
    )?;
    collect_doctor_classic_requirements(
        api,
        budget,
        repository_path,
        repository_url,
        branch,
        &mut visibility_gaps,
        &mut requirements,
    )?;
    sort_doctor_requirements(&mut requirements);
    sort_doctor_gaps(&mut visibility_gaps);
    Ok(DoctorPolicySnapshot {
        target_rules,
        requirements,
        visibility_gaps,
    })
}

fn collect_doctor_check_runs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    head_sha: &str,
    gaps: &mut Vec<DoctorCollectionGap>,
) -> Result<Vec<DoctorCheckRun>> {
    let mut check_runs = Vec::new();
    let mut ids = BTreeSet::new();
    let mut expected_total = None;
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/commits/{}/check-runs?filter=latest&per_page={PAGE_SIZE}&page={page}",
            encode_path_component(head_sha)
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CheckRuns,
                format!("GitHub returned HTTP {} for {endpoint}", response.status),
            );
            break;
        }
        let page_body: ApiDoctorCheckRunList = parse_json(&response.body, &endpoint)?;
        if let Some(total) = expected_total {
            if total != page_body.total_count {
                add_doctor_gap(
                    gaps,
                    DoctorCollectionSurface::CheckRuns,
                    "check-run total_count changed during collection".to_owned(),
                );
                break;
            }
        } else {
            expected_total = Some(page_body.total_count);
        }
        let page_len = page_body.check_runs.len();
        for check in page_body.check_runs {
            ensure!(
                check.head_sha.eq_ignore_ascii_case(head_sha),
                "GitHub returned a check run for a different head SHA"
            );
            if !ids.insert(check.id) {
                add_doctor_gap(
                    gaps,
                    DoctorCollectionSurface::CheckRuns,
                    format!("GitHub returned duplicate check-run ID {}", check.id),
                );
                continue;
            }
            check_runs.push(DoctorCheckRun {
                id: check.id,
                url: check.html_url,
                name: check.name,
                app_id: check.app.as_ref().map(|app| app.id),
                app_slug: check.app.map(|app| app.slug),
                status: check.status,
                conclusion: check.conclusion,
            });
        }
        let observed = u64::try_from(check_runs.len())?;
        if observed == page_body.total_count {
            break;
        }
        if observed > page_body.total_count {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CheckRuns,
                "check-run response returned more unique results than total_count".to_owned(),
            );
            break;
        }
        if page_len < PAGE_SIZE {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CheckRuns,
                "check-run response declared more results than it returned".to_owned(),
            );
            break;
        }
        if page == MAX_PAGES {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CheckRuns,
                format!("check-run pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    check_runs.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.app_id.cmp(&right.app_id))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(check_runs)
}

fn collect_doctor_statuses<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    head_sha: &str,
    gaps: &mut Vec<DoctorCollectionGap>,
) -> Result<Vec<DoctorCommitStatus>> {
    let mut statuses = Vec::new();
    let mut contexts = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/commits/{}/statuses?per_page={PAGE_SIZE}&page={page}",
            encode_path_component(head_sha)
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CommitStatuses,
                format!("GitHub returned HTTP {} for {endpoint}", response.status),
            );
            break;
        }
        let batch: Vec<ApiCommitStatus> = parse_json(&response.body, &endpoint)?;
        let page_len = batch.len();
        for status in batch {
            if !ids.insert(status.id) {
                add_doctor_gap(
                    gaps,
                    DoctorCollectionSurface::CommitStatuses,
                    format!("GitHub returned duplicate commit-status ID {}", status.id),
                );
                continue;
            }
            if contexts.insert(status.context.clone()) {
                statuses.push(DoctorCommitStatus {
                    id: status.id,
                    url: status.url,
                    context: status.context,
                    creator_id: status.creator.as_ref().map(|creator| creator.id),
                    creator_login: status.creator.map(|creator| creator.login),
                    state: status.state,
                });
            }
        }
        if !has_next_page(response.link_header.as_deref()) {
            break;
        }
        if page_len == 0 {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CommitStatuses,
                "commit-status pagination returned an empty page with a next link".to_owned(),
            );
            break;
        }
        if page == MAX_PAGES {
            add_doctor_gap(
                gaps,
                DoctorCollectionSurface::CommitStatuses,
                format!("commit-status pagination exceeded {MAX_PAGES} pages"),
            );
        }
    }
    statuses.sort_by(|left, right| {
        left.context
            .cmp(&right.context)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(statuses)
}

fn collect_pull_request_doctor_snapshot_with_budget<A: GithubPullRequestDoctorApi>(
    request: PullRequestDoctorCollection<'_>,
    api: &mut A,
    budget: &mut ApiBudget,
) -> Result<PullRequestDoctorSnapshotV2> {
    validate_provider_url(request.provider_url)?;
    ensure!(
        !request.captured_at.is_empty(),
        "captured_at must not be empty"
    );
    ensure!(
        request.pull_request_number > 0,
        "pull_request_number must be greater than zero"
    );
    let (owner, name) = parse_repository(request.repository)?;
    let repository_path = format!(
        "{}/{}",
        encode_path_component(&owner),
        encode_path_component(&name)
    );
    let pull_endpoint = format!(
        "repos/{repository_path}/pulls/{}",
        request.pull_request_number
    );
    let pull_response = budget.get(api, &pull_endpoint)?;
    ensure!(
        successful(&pull_response),
        "GitHub returned HTTP {} for {pull_endpoint}",
        pull_response.status
    );
    let pull: ApiDoctorPullRequest = parse_json(&pull_response.body, &pull_endpoint)?;

    let repository_endpoint = format!("repos/{repository_path}");
    let repository_response = budget.get(api, &repository_endpoint)?;
    ensure!(
        successful(&repository_response),
        "GitHub returned HTTP {} for {repository_endpoint}",
        repository_response.status
    );
    let repository: ApiRepository = parse_json(&repository_response.body, &repository_endpoint)?;
    ensure!(
        repository
            .full_name
            .eq_ignore_ascii_case(request.repository),
        "GitHub repository identity does not match the requested repository"
    );
    ensure!(
        repository.html_url == format!("{}/{}", request.provider_url, repository.full_name),
        "GitHub repository URL does not match the requested provider"
    );
    let pull_url = format!(
        "{}/{}/pull/{}",
        request.provider_url, repository.full_name, request.pull_request_number
    );
    validate_doctor_pull(&pull, request.pull_request_number, &pull_url)?;

    let mut gaps = Vec::new();
    let graphql_pull = collect_doctor_candidate_observation(
        api,
        budget,
        &owner,
        &name,
        &repository,
        &pull,
        &mut gaps,
    )?;
    let initial_policy = collect_doctor_policy_snapshot(
        api,
        budget,
        &repository_path,
        &repository.html_url,
        &pull.base.reference,
    )?;
    let target_rules = initial_policy.target_rules;
    let requirements = initial_policy.requirements.clone();
    gaps.extend(initial_policy.visibility_gaps.iter().cloned());
    if target_rules.required_workflows {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "effective rules require workflows whose expected check identities this Doctor version does not yet collect"
                .to_owned(),
        );
    }
    if target_rules.merge_queue && graphql_pull.is_none() {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "the base branch uses a merge queue, but Doctor could not observe whether this pull request currently has a queue candidate"
                .to_owned(),
        );
    }
    let queue_entry = graphql_pull
        .as_ref()
        .and_then(|observation| observation.merge_queue_entry.as_ref());
    let mut selection_signals = Vec::new();
    let (mut evaluation, signal_sha, check_runs, statuses) = if let Some(entry) = queue_entry {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "merge-queue candidate identity came from GraphQL mergeQueueEntry.headCommit; this polling source is not yet cross-validated against a merge_group webhook delivery"
                .to_owned(),
        );
        let check_runs = collect_doctor_check_runs(
            api,
            budget,
            &repository_path,
            &entry.head_commit.oid,
            &mut gaps,
        )?;
        let statuses = collect_doctor_statuses(
            api,
            budget,
            &repository_path,
            &entry.head_commit.oid,
            &mut gaps,
        )?;
        selection_signals.push(candidate_signals(
            &entry.head_commit.oid,
            &check_runs,
            &statuses,
            &gaps,
        )?);
        (
            DoctorEvaluationTarget {
                kind: DoctorEvaluationTargetKind::MergeGroup,
                resolution: DoctorEvaluationTargetResolution::Provisional,
                sha: entry.head_commit.oid.to_ascii_lowercase(),
                base_sha: Some(entry.base_commit.oid.to_ascii_lowercase()),
                queue_entry_id: Some(entry.id.clone()),
                queue_state: Some(entry.state.to_ascii_lowercase()),
            },
            entry.head_commit.oid.clone(),
            check_runs,
            statuses,
        )
    } else if graphql_pull
        .as_ref()
        .is_some_and(|observation| observation.is_in_merge_queue)
    {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "GitHub reports that the pull request is in the merge queue but did not expose its queue entry candidate"
                .to_owned(),
        );
        let check_runs =
            collect_doctor_check_runs(api, budget, &repository_path, &pull.head.sha, &mut gaps)?;
        let statuses =
            collect_doctor_statuses(api, budget, &repository_path, &pull.head.sha, &mut gaps)?;
        selection_signals.push(candidate_signals(
            &pull.head.sha,
            &check_runs,
            &statuses,
            &gaps,
        )?);
        (
            DoctorEvaluationTarget {
                kind: DoctorEvaluationTargetKind::PrHead,
                resolution: DoctorEvaluationTargetResolution::Provisional,
                sha: pull.head.sha.to_ascii_lowercase(),
                base_sha: None,
                queue_entry_id: None,
                queue_state: None,
            },
            pull.head.sha.clone(),
            check_runs,
            statuses,
        )
    } else {
        let head_check_runs =
            collect_doctor_check_runs(api, budget, &repository_path, &pull.head.sha, &mut gaps)?;
        let head_statuses =
            collect_doctor_statuses(api, budget, &repository_path, &pull.head.sha, &mut gaps)?;
        selection_signals.push(candidate_signals(
            &pull.head.sha,
            &head_check_runs,
            &head_statuses,
            &gaps,
        )?);
        let test_merge_sha = graphql_pull
            .as_ref()
            .and_then(|observation| observation.potential_merge_commit.as_ref())
            .map(|candidate| candidate.oid.as_str())
            .or(pull.merge_commit_sha.as_deref());
        let mut selected_test_merge = None;
        if let Some(merge_commit_sha) = test_merge_sha {
            if merge_commit_sha.eq_ignore_ascii_case(&pull.head.sha) {
                add_doctor_gap(
                    &mut gaps,
                    DoctorCollectionSurface::Target,
                    "GitHub returned the PR head as its test-merge SHA, so Doctor could not distinguish the active check target"
                        .to_owned(),
                );
            } else {
                let mut target_probe_gaps = Vec::new();
                let merge_check_runs = collect_doctor_check_runs(
                    api,
                    budget,
                    &repository_path,
                    merge_commit_sha,
                    &mut target_probe_gaps,
                )?;
                let merge_statuses = collect_doctor_statuses(
                    api,
                    budget,
                    &repository_path,
                    merge_commit_sha,
                    &mut target_probe_gaps,
                )?;
                selection_signals.push(candidate_signals(
                    merge_commit_sha,
                    &merge_check_runs,
                    &merge_statuses,
                    &target_probe_gaps,
                )?);
                let observed_test_merge_signal =
                    !merge_check_runs.is_empty() || !merge_statuses.is_empty();
                if observed_test_merge_signal {
                    selected_test_merge = Some((
                        DoctorEvaluationTarget {
                            kind: DoctorEvaluationTargetKind::TestMerge,
                            resolution: DoctorEvaluationTargetResolution::Selected,
                            sha: merge_commit_sha.to_ascii_lowercase(),
                            base_sha: Some(pull.base.sha.to_ascii_lowercase()),
                            queue_entry_id: None,
                            queue_state: None,
                        },
                        merge_commit_sha.to_owned(),
                        merge_check_runs,
                        merge_statuses,
                    ));
                }
                for gap in target_probe_gaps {
                    add_doctor_gap(
                        &mut gaps,
                        DoctorCollectionSurface::Target,
                        format!(
                            "could not completely inspect test-merge commit {merge_commit_sha}: {}",
                            gap.reason
                        ),
                    );
                }
            }
        } else {
            add_doctor_gap(
                &mut gaps,
                DoctorCollectionSurface::Target,
                "GitHub did not provide a test-merge SHA, so Doctor could not prove that the PR head is the active check target"
                    .to_owned(),
            );
        }
        if let Some(selected) = selected_test_merge {
            selected
        } else {
            (
                DoctorEvaluationTarget {
                    kind: DoctorEvaluationTargetKind::PrHead,
                    resolution: DoctorEvaluationTargetResolution::Provisional,
                    sha: pull.head.sha.to_ascii_lowercase(),
                    base_sha: None,
                    queue_entry_id: None,
                    queue_state: None,
                },
                pull.head.sha.clone(),
                head_check_runs,
                head_statuses,
            )
        }
    };

    let final_pull_response = budget.get(api, &pull_endpoint)?;
    ensure!(
        successful(&final_pull_response),
        "GitHub returned HTTP {} while re-reading {pull_endpoint}",
        final_pull_response.status
    );
    let final_pull: ApiDoctorPullRequest = parse_json(&final_pull_response.body, &pull_endpoint)?;
    ensure!(
        final_pull == pull,
        "pull request target changed during collection; retry the doctor command"
    );
    let final_graphql_pull = collect_doctor_candidate_observation(
        api,
        budget,
        &owner,
        &name,
        &repository,
        &final_pull,
        &mut gaps,
    )
    .context(
        "failed to revalidate the pull-request evaluation candidate; retry the doctor command",
    )?;
    if graphql_pull.is_none() || final_graphql_pull.is_none() {
        evaluation.resolution = DoctorEvaluationTargetResolution::Provisional;
    }
    if final_graphql_pull.is_none() {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "Doctor could not revalidate the pull-request evaluation candidate after collecting its signals; retry the doctor command"
                .to_owned(),
        );
    }

    let candidate_selection = select_candidate(&CandidateSelectionInput {
        before: candidate_target_identity(&pull, graphql_pull.as_ref(), target_rules),
        after: candidate_target_identity(&final_pull, final_graphql_pull.as_ref(), target_rules),
        signals: selection_signals,
    })?;
    match candidate_selection.status {
        CandidateSelectionStatus::Retry => {
            bail!(
                "pull-request evaluation candidate changed during collection; retry the doctor command"
            )
        }
        CandidateSelectionStatus::Inconclusive => ensure!(
            evaluation.resolution == DoctorEvaluationTargetResolution::Provisional,
            "candidate selector was inconclusive but collector marked the target selected"
        ),
        CandidateSelectionStatus::Selected => {
            let candidate_kind = candidate_selection
                .candidate_kind
                .context("selected candidate is missing its kind")?;
            let expected_kind = match candidate_kind {
                CandidateKind::PrHead => DoctorEvaluationTargetKind::PrHead,
                CandidateKind::TestMerge => DoctorEvaluationTargetKind::TestMerge,
                CandidateKind::MergeGroup => DoctorEvaluationTargetKind::MergeGroup,
            };
            ensure!(
                evaluation.kind == expected_kind
                    && candidate_selection.sha.as_deref() == Some(evaluation.sha.as_str())
                    && candidate_selection.base_sha == evaluation.base_sha
                    && candidate_selection.queue_entry_id == evaluation.queue_entry_id,
                "collector evaluation target disagrees with the benchmarked candidate selector"
            );
        }
    }

    let final_policy = collect_doctor_policy_snapshot(
        api,
        budget,
        &repository_path,
        &repository.html_url,
        &pull.base.reference,
    )
    .context("failed to revalidate base-branch policy; retry the doctor command")?;
    ensure!(
        final_policy == initial_policy,
        "base-branch policy or its visibility changed during collection; retry the doctor command"
    );

    sort_doctor_gaps(&mut gaps);
    let collection = DoctorCollection {
        status: if gaps.is_empty() {
            DoctorCollectionStatus::Complete
        } else {
            DoctorCollectionStatus::Partial
        },
        api_calls: u64::try_from(budget.requests)?,
        response_bytes: u64::try_from(budget.response_bytes)?,
        gaps,
    };
    Ok(PullRequestDoctorSnapshotV2 {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_V2_SCHEMA.to_owned(),
        captured_at: request.captured_at.to_owned(),
        provider_url: request.provider_url.to_owned(),
        repository: repository.full_name,
        target: DoctorTargetV2 {
            number: pull.number,
            url: pull.html_url,
            base_ref: pull.base.reference,
            base_sha: pull.base.sha.to_ascii_lowercase(),
            head_sha: pull.head.sha.to_ascii_lowercase(),
            evaluation,
        },
        signal_sha,
        collection,
        requirements,
        check_runs,
        statuses,
    })
}

pub fn collect_pull_request_doctor_snapshot<A: GithubPullRequestDoctorApi>(
    request: PullRequestDoctorCollection<'_>,
    api: &mut A,
) -> Result<PullRequestDoctorSnapshotV2> {
    let mut budget = ApiBudget::new();
    collect_pull_request_doctor_snapshot_with_budget(request, api, &mut budget)
}

const MAX_DOCTOR_WORKFLOW_INVESTIGATIONS: usize = 25;

fn collect_doctor_producer_check_runs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    sha: &str,
    check_name: &str,
    app_id: u64,
) -> Result<(Vec<ApiDoctorProducerCheckRun>, Option<String>)> {
    let mut check_runs = Vec::new();
    let mut ids = BTreeSet::new();
    let mut expected_total = None;
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/commits/{}/check-runs?check_name={}&app_id={app_id}&filter=all&per_page={PAGE_SIZE}&page={page}",
            encode_path_component(sha),
            encode_path_component(check_name),
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                check_runs,
                Some(format!(
                    "GitHub returned HTTP {} while reading producer check runs on {sha}",
                    response.status
                )),
            ));
        }
        let page_body: ApiDoctorProducerCheckRunList = parse_json(&response.body, &endpoint)?;
        if let Some(total) = expected_total {
            if total != page_body.total_count {
                return Ok((
                    check_runs,
                    Some("producer check-run total_count changed during collection".to_owned()),
                ));
            }
        } else {
            expected_total = Some(page_body.total_count);
        }
        let page_len = page_body.check_runs.len();
        for check in page_body.check_runs {
            ensure!(
                check.head_sha.eq_ignore_ascii_case(sha),
                "GitHub returned a producer check run for a different SHA"
            );
            ensure!(
                check.name == check_name && check.app.as_ref().is_some_and(|app| app.id == app_id),
                "GitHub returned a producer check run outside the exact name and App filter"
            );
            if !ids.insert(check.id) {
                return Ok((
                    check_runs,
                    Some(format!(
                        "GitHub returned duplicate producer check-run ID {}",
                        check.id
                    )),
                ));
            }
            check_runs.push(check);
        }
        let observed = u64::try_from(check_runs.len())?;
        if observed == page_body.total_count {
            break;
        }
        if observed > page_body.total_count || page_len < PAGE_SIZE {
            return Ok((
                check_runs,
                Some("producer check-run response declared a different result count".to_owned()),
            ));
        }
        if page == MAX_PAGES {
            return Ok((
                check_runs,
                Some(format!(
                    "producer check-run pagination exceeded {MAX_PAGES} pages"
                )),
            ));
        }
    }
    check_runs.sort_by_key(|check| check.id);
    Ok((check_runs, None))
}

fn collect_doctor_workflow_runs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    sha: &str,
    check_suite_id: Option<u64>,
) -> Result<(Vec<ApiDoctorWorkflowRun>, Option<String>)> {
    let mut runs = Vec::new();
    let mut ids = BTreeSet::new();
    let mut expected_total = None;
    for page in 1..=MAX_PAGES {
        let endpoint = match check_suite_id {
            Some(check_suite_id) => format!(
                "repos/{repository_path}/actions/runs?check_suite_id={check_suite_id}&per_page={PAGE_SIZE}&page={page}"
            ),
            None => format!(
                "repos/{repository_path}/actions/runs?head_sha={}&per_page={PAGE_SIZE}&page={page}",
                encode_path_component(sha)
            ),
        };
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                runs,
                Some(format!(
                    "GitHub returned HTTP {} while reading workflow runs on {sha}",
                    response.status
                )),
            ));
        }
        let page_body: ApiDoctorWorkflowRunList = parse_json(&response.body, &endpoint)?;
        if let Some(total) = expected_total {
            if total != page_body.total_count {
                return Ok((
                    runs,
                    Some("workflow-run total_count changed during collection".to_owned()),
                ));
            }
        } else {
            expected_total = Some(page_body.total_count);
        }
        let page_len = page_body.workflow_runs.len();
        for run in page_body.workflow_runs {
            ensure!(
                run.head_sha.eq_ignore_ascii_case(sha),
                "GitHub returned a workflow run for a different SHA"
            );
            if let Some(check_suite_id) = check_suite_id {
                ensure!(
                    run.check_suite_id == check_suite_id,
                    "GitHub returned a workflow run for a different check suite"
                );
            }
            ensure!(run.run_attempt > 0, "workflow run attempt must be positive");
            ensure!(
                !run.path.is_empty() && !run.event.is_empty() && !run.status.is_empty(),
                "GitHub returned malformed workflow-run identity"
            );
            ensure!(
                ids.insert(run.id),
                "GitHub returned a duplicate workflow-run ID"
            );
            runs.push(run);
        }
        let observed = u64::try_from(runs.len())?;
        if observed == page_body.total_count {
            break;
        }
        if observed > page_body.total_count || page_len < PAGE_SIZE {
            return Ok((
                runs,
                Some("workflow-run response declared a different result count".to_owned()),
            ));
        }
        if page == MAX_PAGES {
            return Ok((
                runs,
                Some(format!(
                    "workflow-run pagination exceeded {MAX_PAGES} pages"
                )),
            ));
        }
    }
    runs.sort_by_key(|run| run.id);
    Ok((runs, None))
}

fn collect_doctor_workflow_jobs<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    run_id: u64,
) -> Result<(Vec<ApiDoctorWorkflowJob>, Option<String>)> {
    let mut jobs = Vec::new();
    let mut ids = BTreeSet::new();
    let mut expected_total = None;
    for page in 1..=MAX_PAGES {
        let endpoint = format!(
            "repos/{repository_path}/actions/runs/{run_id}/jobs?filter=all&per_page={PAGE_SIZE}&page={page}"
        );
        let response = budget.get(api, &endpoint)?;
        if !successful(&response) {
            return Ok((
                jobs,
                Some(format!(
                    "GitHub returned HTTP {} while reading jobs for workflow run {run_id}",
                    response.status
                )),
            ));
        }
        let page_body: ApiDoctorWorkflowJobList = parse_json(&response.body, &endpoint)?;
        if let Some(total) = expected_total {
            if total != page_body.total_count {
                return Ok((
                    jobs,
                    Some("workflow-job total_count changed during collection".to_owned()),
                ));
            }
        } else {
            expected_total = Some(page_body.total_count);
        }
        let page_len = page_body.jobs.len();
        for job in page_body.jobs {
            ensure!(job.run_attempt > 0, "workflow job attempt must be positive");
            ensure!(
                !job.name.is_empty() && !job.check_run_url.is_empty(),
                "GitHub returned malformed workflow-job identity"
            );
            ensure!(
                ids.insert(job.id),
                "GitHub returned a duplicate workflow-job ID"
            );
            jobs.push(job);
        }
        let observed = u64::try_from(jobs.len())?;
        if observed == page_body.total_count {
            break;
        }
        if observed > page_body.total_count || page_len < PAGE_SIZE {
            return Ok((
                jobs,
                Some("workflow-job response declared a different result count".to_owned()),
            ));
        }
        if page == MAX_PAGES {
            return Ok((
                jobs,
                Some(format!(
                    "workflow-job pagination exceeded {MAX_PAGES} pages"
                )),
            ));
        }
    }
    jobs.sort_by_key(|job| job.id);
    Ok((jobs, None))
}

fn direct_workflow_path(path: &str) -> bool {
    path.strip_prefix(".github/workflows/").is_some_and(|name| {
        !name.is_empty()
            && !name.contains('/')
            && !matches!(name, "." | "..")
            && (name.ends_with(".yml") || name.ends_with(".yaml"))
    })
}

fn collect_doctor_workflow_inventory<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    signal_sha: &str,
) -> Result<(Option<DoctorWorkflowInventory>, Option<String>)> {
    let directory_endpoint = format!(
        "repos/{repository_path}/contents/.github/workflows?ref={}",
        encode_path_component(signal_sha)
    );
    let directory_response = budget.get(api, &directory_endpoint)?;
    if !successful(&directory_response) {
        return Ok((
            None,
            Some(format!(
                "GitHub returned HTTP {} while listing workflows at evaluation SHA {signal_sha}",
                directory_response.status
            )),
        ));
    }
    if directory_response.link_header.is_some() {
        return Ok((
            None,
            Some("workflow directory response advertised another page".to_owned()),
        ));
    }
    let entries: Vec<ApiDoctorWorkflowDirectoryEntry> =
        parse_json(&directory_response.body, &directory_endpoint)?;
    if entries.len() >= MAX_DOCTOR_WORKFLOW_DIRECTORY_ENTRIES {
        return Ok((
            None,
            Some(format!(
                "workflow directory contains at least {MAX_DOCTOR_WORKFLOW_DIRECTORY_ENTRIES} entries; completeness is not proven"
            )),
        ));
    }

    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    for entry in entries {
        ensure!(
            entry.path.starts_with(".github/workflows/"),
            "GitHub returned a workflow-directory entry outside the requested directory"
        );
        if !(entry.path.ends_with(".yml") || entry.path.ends_with(".yaml")) {
            continue;
        }
        if entry.kind != "file" || !direct_workflow_path(&entry.path) {
            return Ok((
                None,
                Some(format!(
                    "workflow entry {} is not a direct regular workflow file",
                    entry.path
                )),
            ));
        }
        ensure!(
            valid_doctor_sha(&entry.sha),
            "GitHub returned a workflow file with an invalid blob SHA"
        );
        ensure!(
            paths.insert(entry.path.clone()),
            "GitHub returned a duplicate workflow path"
        );

        let content_endpoint = format!(
            "repos/{repository_path}/contents/{}?ref={}",
            encode_repository_path(&entry.path),
            encode_path_component(signal_sha)
        );
        let content_response = budget.get(api, &content_endpoint)?;
        if !successful(&content_response) {
            return Ok((
                None,
                Some(format!(
                    "GitHub returned HTTP {} for {} at evaluation SHA {signal_sha}",
                    content_response.status, entry.path
                )),
            ));
        }
        let content: ApiDoctorWorkflowContent =
            parse_json(&content_response.body, &content_endpoint)?;
        ensure!(
            content.path == entry.path && content.sha == entry.sha,
            "workflow content identity did not match its exact-SHA directory entry"
        );
        let bytes = decode_workflow_content(ApiContent {
            kind: content.kind,
            encoding: content.encoding,
            content: content.content,
        })?;
        let definition =
            match parse_doctor_workflow_definition(&bytes, &entry.path, WorkflowState::Active) {
                Ok(definition) => definition,
                Err(error) => {
                    return Ok((None, Some(format!("{}: {error:#}", entry.path))));
                }
            };
        files.push(DoctorWorkflowInventoryFile {
            blob_sha: entry.sha.to_ascii_lowercase(),
            jobs: definition.jobs,
            path: definition.path,
            triggers: definition.triggers,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((
        Some(DoctorWorkflowInventory {
            sha: signal_sha.to_owned(),
            files,
        }),
        None,
    ))
}

fn doctor_workflow_state(value: &str) -> Option<WorkflowState> {
    match value {
        "active" => Some(WorkflowState::Active),
        "disabled_inactivity" => Some(WorkflowState::DisabledInactivity),
        "disabled_manually" => Some(WorkflowState::DisabledManually),
        _ => None,
    }
}

fn doctor_run_status(value: &str) -> Option<WorkflowRunStatus> {
    match value {
        "completed" => Some(WorkflowRunStatus::Completed),
        "in_progress" => Some(WorkflowRunStatus::InProgress),
        "pending" => Some(WorkflowRunStatus::Pending),
        "queued" => Some(WorkflowRunStatus::Queued),
        "requested" => Some(WorkflowRunStatus::Requested),
        "waiting" => Some(WorkflowRunStatus::Waiting),
        _ => None,
    }
}

fn doctor_run_conclusion(value: &str) -> Option<WorkflowRunConclusion> {
    match value {
        "action_required" => Some(WorkflowRunConclusion::ActionRequired),
        "cancelled" => Some(WorkflowRunConclusion::Cancelled),
        "failure" => Some(WorkflowRunConclusion::Failure),
        "neutral" => Some(WorkflowRunConclusion::Neutral),
        "skipped" => Some(WorkflowRunConclusion::Skipped),
        "stale" => Some(WorkflowRunConclusion::Stale),
        "startup_failure" => Some(WorkflowRunConclusion::StartupFailure),
        "success" => Some(WorkflowRunConclusion::Success),
        "timed_out" => Some(WorkflowRunConclusion::TimedOut),
        _ => None,
    }
}

fn doctor_run_observation(run: &ApiDoctorWorkflowRun) -> Option<WorkflowRunObservation> {
    let event = match run.event.as_str() {
        "merge_group" => WorkflowRunEvent::MergeGroup,
        "pull_request" => WorkflowRunEvent::PullRequest,
        _ => return None,
    };
    let status = doctor_run_status(&run.status)?;
    let conclusion = match run.conclusion.as_deref() {
        Some(value) => Some(doctor_run_conclusion(value)?),
        None => None,
    };
    Some(WorkflowRunObservation {
        conclusion,
        event,
        head_sha: run.head_sha.to_ascii_lowercase(),
        status,
        workflow_path: run.path.clone(),
    })
}

fn add_workflow_gap(
    gaps: &mut Vec<DoctorWorkflowCollectionGap>,
    requirement: &DoctorRequirementKey,
    code: &str,
    reason: impl Into<String>,
) {
    gaps.push(DoctorWorkflowCollectionGap {
        requirement: requirement.clone(),
        code: code.to_owned(),
        reason: reason.into(),
    });
}

fn workflow_run_path_matches(run_path: &str, workflow_path: &str) -> bool {
    run_path == workflow_path
        || run_path
            .strip_prefix(workflow_path)
            .is_some_and(|suffix| suffix.starts_with('@') && suffix.len() > 1)
}

fn v3_from_v2(
    snapshot: PullRequestDoctorSnapshotV2,
    workflow_collection: DoctorWorkflowCollection,
    workflow_trigger_investigations: Vec<DoctorWorkflowTriggerInvestigation>,
) -> PullRequestDoctorSnapshotV3 {
    PullRequestDoctorSnapshotV3 {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_V3_SCHEMA.to_owned(),
        captured_at: snapshot.captured_at,
        provider_url: snapshot.provider_url,
        repository: snapshot.repository,
        target: snapshot.target,
        signal_sha: snapshot.signal_sha,
        collection: snapshot.collection,
        requirements: snapshot.requirements,
        check_runs: snapshot.check_runs,
        statuses: snapshot.statuses,
        workflow_collection,
        workflow_trigger_investigations,
    }
}

fn doctor_snapshot_semantics_equal(
    before: &PullRequestDoctorSnapshotV2,
    after: &PullRequestDoctorSnapshotV2,
) -> bool {
    before.schema == after.schema
        && before.captured_at == after.captured_at
        && before.provider_url == after.provider_url
        && before.repository == after.repository
        && before.target == after.target
        && before.signal_sha == after.signal_sha
        && before.collection.status == after.collection.status
        && before.collection.gaps == after.collection.gaps
        && before.requirements == after.requirements
        && before.check_runs == after.check_runs
        && before.statuses == after.statuses
}

fn not_applicable_workflow_collection() -> DoctorWorkflowCollection {
    DoctorWorkflowCollection {
        status: DoctorWorkflowCollectionStatus::NotApplicable,
        api_calls: 0,
        response_bytes: 0,
        inventory: None,
        probes: Vec::new(),
        gaps: Vec::new(),
    }
}

fn collect_doctor_workflow_investigations<A: GithubPullRequestDoctorApi>(
    request: &PullRequestDoctorCollection<'_>,
    snapshot: &PullRequestDoctorSnapshotV2,
    requirements: &[DoctorRequirementKey],
    api: &mut A,
    budget: &mut ApiBudget,
) -> Result<(
    DoctorWorkflowCollection,
    Vec<DoctorWorkflowTriggerInvestigation>,
)> {
    let starting_requests = budget.requests;
    let starting_response_bytes = budget.response_bytes;
    let mut gaps = Vec::new();
    let mut investigations = requirements
        .iter()
        .map(|requirement| DoctorWorkflowTriggerInvestigation {
            requirement: requirement.clone(),
            producer: None,
            input: None,
        })
        .collect::<Vec<_>>();
    if investigations.len() > MAX_DOCTOR_WORKFLOW_INVESTIGATIONS {
        for requirement in requirements {
            add_workflow_gap(
                &mut gaps,
                requirement,
                "investigation_limit_exceeded",
                format!(
                    "Doctor limits one run to {MAX_DOCTOR_WORKFLOW_INVESTIGATIONS} pinned missing checks"
                ),
            );
        }
        return Ok((
            DoctorWorkflowCollection {
                status: DoctorWorkflowCollectionStatus::Partial,
                api_calls: 0,
                response_bytes: 0,
                inventory: None,
                probes: Vec::new(),
                gaps,
            },
            investigations,
        ));
    }

    let (owner, name) = parse_repository(request.repository)?;
    let repository_path = format!(
        "{}/{}",
        encode_path_component(&owner),
        encode_path_component(&name)
    );
    let pull_endpoint = format!(
        "repos/{repository_path}/pulls/{}",
        request.pull_request_number
    );
    let pull_response = budget.get(api, &pull_endpoint)?;
    if !successful(&pull_response) {
        for requirement in requirements {
            add_workflow_gap(
                &mut gaps,
                requirement,
                "producer_candidate_unavailable",
                format!(
                    "GitHub returned HTTP {} while refreshing producer candidates",
                    pull_response.status
                ),
            );
        }
        return Ok((
            DoctorWorkflowCollection {
                status: DoctorWorkflowCollectionStatus::Partial,
                api_calls: u64::try_from(budget.requests - starting_requests)?,
                response_bytes: u64::try_from(budget.response_bytes - starting_response_bytes)?,
                inventory: None,
                probes: Vec::new(),
                gaps,
            },
            investigations,
        ));
    }
    let pull: ApiDoctorPullRequest = parse_json(&pull_response.body, &pull_endpoint)?;
    validate_doctor_pull(&pull, snapshot.target.number, &snapshot.target.url)?;
    ensure!(
        pull.base.reference == snapshot.target.base_ref
            && pull
                .base
                .sha
                .eq_ignore_ascii_case(&snapshot.target.base_sha)
            && pull
                .head
                .sha
                .eq_ignore_ascii_case(&snapshot.target.head_sha),
        "pull request changed before workflow producer collection; retry the doctor command"
    );

    let declared_probes = expected_workflow_probes(&pull, &snapshot.signal_sha);
    ensure!(
        !declared_probes.is_empty(),
        "merge-group workflow diagnosis requires a distinct producer candidate"
    );

    let (workflow_inventory, workflow_inventory_gap) =
        collect_doctor_workflow_inventory(api, budget, &repository_path, &snapshot.signal_sha)?;
    if let Some(reason) = &workflow_inventory_gap {
        for requirement in requirements {
            add_workflow_gap(
                &mut gaps,
                requirement,
                "workflow_inventory_incomplete",
                reason,
            );
        }
    }

    let (target_runs, target_run_gap) =
        collect_doctor_workflow_runs(api, budget, &repository_path, &snapshot.signal_sha, None)?;
    if let Some(reason) = &target_run_gap {
        for requirement in requirements {
            add_workflow_gap(
                &mut gaps,
                requirement,
                "target_workflow_runs_incomplete",
                reason,
            );
        }
    }

    let mut run_cache: BTreeMap<
        (String, u64),
        std::result::Result<Vec<ApiDoctorWorkflowRun>, String>,
    > = BTreeMap::new();
    let mut job_cache: BTreeMap<u64, std::result::Result<Vec<ApiDoctorWorkflowJob>, String>> =
        BTreeMap::new();
    for investigation in &mut investigations {
        let expected_app_id = investigation
            .requirement
            .expected_app_id
            .context("workflow investigation requires a pinned App ID")?;
        let mut matching_checks = Vec::new();
        let mut producer_checks_complete = true;
        for probe in &declared_probes {
            let (checks, gap) = collect_doctor_producer_check_runs(
                api,
                budget,
                &repository_path,
                &probe.sha,
                &investigation.requirement.context,
                expected_app_id,
            )?;
            if let Some(reason) = gap {
                producer_checks_complete = false;
                add_workflow_gap(
                    &mut gaps,
                    &investigation.requirement,
                    "producer_check_runs_incomplete",
                    format!("{}: {reason}", probe.sha),
                );
            }
            for check in checks {
                if check
                    .app
                    .as_ref()
                    .is_some_and(|app| app.slug == "github-actions")
                {
                    matching_checks.push((probe.sha.clone(), check));
                }
            }
        }
        if !producer_checks_complete {
            continue;
        }
        if matching_checks.is_empty() {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "github_actions_producer_not_observed",
                "No prior candidate exposed the required context from the pinned GitHub Actions App",
            );
            continue;
        }
        if matching_checks
            .iter()
            .any(|(_, check)| check.check_suite.is_none())
        {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "producer_check_suite_missing",
                "A matching GitHub Actions check run did not expose check_suite.id",
            );
            continue;
        }

        let mut links = Vec::new();
        let mut run_lookup_failed = false;
        for (source_sha, check) in matching_checks {
            let check_suite_id = check
                .check_suite
                .as_ref()
                .context("matching producer check is missing check_suite.id")?
                .id;
            let cache_key = (source_sha.clone(), check_suite_id);
            if let Entry::Vacant(entry) = run_cache.entry(cache_key.clone()) {
                let (runs, gap) = collect_doctor_workflow_runs(
                    api,
                    budget,
                    &repository_path,
                    &source_sha,
                    Some(check_suite_id),
                )?;
                let value = match gap {
                    Some(reason) => Err(reason),
                    None => Ok(runs),
                };
                entry.insert(value);
            }
            match &run_cache[&cache_key] {
                Ok(runs) if runs.len() == 1 => {
                    let run = runs[0].clone();
                    if let Entry::Vacant(entry) = job_cache.entry(run.id) {
                        let (jobs, gap) =
                            collect_doctor_workflow_jobs(api, budget, &repository_path, run.id)?;
                        let value = match gap {
                            Some(reason) => Err(reason),
                            None => Ok(jobs),
                        };
                        entry.insert(value);
                    }
                    match &job_cache[&run.id] {
                        Ok(jobs) => {
                            let matching_jobs = jobs
                                .iter()
                                .filter(|job| {
                                    job.check_run_url == check.url && job.name == check.name
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            if matching_jobs.len() == 1 {
                                links.push((source_sha, check, run, matching_jobs[0].clone()));
                            } else {
                                run_lookup_failed = true;
                                add_workflow_gap(
                                    &mut gaps,
                                    &investigation.requirement,
                                    "producer_workflow_job_ambiguous",
                                    format!(
                                        "workflow run {} exposed {} jobs linked to check run {}",
                                        run.id,
                                        matching_jobs.len(),
                                        check.id
                                    ),
                                );
                            }
                        }
                        Err(reason) => {
                            run_lookup_failed = true;
                            add_workflow_gap(
                                &mut gaps,
                                &investigation.requirement,
                                "producer_workflow_jobs_incomplete",
                                reason,
                            );
                        }
                    }
                }
                Ok(runs) => {
                    run_lookup_failed = true;
                    add_workflow_gap(
                        &mut gaps,
                        &investigation.requirement,
                        "producer_workflow_run_ambiguous",
                        format!(
                            "check suite {check_suite_id} resolved to {} workflow runs",
                            runs.len()
                        ),
                    );
                }
                Err(reason) => {
                    run_lookup_failed = true;
                    add_workflow_gap(
                        &mut gaps,
                        &investigation.requirement,
                        "producer_workflow_runs_incomplete",
                        reason,
                    );
                }
            }
        }
        if run_lookup_failed || links.is_empty() {
            continue;
        }
        let workflow_ids = links
            .iter()
            .map(|(_, _, run, _)| run.workflow_id)
            .collect::<BTreeSet<_>>();
        if workflow_ids.len() != 1 {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "producer_workflow_ambiguous",
                "Matching check suites resolved to more than one workflow",
            );
            continue;
        }
        let (source_sha, check, run, job) = links[0].clone();
        let workflow_endpoint = format!(
            "repos/{repository_path}/actions/workflows/{}",
            run.workflow_id
        );
        let workflow_response = budget.get(api, &workflow_endpoint)?;
        if !successful(&workflow_response) {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_metadata_unavailable",
                format!(
                    "GitHub returned HTTP {} for workflow {}",
                    workflow_response.status, run.workflow_id
                ),
            );
            continue;
        }
        let workflow: ApiWorkflow = parse_json(&workflow_response.body, &workflow_endpoint)?;
        if workflow.id != run.workflow_id
            || links.iter().any(|(_, _, linked_run, _)| {
                linked_run.workflow_id != workflow.id
                    || !workflow_run_path_matches(&linked_run.path, &workflow.path)
            })
        {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_identity_mismatch",
                "Workflow run identity did not match canonical workflow metadata",
            );
            continue;
        }
        if !direct_workflow_path(&workflow.path) {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_path_unsupported",
                format!(
                    "Workflow {} is not a direct .github/workflows YAML file",
                    workflow.path
                ),
            );
            continue;
        }
        let producer = DoctorWorkflowProducer {
            source_sha,
            check_run_id: check.id,
            check_run_api_url: check.url,
            check_run_url: check.html_url,
            check_name: check.name,
            app_id: check
                .app
                .as_ref()
                .context("matching producer check is missing its App")?
                .id,
            app_slug: check
                .app
                .as_ref()
                .context("matching producer check is missing its App")?
                .slug
                .clone(),
            check_suite_id: check
                .check_suite
                .context("matching producer check is missing check_suite.id")?
                .id,
            workflow_run_id: run.id,
            workflow_run_attempt: job.run_attempt,
            workflow_run_url: run.html_url,
            workflow_run_path: run.path,
            workflow_job_id: job.id,
            workflow_job_url: job.html_url,
            workflow_job_name: job.name,
            workflow_job_check_run_url: job.check_run_url,
            workflow_id: workflow.id,
            workflow_path: workflow.path.clone(),
            workflow_url: workflow.html_url.clone(),
        };
        investigation.producer = Some(producer);

        let Some(state) = doctor_workflow_state(&workflow.state) else {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_state_unsupported",
                format!(
                    "Workflow {} reported unsupported state {}",
                    workflow.path, workflow.state
                ),
            );
            continue;
        };
        if state != WorkflowState::Active {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_disabled_outside_v3_scope",
                format!(
                    "Workflow {} is {}; v3 only classifies active workflow triggers",
                    workflow.path, workflow.state
                ),
            );
            continue;
        }
        let Some(inventory) = &workflow_inventory else {
            continue;
        };
        let Some(inventory_file) = inventory
            .files
            .iter()
            .find(|file| file.path == workflow.path)
        else {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_definition_absent_from_inventory",
                format!(
                    "Workflow {} is absent from the exact-SHA workflow inventory",
                    workflow.path
                ),
            );
            continue;
        };
        if inventory
            .files
            .iter()
            .flat_map(|file| &file.jobs)
            .any(|job| !job.name_static || job.reusable)
        {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_inventory_job_mapping_unresolved",
                "The exact-SHA inventory contains a dynamic job name or reusable workflow job",
            );
            continue;
        }
        let required_context = investigation.requirement.context.as_str();
        let matching_jobs = inventory
            .files
            .iter()
            .flat_map(|file| {
                file.jobs.iter().filter_map(move |job| {
                    (job.name == required_context).then_some((file.path.as_str(), job.id.as_str()))
                })
            })
            .collect::<Vec<_>>();
        if matching_jobs.len() != 1 || matching_jobs[0].0 != workflow.path {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_producer_not_unique",
                format!(
                    "The exact-SHA inventory resolved {} static jobs for the required context",
                    matching_jobs.len()
                ),
            );
            continue;
        }
        let definition = DoctorWorkflowDefinition {
            jobs: inventory_file.jobs.clone(),
            path: inventory_file.path.clone(),
            state,
            syntax: WorkflowSyntax::Valid,
            triggers: inventory_file.triggers.clone(),
        };
        let Some(head_ref) = pull.head.reference.clone() else {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "pull_request_head_ref_unavailable",
                "GitHub did not expose the pull-request head ref",
            );
            continue;
        };
        let Some(head_repository) = pull.head.repo.as_ref() else {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "pull_request_head_repository_unavailable",
                "GitHub did not expose the pull-request head repository",
            );
            continue;
        };

        let mut run_gaps = Vec::new();
        let mut observed_runs = Vec::new();
        for target_run in target_runs
            .iter()
            .filter(|target_run| target_run.workflow_id == workflow.id)
        {
            if !workflow_run_path_matches(&target_run.path, &workflow.path) {
                run_gaps.push("target_workflow_path_mismatch".to_owned());
                continue;
            }
            let mut normalized = target_run.clone();
            normalized.path = workflow.path.clone();
            match doctor_run_observation(&normalized) {
                Some(observation) => observed_runs.push(observation),
                None if matches!(target_run.event.as_str(), "merge_group" | "pull_request") => {
                    run_gaps.push("target_workflow_run_state_unknown".to_owned());
                }
                None => {}
            }
        }
        if definition.triggers.merge_group.is_none()
            && observed_runs
                .iter()
                .any(|run| run.event == WorkflowRunEvent::MergeGroup)
        {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                "workflow_trigger_evidence_conflict",
                "An exact-candidate merge_group run conflicts with the collected workflow definition",
            );
            continue;
        }
        for code in &run_gaps {
            add_workflow_gap(
                &mut gaps,
                &investigation.requirement,
                code,
                "An exact-candidate workflow run could not be normalized safely",
            );
        }
        if target_run_gap.is_some() {
            run_gaps.push("target_workflow_runs_incomplete".to_owned());
        }
        run_gaps.sort();
        run_gaps.dedup();
        if !run_gaps.is_empty() {
            continue;
        }
        investigation.input = Some(DoctorWorkflowTriggerInput {
            changed_files: WorkflowChangedFiles {
                complete: false,
                github_filter_file_limit_reached: false,
                paths: Vec::new(),
                total: 0,
            },
            collection_gaps: run_gaps,
            expected_app: WorkflowExpectedApp::GithubActions,
            historical_check_names: vec![investigation.requirement.context.clone()],
            last_activity: "not_applicable".to_owned(),
            provider_capability: WorkflowProviderCapability::NotApplicable,
            pull_request: crate::doctor_workflow::WorkflowPullRequest {
                base_ref: pull.base.reference.clone(),
                base_sha: pull.base.sha.to_ascii_lowercase(),
                head_ref,
                head_repository_is_fork: !head_repository
                    .full_name
                    .eq_ignore_ascii_case(request.repository),
                head_sha: pull.head.sha.to_ascii_lowercase(),
                mergeable_state: WorkflowMergeableState::Unknown,
                number: pull.number,
            },
            required_context: investigation.requirement.context.clone(),
            runs: observed_runs,
            target: WorkflowTarget {
                kind: WorkflowTargetKind::MergeGroup,
                sha: snapshot.signal_sha.clone(),
            },
            workflows: vec![definition],
        });
    }

    let final_pull_response = budget.get(api, &pull_endpoint)?;
    ensure!(
        successful(&final_pull_response),
        "GitHub returned HTTP {} while revalidating workflow producer candidates",
        final_pull_response.status
    );
    let final_pull: ApiDoctorPullRequest = parse_json(&final_pull_response.body, &pull_endpoint)?;
    ensure!(
        final_pull == pull,
        "pull request producer candidates changed during workflow collection; retry the doctor command"
    );
    gaps.sort_by(|left, right| {
        left.requirement
            .cmp(&right.requirement)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    gaps.dedup();
    let complete = gaps.is_empty() && investigations.iter().all(|item| item.input.is_some());
    Ok((
        DoctorWorkflowCollection {
            status: if complete {
                DoctorWorkflowCollectionStatus::Complete
            } else {
                DoctorWorkflowCollectionStatus::Partial
            },
            api_calls: u64::try_from(budget.requests - starting_requests)?,
            response_bytes: u64::try_from(budget.response_bytes - starting_response_bytes)?,
            inventory: workflow_inventory,
            probes: declared_probes,
            gaps,
        },
        investigations,
    ))
}

fn expected_workflow_probes(
    pull: &ApiDoctorPullRequest,
    signal_sha: &str,
) -> Vec<DoctorWorkflowProbe> {
    let mut probes = Vec::new();
    if let Some(test_merge_sha) = &pull.merge_commit_sha
        && !test_merge_sha.eq_ignore_ascii_case(signal_sha)
        && !test_merge_sha.eq_ignore_ascii_case(&pull.head.sha)
        && !test_merge_sha.eq_ignore_ascii_case(&pull.base.sha)
    {
        probes.push(DoctorWorkflowProbe {
            kind: DoctorWorkflowProbeKind::TestMerge,
            sha: test_merge_sha.to_ascii_lowercase(),
        });
    }
    if !pull.head.sha.eq_ignore_ascii_case(signal_sha) {
        probes.push(DoctorWorkflowProbe {
            kind: DoctorWorkflowProbeKind::PullRequestHead,
            sha: pull.head.sha.to_ascii_lowercase(),
        });
    }
    probes
}

fn doctor_workflow_semantics_equal(
    before_collection: &DoctorWorkflowCollection,
    before_investigations: &[DoctorWorkflowTriggerInvestigation],
    after_collection: &DoctorWorkflowCollection,
    after_investigations: &[DoctorWorkflowTriggerInvestigation],
) -> bool {
    before_collection.status == after_collection.status
        && before_collection.inventory == after_collection.inventory
        && before_collection.probes == after_collection.probes
        && before_collection.gaps == after_collection.gaps
        && before_investigations == after_investigations
}

pub fn collect_pull_request_doctor_snapshot_v3<A: GithubPullRequestDoctorApi>(
    request: PullRequestDoctorCollection<'_>,
    api: &mut A,
) -> Result<PullRequestDoctorSnapshotV3> {
    let mut budget = ApiBudget::new();
    let initial =
        collect_pull_request_doctor_snapshot_with_budget(request.clone(), api, &mut budget)?;
    let initial_report = evaluate_pull_request_doctor_v2(&initial)?;
    let requirements = if initial.target.evaluation.kind == DoctorEvaluationTargetKind::MergeGroup {
        initial_report
            .requirements
            .iter()
            .filter(|diagnosis| {
                diagnosis.status == DoctorRequirementStatus::Missing
                    && diagnosis.key.expected_app_id.is_some()
            })
            .map(|diagnosis| diagnosis.key.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if requirements.is_empty() {
        return Ok(v3_from_v2(
            initial,
            not_applicable_workflow_collection(),
            Vec::new(),
        ));
    }

    let (first_workflow_collection, first_investigations) = collect_doctor_workflow_investigations(
        &request,
        &initial,
        &requirements,
        api,
        &mut budget,
    )?;
    let middle =
        collect_pull_request_doctor_snapshot_with_budget(request.clone(), api, &mut budget)?;
    ensure!(
        doctor_snapshot_semantics_equal(&initial, &middle),
        "pull-request doctor evidence changed during workflow diagnosis; retry the doctor command"
    );
    let (mut workflow_collection, investigations) =
        collect_doctor_workflow_investigations(&request, &middle, &requirements, api, &mut budget)?;
    ensure!(
        doctor_workflow_semantics_equal(
            &first_workflow_collection,
            &first_investigations,
            &workflow_collection,
            &investigations,
        ),
        "workflow producer evidence changed during diagnosis; retry the doctor command"
    );
    workflow_collection.api_calls = first_workflow_collection
        .api_calls
        .checked_add(workflow_collection.api_calls)
        .context("workflow API call count overflow")?;
    workflow_collection.response_bytes = first_workflow_collection
        .response_bytes
        .checked_add(workflow_collection.response_bytes)
        .context("workflow response byte count overflow")?;
    let mut revalidated =
        collect_pull_request_doctor_snapshot_with_budget(request.clone(), api, &mut budget)?;
    ensure!(
        doctor_snapshot_semantics_equal(&initial, &revalidated)
            && doctor_snapshot_semantics_equal(&middle, &revalidated),
        "pull-request doctor evidence changed during final revalidation; retry the doctor command"
    );
    revalidated.collection.api_calls = u64::try_from(budget.requests)?;
    revalidated.collection.response_bytes = u64::try_from(budget.response_bytes)?;
    Ok(v3_from_v2(revalidated, workflow_collection, investigations))
}
