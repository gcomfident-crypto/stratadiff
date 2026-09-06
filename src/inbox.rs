use std::{
    collections::HashSet,
    env,
    ffi::OsStr,
    io::Write,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use stratadiff::github::{
    GithubReviewCheckpoint, MAX_GITHUB_REVIEWS, MAX_GITHUB_REVIEWS_BYTES,
    resolve_github_review_checkpoint,
};

use crate::{
    process::{SignalState, run_bounded_process},
    value_funnel,
};

const INBOX_SCHEMA: &str = "stratadiff-review-inbox-v2";
const MAX_CANDIDATES: usize = 100;
const MAX_API_CALLS: usize = 256;
const MAX_CAPTURED_REVIEW_NODES: usize = 100_000;
const MAX_TOTAL_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const API_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const REPOSITORY_QUERY: &str = r#"
query StrataDiffReviewInboxRepository(
  $owner: String!
  $name: String!
) {
  viewer { id login }
  repository(owner: $owner, name: $name) { id nameWithOwner url }
  rateLimit { cost remaining resetAt }
}
"#;

const SEARCH_QUERY: &str = r#"
query StrataDiffReviewInboxSearch(
  $searchQuery: String!
  $first: Int!
  $reviewer: String!
) {
  viewer { id login }
  search(query: $searchQuery, type: ISSUE, first: $first) {
    issueCount
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        id number state url isDraft updatedAt headRefOid
        repository { id nameWithOwner url }
        allReviews: reviews { totalCount }
        reviews(first: 100, author: $reviewer) {
          totalCount
          pageInfo { hasNextPage endCursor }
          nodes {
            id fullDatabaseId state submittedAt url authorAssociation
            author { __typename login ... on User { id } }
            commit { oid }
          }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"#;

const REVIEW_PAGE_QUERY: &str = r#"
query StrataDiffReviewInboxReviewPage(
  $owner: String!
  $name: String!
  $number: Int!
  $reviewer: String!
  $cursor: String!
) {
  viewer { id login }
  repository(owner: $owner, name: $name) {
    id nameWithOwner url
    pullRequest(number: $number) {
      id number state url isDraft updatedAt headRefOid
      allReviews: reviews { totalCount }
      reviews(first: 100, after: $cursor, author: $reviewer) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          id fullDatabaseId state submittedAt url authorAssociation
          author { __typename login ... on User { id } }
          commit { oid }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"#;

const REVALIDATE_QUERY: &str = r#"
query StrataDiffReviewInboxRevalidate(
  $owner: String!
  $name: String!
  $number: Int!
  $reviewer: String!
) {
  viewer { id login }
  repository(owner: $owner, name: $name) {
    id nameWithOwner url
    pullRequest(number: $number) {
      id number state url isDraft updatedAt headRefOid
      allReviews: reviews { totalCount }
      reviews(first: 100, author: $reviewer) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          id fullDatabaseId state submittedAt url authorAssociation
          author { __typename login ... on User { id } }
          commit { oid }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"#;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum InboxFormat {
    Markdown,
    Json,
}

#[derive(Debug, Args)]
pub(crate) struct InboxArgs {
    /// Exact reviewer login; defaults to the authenticated `gh` user.
    #[arg(long)]
    reviewer: Option<String>,
    /// Limit the global queue to one repository in [HOST/]OWNER/REPO form.
    #[arg(short = 'R', long = "repo", value_name = "REPO")]
    repository: Option<String>,
    /// GitHub hostname used when -R does not include one; defaults to github.com.
    #[arg(long)]
    hostname: Option<String>,
    /// Maximum number of recently updated reviewed pull requests to inspect.
    #[arg(long, default_value_t = 100, value_parser = parse_limit)]
    limit: usize,
    /// Output format.
    #[arg(long, value_enum, default_value_t = InboxFormat::Markdown)]
    format: InboxFormat,
    /// Write the report to a file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Opt in to a private, local-only, integrity-chained value-funnel log.
    #[arg(long, value_name = "PATH")]
    value_log: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
    id: u64,
    node_id: String,
    #[serde(rename = "type")]
    account_type: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    errors: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchData {
    viewer: GraphqlViewer,
    search: SearchConnection,
    rate_limit: RateLimit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchConnection {
    issue_count: usize,
    page_info: PageInfo,
    nodes: Vec<Option<PullRequestRecord>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevalidateData {
    viewer: GraphqlViewer,
    repository: Option<RepositoryWithPullRequest>,
    rate_limit: RateLimit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryData {
    viewer: GraphqlViewer,
    repository: Option<RepositoryRecord>,
    rate_limit: RateLimit,
}

#[derive(Debug, Deserialize)]
struct GraphqlViewer {
    login: String,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryWithPullRequest {
    id: String,
    name_with_owner: String,
    url: String,
    pull_request: Option<PullRequestBody>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PullRequestRecord {
    id: String,
    number: u64,
    state: String,
    url: String,
    is_draft: bool,
    updated_at: String,
    head_ref_oid: Option<String>,
    repository: RepositoryRecord,
    all_reviews: Count,
    reviews: ReviewConnection,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PullRequestBody {
    id: String,
    number: u64,
    state: String,
    url: String,
    is_draft: bool,
    updated_at: String,
    head_ref_oid: Option<String>,
    all_reviews: Count,
    reviews: ReviewConnection,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RepositoryRecord {
    id: String,
    name_with_owner: String,
    url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ReviewConnection {
    total_count: usize,
    page_info: PageInfo,
    nodes: Vec<ReviewRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ReviewRecord {
    id: String,
    full_database_id: Option<DatabaseId>,
    state: String,
    submitted_at: Option<String>,
    url: String,
    author_association: String,
    author: Option<ReviewAuthor>,
    commit: Option<Commit>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum DatabaseId {
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ReviewAuthor {
    #[serde(rename = "__typename")]
    account_type: String,
    login: String,
    id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Commit {
    oid: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimit {
    cost: usize,
    remaining: usize,
    reset_at: String,
}

#[derive(Debug, Serialize)]
struct ReviewInbox {
    schema: &'static str,
    tool_version: &'static str,
    observed_at_unix_seconds: u64,
    scope: InboxScope,
    collection: Collection,
    privacy: Privacy,
    summary: Summary,
    actionable: Vec<ActionableItem>,
    unobservable: Vec<UnobservableItem>,
}

#[derive(Debug, Serialize)]
struct InboxScope {
    provider_url: String,
    repository: Option<String>,
    authenticated_actor: ActorIdentity,
    reviewer: ReviewerIdentity,
}

#[derive(Clone, Debug, Serialize)]
struct ActorIdentity {
    login: String,
    database_id: u64,
    node_id: String,
}

#[derive(Debug, Serialize)]
struct ReviewerIdentity {
    login: String,
    database_id: u64,
    node_id: String,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct Collection {
    status: &'static str,
    temporal_consistency: &'static str,
    search_candidates: usize,
    inspected_candidates: usize,
    truncated: bool,
    revalidated_review_prs: usize,
    api_calls: usize,
    captured_review_nodes: usize,
    response_bytes: usize,
    minimum_rate_limit_remaining: Option<usize>,
    last_rate_limit_reset_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct Privacy {
    source_collected: bool,
    pr_text_collected: bool,
    review_text_collected: bool,
    commit_messages_collected: bool,
    authenticated_actor_identity_persisted: bool,
    reviewer_identity_persisted: bool,
}

#[derive(Debug, Serialize)]
struct Summary {
    status: &'static str,
    completed_review_prs: usize,
    resume_available_prs: usize,
    up_to_date_prs: usize,
    no_completed_review_prs: usize,
    unobservable_review_prs: usize,
}

#[derive(Debug, Serialize)]
struct ActionableItem {
    event_id: String,
    #[serde(skip)]
    value_transition_id: String,
    repository: String,
    number: u64,
    url: String,
    is_draft: bool,
    updated_at: String,
    checkpoint: GithubReviewCheckpoint,
    head_oid: String,
    total_review_count: usize,
    resume_argv: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UnobservableItem {
    repository: String,
    number: u64,
    url: String,
    updated_at: String,
    reason: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CandidateSnapshot {
    pull_request: PullRequestRecord,
    reviews: Vec<ReviewRecord>,
}

struct GhClient<'a> {
    hostname: String,
    signals: &'a SignalState,
    started: Instant,
    calls: usize,
    captured_review_nodes: usize,
    response_bytes: usize,
    minimum_rate_limit_remaining: Option<usize>,
    last_rate_limit_reset_at: Option<String>,
}

impl<'a> GhClient<'a> {
    fn new(hostname: String, signals: &'a SignalState) -> Self {
        Self {
            hostname,
            signals,
            started: Instant::now(),
            calls: 0,
            captured_review_nodes: 0,
            response_bytes: 0,
            minimum_rate_limit_remaining: None,
            last_rate_limit_reset_at: None,
        }
    }

    fn rest<T: DeserializeOwned>(&mut self, endpoint: &str, label: &str) -> Result<T> {
        let mut command = gh_command();
        command.args(["api", "--hostname", &self.hostname, endpoint]);
        let bytes = self.call(&mut command, label)?;
        serde_json::from_slice(&bytes).with_context(|| format!("failed to decode {label}"))
    }

    fn graphql<T: DeserializeOwned>(
        &mut self,
        query: &str,
        raw_fields: &[(&str, String)],
        typed_fields: &[(&str, String)],
        label: &str,
    ) -> Result<T> {
        let mut command = gh_command();
        command
            .args(["api", "--hostname", &self.hostname, "graphql", "-f"])
            .arg(format!("query={query}"));
        for (name, value) in raw_fields {
            command.args(["-f", &format!("{name}={value}")]);
        }
        for (name, value) in typed_fields {
            command.args(["-F", &format!("{name}={value}")]);
        }
        let bytes = self.call(&mut command, label)?;
        let envelope: GraphqlEnvelope<T> =
            serde_json::from_slice(&bytes).with_context(|| format!("failed to decode {label}"))?;
        ensure!(
            envelope.errors.is_none(),
            "{label} returned GraphQL errors: {}",
            envelope.errors.unwrap_or(Value::Null)
        );
        envelope
            .data
            .with_context(|| format!("{label} omitted GraphQL data"))
    }

    fn call(&mut self, command: &mut Command, label: &str) -> Result<Vec<u8>> {
        self.check_budget()?;
        ensure!(
            self.calls < MAX_API_CALLS,
            "review inbox exceeds its {MAX_API_CALLS}-call API budget"
        );
        let remaining = TOTAL_TIMEOUT
            .checked_sub(self.started.elapsed())
            .context("review inbox exceeded its five-minute wall-time budget")?;
        let remaining_response_bytes = MAX_TOTAL_RESPONSE_BYTES
            .checked_sub(self.response_bytes)
            .context("review inbox response byte count exceeds its budget")?;
        ensure!(
            remaining_response_bytes > 0,
            "review inbox exceeds its {MAX_TOTAL_RESPONSE_BYTES}-byte response budget"
        );
        let output = run_bounded_process(
            command,
            MAX_GITHUB_REVIEWS_BYTES.min(remaining_response_bytes),
            MAX_DIAGNOSTIC_BYTES,
            remaining.min(API_CALL_TIMEOUT),
            label,
            Some(self.signals),
        )?;
        ensure!(
            output.status.success(),
            "{label} failed with {}: {}",
            output.status,
            diagnostic(&output.stderr)
        );
        self.calls += 1;
        self.response_bytes = self
            .response_bytes
            .checked_add(output.stdout.len())
            .context("review inbox response byte count overflow")?;
        ensure!(
            self.response_bytes <= MAX_TOTAL_RESPONSE_BYTES,
            "review inbox exceeds its {MAX_TOTAL_RESPONSE_BYTES}-byte response budget"
        );
        self.check_budget()?;
        Ok(output.stdout)
    }

    fn observe_rate_limit(&mut self, rate_limit: &RateLimit) -> Result<()> {
        ensure!(
            valid_timestamp(&rate_limit.reset_at),
            "GitHub rate-limit reset timestamp is invalid"
        );
        ensure!(
            rate_limit.cost <= 5_000,
            "GitHub reported an invalid query cost"
        );
        self.minimum_rate_limit_remaining = Some(
            self.minimum_rate_limit_remaining
                .map_or(rate_limit.remaining, |value| {
                    value.min(rate_limit.remaining)
                }),
        );
        self.last_rate_limit_reset_at = Some(rate_limit.reset_at.clone());
        Ok(())
    }

    fn observe_review_nodes(&mut self, count: usize) -> Result<()> {
        self.captured_review_nodes = self
            .captured_review_nodes
            .checked_add(count)
            .context("review inbox captured review node count overflow")?;
        ensure!(
            self.captured_review_nodes <= MAX_CAPTURED_REVIEW_NODES,
            "review inbox exceeds its {MAX_CAPTURED_REVIEW_NODES}-node review budget"
        );
        Ok(())
    }

    fn check_budget(&self) -> Result<()> {
        ensure!(
            self.started.elapsed() <= TOTAL_TIMEOUT,
            "review inbox exceeded its five-minute wall-time budget"
        );
        self.signals.check()
    }
}

pub(crate) fn run(args: InboxArgs) -> Result<()> {
    let signals = SignalState::register()?;
    let value_log = args
        .value_log
        .as_deref()
        .map(value_funnel::canonical_log_path)
        .transpose()?;
    if let (Some(log), Some(output)) = (&value_log, &args.output) {
        value_funnel::ensure_distinct_output(log, output)?;
    }
    let (hostname, repository) =
        parse_repository(args.repository.as_deref(), args.hostname.as_deref())?;
    let mut client = GhClient::new(hostname.clone(), &signals);
    let authenticated_actor = resolve_authenticated_actor(&mut client)?;
    let reviewer = resolve_reviewer(&mut client, args.reviewer.as_deref(), &authenticated_actor)?;
    let inbox = collect_inbox(
        &mut client,
        authenticated_actor,
        reviewer,
        repository,
        args.limit,
        value_log.as_deref(),
    )?;
    let bytes = match args.format {
        InboxFormat::Markdown => render_markdown(&inbox).into_bytes(),
        InboxFormat::Json => serde_json::to_vec(&inbox)?,
    };
    let value_scan_id = if let Some(path) = &value_log {
        let transition_ids = inbox
            .actionable
            .iter()
            .map(|item| item.value_transition_id.clone())
            .collect::<Vec<_>>();
        Some(value_funnel::record_inbox_discovery(
            path,
            inbox.collection.status == "complete",
            inbox.collection.inspected_candidates,
            inbox.summary.completed_review_prs,
            &transition_ids,
        )?)
    } else {
        None
    };
    if let Some(path) = args.output {
        write_private(&path, &bytes)?;
        eprintln!("wrote review inbox to {}", crate::display_path(&path));
    } else {
        let mut stdout = std::io::stdout().lock();
        match args.format {
            InboxFormat::Markdown => stdout.write_all(&bytes)?,
            InboxFormat::Json => {
                stdout.write_all(crate::escape_terminal_unsafe_json(&bytes).as_bytes())?
            }
        }
        if !bytes.ends_with(b"\n") {
            stdout.write_all(b"\n")?;
        }
        stdout.flush()?;
    }
    if let (Some(path), Some(scan_id)) = (&value_log, &value_scan_id) {
        value_funnel::record_inbox_delivery(path, scan_id).with_context(|| {
            "review Inbox output was delivered, but its local value-log delivery confirmation failed"
        })?;
    }
    Ok(())
}

fn collect_inbox(
    client: &mut GhClient<'_>,
    authenticated_actor: ActorIdentity,
    reviewer: ReviewerIdentity,
    repository: Option<String>,
    limit: usize,
    value_log: Option<&std::path::Path>,
) -> Result<ReviewInbox> {
    let scoped_repository =
        resolve_repository_scope(client, &authenticated_actor, repository.as_deref())?;
    let repository = scoped_repository
        .as_ref()
        .map(|repository| repository.name_with_owner.clone());
    let mut search_query = format!(
        "is:pr is:open reviewed-by:{} sort:updated-desc",
        reviewer.login
    );
    if let Some(repository) = &repository {
        search_query.push_str(" repo:");
        search_query.push_str(repository);
    }
    let data: SearchData = client.graphql(
        SEARCH_QUERY,
        &[
            ("searchQuery", search_query),
            ("reviewer", reviewer.login.clone()),
        ],
        &[("first", limit.to_string())],
        "GitHub review inbox search",
    )?;
    ensure_authenticated_actor(&data.viewer, &authenticated_actor)?;
    client.observe_rate_limit(&data.rate_limit)?;
    ensure!(
        data.search.nodes.len() <= limit,
        "GitHub review inbox search returned more than {limit} candidates"
    );
    ensure!(
        data.search.issue_count >= data.search.nodes.len(),
        "GitHub review inbox search count is smaller than its result page"
    );
    ensure!(
        data.search.page_info.has_next_page == (data.search.issue_count > data.search.nodes.len()),
        "GitHub review inbox search pagination metadata is inconsistent"
    );
    let mut initial = Vec::with_capacity(data.search.nodes.len());
    let mut pull_request_ids = HashSet::new();
    for (index, node) in data.search.nodes.into_iter().enumerate() {
        let pull_request = node.with_context(|| {
            format!(
                "review inbox search result {} is not a pull request",
                index + 1
            )
        })?;
        validate_pull_request(&pull_request, &client.hostname, repository.as_deref())?;
        client.observe_review_nodes(pull_request.reviews.nodes.len())?;
        if let Some(expected_repository) = &scoped_repository {
            ensure_repository_matches(
                &pull_request.repository.id,
                &pull_request.repository.name_with_owner,
                &pull_request.repository.url,
                expected_repository,
            )?;
        }
        ensure!(
            pull_request_ids.insert(pull_request.id.clone()),
            "GitHub review inbox search returned a duplicate pull request"
        );
        let snapshot = complete_snapshot(client, pull_request, &authenticated_actor, &reviewer)?;
        initial.push(snapshot);
    }

    let mut completed_review_prs = 0;
    let mut up_to_date_prs = 0;
    let mut no_completed_review_prs = 0;
    let mut revalidated_review_prs = 0;
    let mut actionable = Vec::new();
    let mut unobservable = Vec::new();
    for snapshot in initial {
        let resolution = resolve_checkpoint(&snapshot.reviews, &reviewer)?;
        let Some(initial_checkpoint) = resolution.checkpoint else {
            no_completed_review_prs += 1;
            continue;
        };
        validate_checkpoint_output(&initial_checkpoint, &snapshot.pull_request.url)?;
        completed_review_prs += 1;
        let observed = revalidate_snapshot(client, &snapshot, &authenticated_actor, &reviewer)?;
        ensure!(
            observed == snapshot,
            "{}#{} changed while its Review Resume action was revalidated; rerun the command",
            snapshot.pull_request.repository.name_with_owner,
            snapshot.pull_request.number
        );
        revalidated_review_prs += 1;
        let pull_request = snapshot.pull_request;
        let Some(head_oid) = pull_request.head_ref_oid.clone() else {
            unobservable.push(UnobservableItem {
                repository: pull_request.repository.name_with_owner,
                number: pull_request.number,
                url: pull_request.url,
                updated_at: pull_request.updated_at,
                reason: "head_oid_unavailable",
            });
            continue;
        };
        if initial_checkpoint.commit_id == head_oid {
            up_to_date_prs += 1;
            continue;
        }
        if pull_request.all_reviews.total_count > MAX_GITHUB_REVIEWS {
            unobservable.push(UnobservableItem {
                repository: pull_request.repository.name_with_owner,
                number: pull_request.number,
                url: pull_request.url,
                updated_at: pull_request.updated_at,
                reason: "resume_review_limit_exceeded",
            });
            continue;
        }
        let checkpoint_review_node_id =
            checkpoint_review_node_id(&snapshot.reviews, initial_checkpoint.review_id)?;
        let event_id = inbox_event_id(
            &client.hostname,
            &pull_request.repository.id,
            &pull_request.id,
            &reviewer.node_id,
            checkpoint_review_node_id,
            &initial_checkpoint.commit_id,
            &head_oid,
        );
        let value_transition_id = value_funnel::transition_id(&value_funnel::TransitionIdentity {
            provider_hostname: &client.hostname,
            repository: &pull_request.repository.name_with_owner,
            pull_request_number: pull_request.number,
            reviewer: &reviewer.login,
            review_id: initial_checkpoint.review_id,
            review_state: &initial_checkpoint.review_state,
            checkpoint: &initial_checkpoint.commit_id,
            head: &head_oid,
        });
        let resume_argv = resume_argv(
            &client.hostname,
            &pull_request.repository.name_with_owner,
            &pull_request.url,
            &reviewer.login,
            value_log,
            &value_transition_id,
        );
        actionable.push(ActionableItem {
            event_id,
            value_transition_id,
            repository: pull_request.repository.name_with_owner,
            number: pull_request.number,
            url: pull_request.url.clone(),
            is_draft: pull_request.is_draft,
            updated_at: pull_request.updated_at,
            checkpoint: initial_checkpoint,
            head_oid,
            total_review_count: pull_request.all_reviews.total_count,
            resume_argv,
        });
    }
    actionable.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.repository.cmp(&right.repository))
            .then_with(|| left.number.cmp(&right.number))
    });
    unobservable.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.repository.cmp(&right.repository))
            .then_with(|| left.number.cmp(&right.number))
    });
    let truncated = data.search.issue_count > pull_request_ids.len();
    let status = if truncated {
        "partial"
    } else if !actionable.is_empty() {
        "actionable"
    } else if !unobservable.is_empty() {
        "insufficient_evidence"
    } else if completed_review_prs > 0 {
        "up_to_date"
    } else {
        "no_eligible_reviews"
    };
    let observed_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    Ok(ReviewInbox {
        schema: INBOX_SCHEMA,
        tool_version: env!("CARGO_PKG_VERSION"),
        observed_at_unix_seconds,
        scope: InboxScope {
            provider_url: format!("https://{}", client.hostname),
            repository,
            authenticated_actor,
            reviewer,
        },
        collection: Collection {
            status: if truncated { "partial" } else { "complete" },
            temporal_consistency: "eligible_candidates_revalidated_non_atomic",
            search_candidates: data.search.issue_count,
            inspected_candidates: pull_request_ids.len(),
            truncated,
            revalidated_review_prs,
            api_calls: client.calls,
            captured_review_nodes: client.captured_review_nodes,
            response_bytes: client.response_bytes,
            minimum_rate_limit_remaining: client.minimum_rate_limit_remaining,
            last_rate_limit_reset_at: client.last_rate_limit_reset_at.clone(),
        },
        privacy: Privacy {
            source_collected: false,
            pr_text_collected: false,
            review_text_collected: false,
            commit_messages_collected: false,
            authenticated_actor_identity_persisted: true,
            reviewer_identity_persisted: true,
        },
        summary: Summary {
            status,
            completed_review_prs,
            resume_available_prs: actionable.len(),
            up_to_date_prs,
            no_completed_review_prs,
            unobservable_review_prs: unobservable.len(),
        },
        actionable,
        unobservable,
    })
}

fn resolve_repository_scope(
    client: &mut GhClient<'_>,
    authenticated_actor: &ActorIdentity,
    requested: Option<&str>,
) -> Result<Option<RepositoryRecord>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let (owner, name) = split_repository(requested)?;
    let data: RepositoryData = client.graphql(
        REPOSITORY_QUERY,
        &[("owner", owner.to_owned()), ("name", name.to_owned())],
        &[],
        "GitHub review inbox repository lookup",
    )?;
    ensure_authenticated_actor(&data.viewer, authenticated_actor)?;
    client.observe_rate_limit(&data.rate_limit)?;
    let repository = data.repository.with_context(|| {
        format!("GitHub repository {requested} does not exist or is not accessible")
    })?;
    validate_repository(&repository.name_with_owner)?;
    ensure!(
        repository.name_with_owner.eq_ignore_ascii_case(requested),
        "GitHub repository lookup returned a different repository"
    );
    ensure!(
        valid_node_id(&repository.id),
        "GitHub repository has an invalid node ID"
    );
    ensure!(
        repository.url == format!("https://{}/{}", client.hostname, repository.name_with_owner),
        "GitHub repository URL is not canonical"
    );
    Ok(Some(repository))
}

fn complete_snapshot(
    client: &mut GhClient<'_>,
    pull_request: PullRequestRecord,
    authenticated_actor: &ActorIdentity,
    reviewer: &ReviewerIdentity,
) -> Result<CandidateSnapshot> {
    let mut reviews = pull_request.reviews.nodes.clone();
    append_review_pages(
        client,
        &pull_request.repository,
        &PullRequestBody {
            id: pull_request.id.clone(),
            number: pull_request.number,
            state: pull_request.state.clone(),
            url: pull_request.url.clone(),
            is_draft: pull_request.is_draft,
            updated_at: pull_request.updated_at.clone(),
            head_ref_oid: pull_request.head_ref_oid.clone(),
            all_reviews: pull_request.all_reviews.clone(),
            reviews: pull_request.reviews.clone(),
        },
        authenticated_actor,
        &reviewer.login,
        &mut reviews,
    )?;
    validate_complete_reviews(&reviews, pull_request.reviews.total_count, reviewer)?;
    Ok(CandidateSnapshot {
        pull_request,
        reviews,
    })
}

fn revalidate_snapshot(
    client: &mut GhClient<'_>,
    expected: &CandidateSnapshot,
    authenticated_actor: &ActorIdentity,
    reviewer: &ReviewerIdentity,
) -> Result<CandidateSnapshot> {
    let repository = &expected.pull_request.repository;
    let (owner, name) = split_repository(&repository.name_with_owner)?;
    let data: RevalidateData = client.graphql(
        REVALIDATE_QUERY,
        &[
            ("owner", owner.to_owned()),
            ("name", name.to_owned()),
            ("reviewer", reviewer.login.clone()),
        ],
        &[("number", expected.pull_request.number.to_string())],
        "GitHub review inbox revalidation",
    )?;
    ensure_authenticated_actor(&data.viewer, authenticated_actor)?;
    client.observe_rate_limit(&data.rate_limit)?;
    let observed_repository = data
        .repository
        .context("GitHub review inbox revalidation omitted the repository")?;
    ensure_repository_matches(
        &observed_repository.id,
        &observed_repository.name_with_owner,
        &observed_repository.url,
        repository,
    )?;
    let body = observed_repository
        .pull_request
        .context("reviewed pull request is no longer available")?;
    let pull_request = PullRequestRecord {
        id: body.id.clone(),
        number: body.number,
        state: body.state.clone(),
        url: body.url.clone(),
        is_draft: body.is_draft,
        updated_at: body.updated_at.clone(),
        head_ref_oid: body.head_ref_oid.clone(),
        repository: repository.clone(),
        all_reviews: body.all_reviews.clone(),
        reviews: body.reviews.clone(),
    };
    validate_pull_request(&pull_request, &client.hostname, None)?;
    client.observe_review_nodes(body.reviews.nodes.len())?;
    let mut reviews = body.reviews.nodes.clone();
    append_review_pages(
        client,
        repository,
        &body,
        authenticated_actor,
        &reviewer.login,
        &mut reviews,
    )?;
    validate_complete_reviews(&reviews, body.reviews.total_count, reviewer)?;
    Ok(CandidateSnapshot {
        pull_request,
        reviews,
    })
}

fn append_review_pages(
    client: &mut GhClient<'_>,
    repository: &RepositoryRecord,
    expected_pull_request: &PullRequestBody,
    authenticated_actor: &ActorIdentity,
    reviewer: &str,
    reviews: &mut Vec<ReviewRecord>,
) -> Result<()> {
    let (owner, name) = split_repository(&repository.name_with_owner)?;
    let mut page_info = expected_pull_request.reviews.page_info.clone();
    let total_count = expected_pull_request.reviews.total_count;
    while page_info.has_next_page {
        let cursor = page_info
            .end_cursor
            .clone()
            .context("GitHub review pagination omitted its end cursor")?;
        let data: RevalidateData = client.graphql(
            REVIEW_PAGE_QUERY,
            &[
                ("owner", owner.to_owned()),
                ("name", name.to_owned()),
                ("reviewer", reviewer.to_owned()),
                ("cursor", cursor),
            ],
            &[("number", expected_pull_request.number.to_string())],
            "GitHub review inbox review pagination",
        )?;
        ensure_authenticated_actor(&data.viewer, authenticated_actor)?;
        client.observe_rate_limit(&data.rate_limit)?;
        let observed_repository = data
            .repository
            .context("GitHub review pagination omitted the repository")?;
        ensure_repository_matches(
            &observed_repository.id,
            &observed_repository.name_with_owner,
            &observed_repository.url,
            repository,
        )?;
        let observed = observed_repository
            .pull_request
            .context("reviewed pull request disappeared during review pagination")?;
        ensure_pull_request_body_matches(&observed, expected_pull_request)?;
        ensure!(
            observed.reviews.total_count == total_count,
            "review count changed during review pagination"
        );
        ensure!(
            !observed.reviews.nodes.is_empty(),
            "GitHub review pagination returned an empty page"
        );
        client.observe_review_nodes(observed.reviews.nodes.len())?;
        reviews.extend(observed.reviews.nodes.iter().cloned());
        ensure!(
            reviews.len() <= MAX_GITHUB_REVIEWS,
            "reviewer review count exceeds {MAX_GITHUB_REVIEWS}"
        );
        ensure!(
            observed.reviews.page_info.end_cursor != page_info.end_cursor,
            "GitHub review pagination cursor did not advance"
        );
        page_info = observed.reviews.page_info;
    }
    Ok(())
}

fn resolve_checkpoint(
    reviews: &[ReviewRecord],
    reviewer: &ReviewerIdentity,
) -> Result<stratadiff::github::GithubCheckpointResolution> {
    let mut values = Vec::with_capacity(reviews.len());
    for review in reviews {
        if !matches!(review.state.as_str(), "APPROVED" | "CHANGES_REQUESTED") {
            continue;
        }
        let database_id_value = review
            .full_database_id
            .as_ref()
            .context("GitHub completed review is missing its database ID")?;
        let database_id = database_id(database_id_value)?;
        let user = review.author.as_ref().map(|author| {
            json!({
                "login": author.login,
                "type": author.account_type,
            })
        });
        values.push(json!({
            "id": database_id,
            "user": user,
            "state": review.state,
            "html_url": review.url,
            "commit_id": review.commit.as_ref().map_or("", |commit| commit.oid.as_str()),
            "submitted_at": review.submitted_at,
            "author_association": review.author_association,
        }));
    }
    resolve_github_review_checkpoint(&serde_json::to_vec(&values)?, &reviewer.login)
}

fn validate_complete_reviews(
    reviews: &[ReviewRecord],
    expected: usize,
    reviewer: &ReviewerIdentity,
) -> Result<()> {
    ensure!(
        reviews.len() == expected,
        "GitHub reviewer history is incomplete: expected {expected}, captured {}",
        reviews.len()
    );
    let mut node_ids = HashSet::new();
    let mut database_ids = HashSet::new();
    for review in reviews {
        ensure!(
            valid_node_id(&review.id),
            "GitHub review has an invalid node ID"
        );
        ensure!(
            node_ids.insert(review.id.clone()),
            "GitHub reviewer history contains a duplicate node"
        );
        if let Some(database_id_value) = &review.full_database_id {
            let database_id = database_id(database_id_value)?;
            ensure!(
                database_ids.insert(database_id),
                "GitHub reviewer history contains a duplicate database ID"
            );
        } else {
            ensure!(
                !matches!(review.state.as_str(), "APPROVED" | "CHANGES_REQUESTED"),
                "GitHub completed review is missing its database ID"
            );
        }
        let author = review
            .author
            .as_ref()
            .context("reviewer-filtered GitHub review has no author")?;
        ensure!(
            author.account_type == "User"
                && author.login.eq_ignore_ascii_case(&reviewer.login)
                && author.id.as_deref() == Some(&reviewer.node_id),
            "reviewer-filtered GitHub review is not bound to the requested immutable reviewer identity"
        );
    }
    Ok(())
}

fn validate_pull_request(
    pull_request: &PullRequestRecord,
    hostname: &str,
    expected_repository: Option<&str>,
) -> Result<()> {
    ensure!(
        pull_request.state == "OPEN",
        "review inbox returned a non-open pull request"
    );
    ensure!(
        valid_node_id(&pull_request.id),
        "pull request has an invalid node ID"
    );
    validate_repository(&pull_request.repository.name_with_owner)?;
    if let Some(expected) = expected_repository {
        ensure!(
            pull_request
                .repository
                .name_with_owner
                .eq_ignore_ascii_case(expected),
            "review inbox result escaped the requested repository"
        );
    }
    ensure!(
        valid_node_id(&pull_request.repository.id),
        "repository has an invalid node ID"
    );
    let expected_repository_url = format!(
        "https://{hostname}/{}",
        pull_request.repository.name_with_owner
    );
    ensure!(
        pull_request.repository.url == expected_repository_url,
        "GitHub repository URL is not canonical"
    );
    ensure!(
        pull_request.url == format!("{expected_repository_url}/pull/{}", pull_request.number),
        "GitHub pull request URL is not canonical"
    );
    ensure!(
        valid_timestamp(&pull_request.updated_at),
        "pull request has an invalid updated timestamp"
    );
    if let Some(head_oid) = &pull_request.head_ref_oid {
        ensure!(
            is_sha1(head_oid),
            "pull request head is not a full lowercase SHA-1"
        );
    }
    ensure!(
        pull_request.reviews.nodes.len() <= 100,
        "GitHub returned more than 100 reviews in one page"
    );
    ensure!(
        pull_request.reviews.total_count <= MAX_GITHUB_REVIEWS,
        "reviewer review count exceeds {MAX_GITHUB_REVIEWS}"
    );
    Ok(())
}

fn ensure_repository_matches(
    id: &str,
    name_with_owner: &str,
    url: &str,
    expected: &RepositoryRecord,
) -> Result<()> {
    ensure!(
        id == expected.id && name_with_owner == expected.name_with_owner && url == expected.url,
        "repository identity changed while collecting the review inbox"
    );
    Ok(())
}

fn ensure_pull_request_body_matches(
    observed: &PullRequestBody,
    expected: &PullRequestBody,
) -> Result<()> {
    ensure!(
        observed.id == expected.id
            && observed.number == expected.number
            && observed.state == expected.state
            && observed.url == expected.url
            && observed.is_draft == expected.is_draft
            && observed.updated_at == expected.updated_at
            && observed.head_ref_oid == expected.head_ref_oid
            && observed.all_reviews == expected.all_reviews,
        "pull request changed during review pagination"
    );
    Ok(())
}

fn resolve_authenticated_actor(client: &mut GhClient<'_>) -> Result<ActorIdentity> {
    let user: GithubUser = client.rest("user", "GitHub authenticated actor identity")?;
    validate_github_user(&user, "authenticated actor")?;
    Ok(ActorIdentity {
        login: user.login,
        database_id: user.id,
        node_id: user.node_id,
    })
}

fn resolve_reviewer(
    client: &mut GhClient<'_>,
    requested: Option<&str>,
    authenticated_actor: &ActorIdentity,
) -> Result<ReviewerIdentity> {
    if let Some(login) = requested {
        validate_login(login)?;
    }
    let user = match requested {
        None => GithubUser {
            login: authenticated_actor.login.clone(),
            id: authenticated_actor.database_id,
            node_id: authenticated_actor.node_id.clone(),
            account_type: "User".to_owned(),
        },
        Some(login) if login.eq_ignore_ascii_case(&authenticated_actor.login) => GithubUser {
            login: authenticated_actor.login.clone(),
            id: authenticated_actor.database_id,
            node_id: authenticated_actor.node_id.clone(),
            account_type: "User".to_owned(),
        },
        Some(login) => {
            let user: GithubUser =
                client.rest(&format!("users/{login}"), "GitHub reviewer identity")?;
            ensure!(
                user.login.eq_ignore_ascii_case(login),
                "GitHub reviewer identity differs from --reviewer"
            );
            user
        }
    };
    validate_github_user(&user, "reviewer")?;
    Ok(ReviewerIdentity {
        login: user.login,
        database_id: user.id,
        node_id: user.node_id,
        source: if requested.is_some() {
            "explicit_reviewer"
        } else {
            "authenticated_viewer"
        },
    })
}

fn validate_github_user(user: &GithubUser, label: &str) -> Result<()> {
    ensure!(
        user.account_type == "User",
        "GitHub {label} identity is not a User"
    );
    ensure!(user.id > 0, "GitHub {label} database ID is not positive");
    validate_login(&user.login).with_context(|| format!("GitHub {label} login is invalid"))?;
    ensure!(
        valid_node_id(&user.node_id),
        "GitHub {label} has an invalid node ID"
    );
    Ok(())
}

fn ensure_authenticated_actor(viewer: &GraphqlViewer, expected: &ActorIdentity) -> Result<()> {
    validate_login(&viewer.login).context("GitHub GraphQL viewer login is invalid")?;
    ensure!(
        valid_node_id(&viewer.id),
        "GitHub GraphQL viewer has an invalid node ID"
    );
    ensure!(
        viewer.login.eq_ignore_ascii_case(&expected.login) && viewer.id == expected.node_id,
        "GitHub authenticated actor changed while collecting the review inbox"
    );
    Ok(())
}

fn parse_repository(
    requested: Option<&str>,
    requested_hostname: Option<&str>,
) -> Result<(String, Option<String>)> {
    if let Some(hostname) = requested_hostname {
        validate_host(hostname)?;
    }
    let Some(requested) = requested else {
        return Ok((
            requested_hostname
                .unwrap_or("github.com")
                .to_ascii_lowercase(),
            None,
        ));
    };
    let parts = requested.split('/').collect::<Vec<_>>();
    let (hostname, repository) = match parts.as_slice() {
        [owner, name] => (
            requested_hostname.unwrap_or("github.com"),
            format!("{owner}/{name}"),
        ),
        [hostname, owner, name] => (*hostname, format!("{owner}/{name}")),
        _ => bail!("GitHub repository must be [HOST/]OWNER/REPO"),
    };
    validate_host(hostname)?;
    if let Some(requested_hostname) = requested_hostname {
        ensure!(
            hostname.eq_ignore_ascii_case(requested_hostname),
            "repository host {hostname} does not match --hostname {requested_hostname}"
        );
    }
    validate_repository(&repository)?;
    Ok((hostname.to_ascii_lowercase(), Some(repository)))
}

fn split_repository(repository: &str) -> Result<(&str, &str)> {
    validate_repository(repository)?;
    repository
        .split_once('/')
        .context("GitHub repository must be OWNER/REPO")
}

fn validate_repository(repository: &str) -> Result<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    ensure!(
        !owner.is_empty()
            && !name.is_empty()
            && parts.next().is_none()
            && owner.chars().all(valid_repository_character)
            && name.chars().all(valid_repository_character),
        "GitHub repository must be OWNER/REPO"
    );
    Ok(())
}

fn validate_host(host: &str) -> Result<()> {
    ensure!(
        host.as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
        "GitHub hostname is invalid"
    );
    Ok(())
}

fn validate_login(login: &str) -> Result<()> {
    let bytes = login.as_bytes();
    ensure!(
        (1..=255).contains(&bytes.len())
            && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && bytes
                .iter()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-') }),
        "reviewer login is invalid"
    );
    Ok(())
}

fn parse_limit(value: &str) -> std::result::Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| "limit must be an integer from 1 through 100".to_owned())?;
    if (1..=MAX_CANDIDATES).contains(&limit) {
        Ok(limit)
    } else {
        Err("limit must be an integer from 1 through 100".to_owned())
    }
}

fn valid_repository_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
}

fn database_id(value: &DatabaseId) -> Result<u64> {
    match value {
        DatabaseId::Number(value) => {
            ensure!(*value > 0, "GitHub review database ID is not positive");
            Ok(*value)
        }
        DatabaseId::String(value) => {
            ensure!(
                !value.is_empty()
                    && !value.starts_with('0')
                    && value.bytes().all(|byte| byte.is_ascii_digit()),
                "GitHub review database ID is not canonical decimal"
            );
            let value = value
                .parse::<u64>()
                .context("GitHub review database ID exceeds u64")?;
            ensure!(value > 0, "GitHub review database ID is not positive");
            Ok(value)
        }
    }
}

fn inbox_event_id(
    provider_hostname: &str,
    repository_node_id: &str,
    pull_request_node_id: &str,
    reviewer_node_id: &str,
    review_node_id: &str,
    checkpoint: &str,
    head: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [
        "stratadiff-review-inbox-event-v2",
        provider_hostname,
        repository_node_id,
        pull_request_node_id,
        reviewer_node_id,
        review_node_id,
        checkpoint,
        head,
    ] {
        hasher.update(field.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn checkpoint_review_node_id(reviews: &[ReviewRecord], review_id: u64) -> Result<&str> {
    let mut node_id = None;
    for review in reviews {
        let Some(value) = &review.full_database_id else {
            continue;
        };
        if database_id(value)? == review_id {
            ensure!(
                node_id.is_none(),
                "selected GitHub checkpoint database ID is not unique"
            );
            node_id = Some(review.id.as_str());
        }
    }
    node_id.context("selected GitHub checkpoint is absent from the reviewer history")
}

fn validate_checkpoint_output(
    checkpoint: &GithubReviewCheckpoint,
    pull_request_url: &str,
) -> Result<()> {
    ensure!(checkpoint.review_id > 0, "GitHub review ID is not positive");
    validate_login(&checkpoint.reviewer_login)?;
    ensure!(
        matches!(
            checkpoint.review_state.as_str(),
            "approved" | "changes_requested"
        ),
        "GitHub review checkpoint has an unsupported state"
    );
    ensure!(
        is_sha1(&checkpoint.commit_id),
        "GitHub review checkpoint commit is not a full lowercase SHA-1"
    );
    ensure!(
        valid_timestamp(&checkpoint.submitted_at),
        "GitHub review checkpoint timestamp is invalid"
    );
    ensure!(
        checkpoint.html_url
            == format!(
                "{pull_request_url}#pullrequestreview-{}",
                checkpoint.review_id
            ),
        "GitHub review checkpoint URL is not canonical"
    );
    ensure!(
        !checkpoint.author_association.is_empty()
            && checkpoint.author_association.len() <= 64
            && checkpoint
                .author_association
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
        "GitHub review checkpoint author association is invalid"
    );
    Ok(())
}

fn resume_argv(
    hostname: &str,
    repository: &str,
    url: &str,
    reviewer: &str,
    value_log: Option<&std::path::Path>,
    transition_id: &str,
) -> Vec<String> {
    let mut arguments = vec![
        "stratadiff".to_owned(),
        "resume".to_owned(),
        url.to_owned(),
        "--reviewer".to_owned(),
        reviewer.to_owned(),
    ];
    if hostname != "github.com" {
        arguments.push("-R".to_owned());
        arguments.push(format!("{hostname}/{repository}"));
    }
    if let Some(value_log) = value_log {
        arguments.push("--value-log".to_owned());
        arguments.push(
            value_log
                .to_str()
                .expect("the canonical value log path was validated as UTF-8")
                .to_owned(),
        );
        arguments.push("--transition-id".to_owned());
        arguments.push(transition_id.to_owned());
    }
    arguments
}

fn render_markdown(inbox: &ReviewInbox) -> String {
    let mut output = String::new();
    output.push_str("# StrataDiff Review Inbox\n\n");
    match inbox.summary.status {
        "partial" => output.push_str(&format!(
            "**Partial queue:** found {} Resume action{} while inspecting {} of {} matching open pull requests for `@{}`. This is not a clean global result.\n\n",
            inbox.summary.resume_available_prs,
            plural(inbox.summary.resume_available_prs),
            inbox.collection.inspected_candidates,
            inbox.collection.search_candidates,
            inbox.scope.reviewer.login,
        )),
        "actionable" => output.push_str(&format!(
            "**{} review{} need{} Resume** for `@{}`. {} completed review{} {} already up to date.\n\n",
            inbox.summary.resume_available_prs,
            plural(inbox.summary.resume_available_prs),
            if inbox.summary.resume_available_prs == 1 {
                "s"
            } else {
                ""
            },
            inbox.scope.reviewer.login,
            inbox.summary.up_to_date_prs,
            plural(inbox.summary.up_to_date_prs),
            if inbox.summary.up_to_date_prs == 1 {
                "is"
            } else {
                "are"
            },
        )),
        "insufficient_evidence" => output.push_str(&format!(
            "**No safe Resume action was emitted** for `@{}`. {} reviewed pull request{} could not be compared completely.\n\n",
            inbox.scope.reviewer.login,
            inbox.summary.unobservable_review_prs,
            plural(inbox.summary.unobservable_review_prs),
        )),
        "up_to_date" => output.push_str(&format!(
            "**All {} completed review{} are up to date** for `@{}`.\n\n",
            inbox.summary.up_to_date_prs,
            plural(inbox.summary.up_to_date_prs),
            inbox.scope.reviewer.login,
        )),
        "no_eligible_reviews" => output.push_str(&format!(
            "**No completed review checkpoint was found** for `@{}` in this scan.\n\n",
            inbox.scope.reviewer.login,
        )),
        _ => unreachable!("Review Inbox status is constructed from a closed set"),
    }
    if inbox.collection.truncated {
        output.push_str(&format!(
            "> Only the {} most recently updated of {} matching open pull requests were inspected. Increase `--limit` or use `-R OWNER/REPO`; unseen candidates may also need Resume.\n\n",
            inbox.collection.inspected_candidates, inbox.collection.search_candidates
        ));
    }
    if !inbox.actionable.is_empty() {
        output.push_str("| Pull request | Updated | Checkpoint → head | Resume |\n");
        output.push_str("|---|---|---|---|\n");
        for item in &inbox.actionable {
            let resume_command = crate::escape_terminal_unsafe_text(&shell_join(&item.resume_argv));
            output.push_str(&format!(
                "| [`{}#{}`]({}) | `{}` | `{}…` → `{}…` | `{}` |\n",
                item.repository,
                item.number,
                item.url,
                item.updated_at,
                &item.checkpoint.commit_id[..8],
                &item.head_oid[..8],
                resume_command,
            ));
        }
    }
    if !inbox.unobservable.is_empty() {
        output.push_str(&format!(
            "\n{} reviewed pull request{} could not be compared safely and {} not actionable.\n",
            inbox.unobservable.len(),
            plural(inbox.unobservable.len()),
            if inbox.unobservable.len() == 1 {
                "is"
            } else {
                "are"
            },
        ));
    }
    output
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn shell_join(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if !argument.is_empty()
                && argument
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
            {
                argument.clone()
            } else {
                format!("'{}'", argument.replace('\'', "'\"'\"'"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    ensure!(parent.is_dir(), "output parent directory does not exist");
    let mut temporary = tempfile::Builder::new()
        .prefix(".stratadiff-inbox-")
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary output beside {}",
                path.display()
            )
        })?;
    temporary
        .write_all(bytes)
        .with_context(|| format!("failed to write temporary output for {}", path.display()))?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .with_context(|| format!("failed to open output directory {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync output directory {}", parent.display()))?;
    Ok(())
}

fn gh_command() -> Command {
    let mut command = Command::new("gh");
    for (name, _) in env::vars_os() {
        if is_git_environment_name(&name) {
            command.env_remove(name);
        }
    }
    command
        .env_remove("GH_REPO")
        .env_remove("GH_DEBUG")
        .env_remove("DEBUG")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env_remove("GH_FORCE_TTY")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1");
    command
}

fn is_git_environment_name(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"GIT_"))
}

fn diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().next().unwrap_or("no diagnostic output");
    line.chars().take(512).collect()
}

fn valid_node_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    let valid_fraction = match bytes.len() {
        20 => true,
        22..=30 => bytes[19] == b'.' && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit),
        _ => false,
    };
    if !valid_fraction
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
        || !bytes[..19]
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
    {
        return false;
    }

    let year = u16::from(bytes[0] - b'0') * 1_000
        + u16::from(bytes[1] - b'0') * 100
        + u16::from(bytes[2] - b'0') * 10
        + u16::from(bytes[3] - b'0');
    let month = (bytes[5] - b'0') * 10 + bytes[6] - b'0';
    let day = (bytes[8] - b'0') * 10 + bytes[9] - b'0';
    let hour = (bytes[11] - b'0') * 10 + bytes[12] - b'0';
    let minute = (bytes[14] - b'0') * 10 + bytes[15] - b'0';
    let second = (bytes[17] - b'0') * 10 + bytes[18] - b'0';
    let leap_year = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let maximum_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };

    day > 0
        && day <= maximum_day
        && hour <= 23
        && minute <= 59
        && (second <= 59 || (second == 60 && hour == 23 && minute == 59))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_identity_changes_only_with_the_review_event_tuple() {
        let first = inbox_event_id(
            "github.com",
            "R_widget",
            "PR_17",
            "U_reviewer",
            "PRR_1701",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        assert_eq!(first.len(), 64);
        assert_eq!(
            first,
            inbox_event_id(
                "github.com",
                "R_widget",
                "PR_17",
                "U_reviewer",
                "PRR_1701",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
        );
        assert_ne!(
            first,
            inbox_event_id(
                "github.com",
                "R_widget",
                "PR_17",
                "U_reviewer",
                "PRR_1701",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cccccccccccccccccccccccccccccccccccccccc",
            )
        );
        assert_ne!(
            first,
            inbox_event_id(
                "ghe.example",
                "R_widget",
                "PR_17",
                "U_reviewer",
                "PRR_1701",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
        );
        assert_eq!(
            first,
            inbox_event_id(
                "github.com",
                "R_widget",
                "PR_17",
                "U_reviewer",
                "PRR_1701",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
        );
    }

    #[test]
    fn repository_selector_rejects_host_confusion() {
        assert_eq!(
            parse_repository(Some("acme/widget"), None).unwrap(),
            ("github.com".to_owned(), Some("acme/widget".to_owned()))
        );
        assert_eq!(
            parse_repository(Some("ghe.example/acme/widget"), None).unwrap(),
            ("ghe.example".to_owned(), Some("acme/widget".to_owned()))
        );
        assert_eq!(
            parse_repository(Some("acme/widget"), Some("ghe.example")).unwrap(),
            ("ghe.example".to_owned(), Some("acme/widget".to_owned()))
        );
        assert!(
            parse_repository(Some("evil.example/acme/widget"), Some("github.com"))
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }

    #[test]
    fn enterprise_resume_command_binds_the_repository_host() {
        assert_eq!(
            resume_argv(
                "ghe.example",
                "acme/widget",
                "https://ghe.example/acme/widget/pull/17",
                "reviewer",
                None,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            [
                "stratadiff",
                "resume",
                "https://ghe.example/acme/widget/pull/17",
                "--reviewer",
                "reviewer",
                "-R",
                "ghe.example/acme/widget",
            ]
        );
    }

    #[test]
    fn reviewer_login_cannot_inject_search_qualifiers() {
        validate_login("reviewer-1").unwrap();
        assert!(validate_login("reviewer repo:private/repo").is_err());
        assert!(validate_login("reviewer:admin").is_err());
    }

    #[test]
    fn markdown_resume_command_quotes_local_paths() {
        assert_eq!(
            shell_join(&[
                "stratadiff".to_owned(),
                "--value-log".to_owned(),
                "/tmp/review pilot/value's.jsonl".to_owned(),
            ]),
            "stratadiff --value-log '/tmp/review pilot/value'\"'\"'s.jsonl'"
        );
    }

    #[test]
    fn timestamps_match_the_public_schema_contract() {
        for timestamp in [
            "0000-01-01T00:00:00Z",
            "2024-02-29T23:59:59.1Z",
            "2026-09-06T01:02:03.123456789Z",
            "2026-12-31T23:59:60Z",
        ] {
            assert!(valid_timestamp(timestamp), "rejected {timestamp}");
        }

        for timestamp in [
            "2026-09-06T01:02:03ZsuffixZ",
            "2026-09-06T01:02:03+00:00",
            "2026-09-06T01:02:03.Z",
            "2026-09-06T01:02:03.1234567890Z",
            "2025-02-29T01:02:03Z",
            "2026-00-06T01:02:03Z",
            "2026-09-31T01:02:03Z",
            "2026-09-06T24:02:03Z",
            "2026-09-06T01:60:03Z",
            "2026-09-06T01:02:60Z",
            "2026-09-06t01:02:03z",
        ] {
            assert!(!valid_timestamp(timestamp), "accepted {timestamp}");
        }
    }
}
