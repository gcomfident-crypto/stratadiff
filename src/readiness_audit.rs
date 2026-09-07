use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_yaml_ng::Value as YamlValue;

use crate::doctor::{
    DoctorCheckRun, DoctorCollection, DoctorCollectionGap, DoctorCollectionStatus,
    DoctorCollectionSurface, DoctorCommitStatus, DoctorPolicyKind, DoctorPolicyRef,
    DoctorRequirement, DoctorTarget, PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA,
    PullRequestDoctorSnapshot,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubReadinessApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub link_header: Option<String>,
}

pub trait GithubReadinessApi {
    fn get(&mut self, endpoint: &str) -> Result<GithubReadinessApiResponse>;
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
    sha: String,
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

fn collect_doctor_effective_requirements<A: GithubReadinessApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    repository_path: &str,
    repository_url: &str,
    branch: &str,
    gaps: &mut Vec<DoctorCollectionGap>,
    requirements: &mut Vec<DoctorRequirement>,
) -> Result<bool> {
    let mut unsupported_check_target = false;
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
            if matches!(rule.kind.as_str(), "merge_queue" | "workflows") {
                unsupported_check_target = true;
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
    Ok(unsupported_check_target)
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

pub fn collect_pull_request_doctor_snapshot<A: GithubReadinessApi>(
    request: PullRequestDoctorCollection<'_>,
    api: &mut A,
) -> Result<PullRequestDoctorSnapshot> {
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
    let mut budget = ApiBudget::new();
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
    let mut requirements = Vec::new();
    let unsupported_check_target = collect_doctor_effective_requirements(
        api,
        &mut budget,
        &repository_path,
        &repository.html_url,
        &pull.base.reference,
        &mut gaps,
        &mut requirements,
    )?;
    if unsupported_check_target {
        add_doctor_gap(
            &mut gaps,
            DoctorCollectionSurface::Target,
            "effective rules require a merge-queue or workflow target that this head-only Doctor does not evaluate"
                .to_owned(),
        );
    }
    collect_doctor_classic_requirements(
        api,
        &mut budget,
        &repository_path,
        &repository.html_url,
        &pull.base.reference,
        &mut gaps,
        &mut requirements,
    )?;
    sort_doctor_requirements(&mut requirements);

    let check_runs = collect_doctor_check_runs(
        api,
        &mut budget,
        &repository_path,
        &pull.head.sha,
        &mut gaps,
    )?;
    let statuses = collect_doctor_statuses(
        api,
        &mut budget,
        &repository_path,
        &pull.head.sha,
        &mut gaps,
    )?;

    if let Some(merge_commit_sha) = &pull.merge_commit_sha {
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
                &mut budget,
                &repository_path,
                merge_commit_sha,
                &mut target_probe_gaps,
            )?;
            let merge_statuses = collect_doctor_statuses(
                api,
                &mut budget,
                &repository_path,
                merge_commit_sha,
                &mut target_probe_gaps,
            )?;
            if target_probe_gaps.is_empty() {
                if !merge_check_runs.is_empty() || !merge_statuses.is_empty() {
                    add_doctor_gap(
                        &mut gaps,
                        DoctorCollectionSurface::Target,
                        format!(
                            "test-merge commit {merge_commit_sha} has status signals, so GitHub is not evaluating the PR head diagnosed by this version"
                        ),
                    );
                }
            } else {
                for gap in target_probe_gaps {
                    add_doctor_gap(
                        &mut gaps,
                        DoctorCollectionSurface::Target,
                        format!(
                            "could not determine whether GitHub is evaluating test-merge commit {merge_commit_sha}: {}",
                            gap.reason
                        ),
                    );
                }
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

    gaps.sort_by(|left, right| {
        doctor_surface_rank(&left.surface)
            .cmp(&doctor_surface_rank(&right.surface))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    gaps.dedup_by(|left, right| left.surface == right.surface && left.reason == right.reason);
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
    Ok(PullRequestDoctorSnapshot {
        schema: PULL_REQUEST_DOCTOR_SNAPSHOT_SCHEMA.to_owned(),
        captured_at: request.captured_at.to_owned(),
        provider_url: request.provider_url.to_owned(),
        repository: repository.full_name,
        target: DoctorTarget {
            number: pull.number,
            url: pull.html_url,
            base_ref: pull.base.reference,
            base_sha: pull.base.sha.to_ascii_lowercase(),
            head_sha: pull.head.sha.to_ascii_lowercase(),
        },
        collection,
        requirements,
        check_runs,
        statuses,
    })
}
