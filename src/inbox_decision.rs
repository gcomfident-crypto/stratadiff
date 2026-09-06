use std::{cmp::Ordering, collections::HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_CANDIDATES: u64 = 100;
const MAX_REVIEWS: u64 = 10_000;
const TRANSITION_DOMAIN: &str = "stratadiff-review-inbox-global-transition-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InboxObservation {
    pub provider: ProviderObservation,
    pub scope: ScopeObservation,
    pub search: SearchObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderObservation {
    pub host: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScopeObservation {
    pub authenticated_actor: ActorObservation,
    pub requested_repository: Option<RepositoryObservation>,
    pub reviewer: ReviewerObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActorObservation {
    pub login: String,
    pub node_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewerObservation {
    pub actor_type: String,
    pub login: String,
    pub node_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepositoryObservation {
    pub name_with_owner: String,
    pub node_id: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchObservation {
    pub candidates: Vec<CandidateObservation>,
    pub has_next_page: bool,
    pub issue_count: u64,
    pub limit: u64,
    pub outcome: String,
    pub viewer: ActorObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CandidateObservation {
    pub pull_request: PullRequestObservation,
    pub repository: RepositoryObservation,
    pub revalidation: RevalidationObservation,
    pub review_history: ReviewHistoryObservation,
    pub review_request_active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PullRequestObservation {
    pub current_base_oid: Option<String>,
    pub head_oid: Option<String>,
    pub node_id: String,
    pub number: u64,
    pub state: String,
    pub total_review_count: u64,
    pub updated_at: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RevalidationObservation {
    pub outcome: String,
    pub pull_request_node_id: String,
    pub repository: RepositoryObservation,
    pub snapshot_matches: bool,
    pub viewer: ActorObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewHistoryObservation {
    pub cursor_advanced: bool,
    pub nodes: Vec<ReviewObservation>,
    pub pages_observed: u64,
    pub reported_count: u64,
    pub terminal_page_observed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewObservation {
    pub author: ReviewerObservation,
    pub checkpoint_base_oid: Option<String>,
    pub commit_oid: Option<String>,
    pub database_id: Option<u64>,
    pub node_id: String,
    pub state: String,
    pub submitted_at: Option<String>,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxDecisionCounts {
    pub actionable: u64,
    pub no_eligible_reviews: u64,
    pub unobservable: u64,
    pub up_to_date: u64,
}

impl InboxDecisionCounts {
    fn empty() -> Self {
        Self {
            actionable: 0,
            no_eligible_reviews: 0,
            unobservable: 0,
            up_to_date: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxDecision {
    pub checkpoint_review_node_id: Option<String>,
    pub counts: InboxDecisionCounts,
    pub error: Option<String>,
    pub reason: Option<String>,
    pub result: String,
    pub status: Option<String>,
    pub trigger: Option<String>,
    #[serde(skip)]
    transition_key: Option<String>,
}

impl InboxDecision {
    pub fn transition_key(&self) -> Option<&str> {
        self.transition_key.as_deref()
    }

    fn error(code: &'static str) -> Self {
        Self {
            checkpoint_review_node_id: None,
            counts: InboxDecisionCounts::empty(),
            error: Some(code.to_owned()),
            reason: None,
            result: "error".to_owned(),
            status: None,
            trigger: None,
            transition_key: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxCandidateDecision {
    pub status: InboxCandidateStatus,
    pub checkpoint_review_node_id: Option<String>,
    pub reason: Option<String>,
    transition_key: Option<String>,
    pub trigger: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InboxCandidateStatus {
    Actionable,
    NoEligibleReviews,
    Unobservable,
    UpToDate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxEvaluation {
    pub decision: InboxDecision,
    pub candidates: Vec<InboxCandidateDecision>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecisionError(&'static str);

type DecisionResult<T> = Result<T, DecisionError>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Timestamp {
    components: [u32; 6],
    fraction: String,
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.components
            .cmp(&other.components)
            .then_with(|| compare_fraction(&self.fraction, &other.fraction))
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn decode_inbox_observation(value: Value) -> Result<InboxObservation, serde_json::Error> {
    serde_json::from_value(value)
}

pub fn decide_inbox_observation_json(value: &Value) -> InboxDecision {
    match decode_inbox_observation(value.clone()) {
        Ok(observation) => decide_inbox_observation(&observation),
        Err(_) => InboxDecision::error("observation_shape_invalid"),
    }
}

pub fn decide_inbox_observation(observation: &InboxObservation) -> InboxDecision {
    evaluate_inbox_observation(observation).decision
}

pub fn evaluate_inbox_observation(observation: &InboxObservation) -> InboxEvaluation {
    evaluate_with_policy(observation, false)
}

pub fn evaluate_inbox_observation_for_resume(observation: &InboxObservation) -> InboxEvaluation {
    evaluate_with_policy(observation, true)
}

fn evaluate_with_policy(
    observation: &InboxObservation,
    require_current_base_for_action: bool,
) -> InboxEvaluation {
    match evaluate(observation, require_current_base_for_action) {
        Ok(evaluation) => evaluation,
        Err(error) => InboxEvaluation {
            decision: InboxDecision::error(error.0),
            candidates: Vec::new(),
        },
    }
}

fn evaluate(
    observation: &InboxObservation,
    require_current_base_for_action: bool,
) -> DecisionResult<InboxEvaluation> {
    validate_host(&observation.provider.host, "provider_identity_invalid")?;
    ensure(
        observation.provider.host == observation.provider.host.to_ascii_lowercase(),
        "provider_identity_invalid",
    )?;
    validate_actor(
        &observation.scope.authenticated_actor,
        "authenticated_actor_invalid",
    )?;
    validate_reviewer(&observation.scope.reviewer, "reviewer_identity_invalid")?;
    if let Some(repository) = &observation.scope.requested_repository {
        validate_repository(
            repository,
            &observation.provider.host,
            "provider_url_mismatch",
        )?;
    }

    ensure(
        matches!(
            observation.search.outcome.as_str(),
            "ok" | "forbidden" | "repository_not_found"
        ),
        "search_outcome_invalid",
    )?;
    match observation.search.outcome.as_str() {
        "forbidden" => return Err(DecisionError("search_forbidden")),
        "repository_not_found" => return Err(DecisionError("repository_not_found")),
        "ok" => {}
        _ => unreachable!("search outcome was validated"),
    }
    validate_actor(&observation.search.viewer, "authenticated_actor_invalid")?;
    ensure(
        same_actor(
            &observation.search.viewer,
            &observation.scope.authenticated_actor,
        ),
        "authenticated_actor_changed",
    )?;
    ensure(
        (1..=MAX_CANDIDATES).contains(&observation.search.limit),
        "search_limit_invalid",
    )?;
    ensure(
        observation.search.candidates.len() as u64 <= observation.search.limit,
        "search_limit_exceeded",
    )?;
    ensure(
        observation.search.issue_count >= observation.search.candidates.len() as u64,
        "search_count_invalid",
    )?;
    ensure(
        observation.search.has_next_page
            == (observation.search.issue_count > observation.search.candidates.len() as u64),
        "search_pagination_inconsistent",
    )?;

    let mut candidate_results = Vec::with_capacity(observation.search.candidates.len());
    let mut pull_request_ids = HashSet::new();
    for candidate in &observation.search.candidates {
        let result = decide_candidate(
            candidate,
            &observation.provider.host,
            &observation.scope.authenticated_actor,
            &observation.scope.reviewer,
            observation.scope.requested_repository.as_ref(),
            require_current_base_for_action,
        )?;
        ensure(
            pull_request_ids.insert(candidate.pull_request.node_id.as_str()),
            "duplicate_pull_request_node",
        )?;
        candidate_results.push(result);
    }

    let mut counts = InboxDecisionCounts::empty();
    for decision in &candidate_results {
        match decision.status {
            InboxCandidateStatus::Actionable => counts.actionable += 1,
            InboxCandidateStatus::NoEligibleReviews => counts.no_eligible_reviews += 1,
            InboxCandidateStatus::Unobservable => counts.unobservable += 1,
            InboxCandidateStatus::UpToDate => counts.up_to_date += 1,
        }
    }
    let status = if observation.search.has_next_page {
        "partial"
    } else if counts.actionable > 0 {
        "actionable"
    } else if counts.unobservable > 0 {
        "insufficient_evidence"
    } else if counts.up_to_date > 0 {
        "up_to_date"
    } else {
        "no_eligible_reviews"
    };

    let decision = InboxDecision {
        checkpoint_review_node_id: exactly_one(
            candidate_results
                .iter()
                .filter_map(|decision| decision.checkpoint_review_node_id.as_deref()),
        )
        .map(str::to_owned),
        counts,
        error: None,
        reason: exactly_one(
            candidate_results
                .iter()
                .filter_map(|decision| decision.reason.as_deref()),
        )
        .map(str::to_owned),
        result: "success".to_owned(),
        status: Some(status.to_owned()),
        trigger: exactly_one(
            candidate_results
                .iter()
                .filter_map(|decision| decision.trigger.as_deref()),
        )
        .map(str::to_owned),
        transition_key: exactly_one(
            candidate_results
                .iter()
                .filter_map(|decision| decision.transition_key.as_deref()),
        )
        .map(str::to_owned),
    };
    Ok(InboxEvaluation {
        decision,
        candidates: candidate_results,
    })
}

fn decide_candidate(
    candidate: &CandidateObservation,
    host: &str,
    authenticated_actor: &ActorObservation,
    reviewer: &ReviewerObservation,
    requested_repository: Option<&RepositoryObservation>,
    require_current_base_for_action: bool,
) -> DecisionResult<InboxCandidateDecision> {
    validate_repository(&candidate.repository, host, "provider_url_mismatch")?;
    if let Some(requested_repository) = requested_repository {
        ensure(
            candidate.repository == *requested_repository,
            "repository_identity_changed",
        )?;
    }

    let pull_request = &candidate.pull_request;
    validate_node(&pull_request.node_id, "pull_request_identity_invalid")?;
    ensure(pull_request.number > 0, "pull_request_identity_invalid")?;
    ensure(pull_request.state == "OPEN", "pull_request_not_open")?;
    ensure(
        pull_request.url == format!("{}/pull/{}", candidate.repository.url, pull_request.number),
        "provider_url_mismatch",
    )?;
    parse_timestamp(&pull_request.updated_at, "pull_request_timestamp_invalid")?;
    validate_optional_oid(&pull_request.head_oid, "head_oid_invalid")?;
    validate_optional_oid(&pull_request.current_base_oid, "current_base_oid_invalid")?;

    let revalidation = &candidate.revalidation;
    ensure(
        matches!(
            revalidation.outcome.as_str(),
            "matched" | "forbidden" | "not_found"
        ),
        "revalidation_outcome_invalid",
    )?;
    match revalidation.outcome.as_str() {
        "forbidden" => return Err(DecisionError("candidate_forbidden")),
        "not_found" => return Err(DecisionError("candidate_not_found")),
        "matched" => {}
        _ => unreachable!("revalidation outcome was validated"),
    }
    validate_actor(&revalidation.viewer, "authenticated_actor_invalid")?;
    ensure(
        same_actor(&revalidation.viewer, authenticated_actor),
        "authenticated_actor_changed",
    )?;
    validate_repository(&revalidation.repository, host, "provider_url_mismatch")?;
    ensure(
        revalidation.repository == candidate.repository,
        "repository_identity_changed",
    )?;
    ensure(
        revalidation.pull_request_node_id == pull_request.node_id,
        "pull_request_identity_changed",
    )?;
    ensure(
        revalidation.snapshot_matches,
        "candidate_changed_during_revalidation",
    )?;

    let history = &candidate.review_history;
    ensure(history.pages_observed >= 1, "review_pagination_invalid")?;
    ensure(
        history.terminal_page_observed,
        "incomplete_review_pagination",
    )?;
    if history.pages_observed > 1 {
        ensure(history.cursor_advanced, "review_pagination_cursor_stalled")?;
    }
    ensure(
        history.reported_count == history.nodes.len() as u64,
        "review_history_count_mismatch",
    )?;
    ensure(
        history.reported_count <= MAX_REVIEWS,
        "reviewer_history_limit_exceeded",
    )?;
    ensure(
        pull_request.total_review_count >= history.reported_count,
        "review_count_invalid",
    )?;

    let mut review_node_ids = HashSet::new();
    let mut database_ids = HashSet::new();
    let mut checkpoint: Option<(Timestamp, u64, &ReviewObservation)> = None;
    for review in &history.nodes {
        validate_node(&review.node_id, "review_node_invalid")?;
        ensure(
            review_node_ids.insert(review.node_id.as_str()),
            "duplicate_review_node",
        )?;
        validate_reviewer(&review.author, "reviewer_identity_mismatch")?;
        ensure(
            same_reviewer(&review.author, reviewer),
            "reviewer_identity_mismatch",
        )?;
        ensure(
            matches!(
                review.state.as_str(),
                "APPROVED" | "CHANGES_REQUESTED" | "COMMENTED" | "DISMISSED" | "PENDING"
            ),
            "review_state_invalid",
        )?;
        if let Some(database_id) = review.database_id {
            ensure(database_id > 0, "review_database_id_invalid")?;
            ensure(
                database_ids.insert(database_id),
                "duplicate_review_database_id",
            )?;
            ensure(
                review.url == format!("{}#pullrequestreview-{database_id}", pull_request.url),
                "review_url_invalid",
            )?;
        }
        validate_optional_oid(&review.commit_oid, "review_commit_invalid")?;
        validate_optional_oid(&review.checkpoint_base_oid, "checkpoint_base_oid_invalid")?;
        let submitted_at = review
            .submitted_at
            .as_deref()
            .map(|value| parse_timestamp(value, "review_timestamp_invalid"))
            .transpose()?;

        if matches!(review.state.as_str(), "APPROVED" | "CHANGES_REQUESTED") {
            let database_id = review
                .database_id
                .ok_or(DecisionError("completed_review_database_id_missing"))?;
            let submitted_at =
                submitted_at.ok_or(DecisionError("completed_review_timestamp_missing"))?;
            ensure(review.commit_oid.is_some(), "review_commit_invalid")?;
            let replace = checkpoint
                .as_ref()
                .is_none_or(|(timestamp, id, _)| (&submitted_at, database_id) > (timestamp, *id));
            if replace {
                checkpoint = Some((submitted_at, database_id, review));
            }
        }
    }

    let Some((_, _, checkpoint)) = checkpoint else {
        return Ok(InboxCandidateDecision {
            status: InboxCandidateStatus::NoEligibleReviews,
            checkpoint_review_node_id: None,
            reason: None,
            transition_key: None,
            trigger: None,
        });
    };
    if pull_request.head_oid.is_none() {
        return Ok(unobservable(checkpoint, "head_oid_unavailable"));
    }
    if pull_request.total_review_count > MAX_REVIEWS {
        return Ok(unobservable(checkpoint, "resume_review_limit_exceeded"));
    }
    if require_current_base_for_action && pull_request.current_base_oid.is_none() {
        return Ok(unobservable(checkpoint, "current_base_oid_unavailable"));
    }

    let checkpoint_commit = checkpoint
        .commit_oid
        .as_deref()
        .expect("formal checkpoint commit was validated");
    let head_oid = pull_request
        .head_oid
        .as_deref()
        .expect("current head was checked above");
    let head_changed = checkpoint_commit != head_oid;
    if !head_changed && checkpoint.checkpoint_base_oid.is_none() {
        return Ok(unobservable(checkpoint, "checkpoint_base_oid_unavailable"));
    }
    if !head_changed && pull_request.current_base_oid.is_none() {
        return Ok(unobservable(checkpoint, "current_base_oid_unavailable"));
    }
    let base_changed = checkpoint.checkpoint_base_oid.is_some()
        && pull_request.current_base_oid.is_some()
        && checkpoint.checkpoint_base_oid != pull_request.current_base_oid;
    let mut triggers = Vec::with_capacity(3);
    if head_changed {
        triggers.push("head_changed");
    }
    if base_changed {
        triggers.push("base_drift");
    }
    if candidate.review_request_active {
        triggers.push("review_re_requested");
    }
    if triggers.is_empty() {
        return Ok(InboxCandidateDecision {
            status: InboxCandidateStatus::UpToDate,
            checkpoint_review_node_id: Some(checkpoint.node_id.clone()),
            reason: None,
            transition_key: None,
            trigger: None,
        });
    }

    Ok(InboxCandidateDecision {
        status: InboxCandidateStatus::Actionable,
        checkpoint_review_node_id: Some(checkpoint.node_id.clone()),
        reason: None,
        transition_key: Some(transition_key(
            host,
            &candidate.repository,
            pull_request,
            reviewer,
            checkpoint,
            candidate.review_request_active,
        )),
        trigger: Some(triggers.join("+")),
    })
}

fn unobservable(checkpoint: &ReviewObservation, reason: &'static str) -> InboxCandidateDecision {
    InboxCandidateDecision {
        status: InboxCandidateStatus::Unobservable,
        checkpoint_review_node_id: Some(checkpoint.node_id.clone()),
        reason: Some(reason.to_owned()),
        transition_key: None,
        trigger: None,
    }
}

fn transition_key(
    host: &str,
    repository: &RepositoryObservation,
    pull_request: &PullRequestObservation,
    reviewer: &ReviewerObservation,
    checkpoint: &ReviewObservation,
    review_request_active: bool,
) -> String {
    let fields = [
        TRANSITION_DOMAIN,
        host,
        &repository.node_id,
        &pull_request.node_id,
        &reviewer.node_id,
        &checkpoint.node_id,
        checkpoint
            .commit_oid
            .as_deref()
            .expect("formal checkpoint commit was validated"),
        pull_request
            .head_oid
            .as_deref()
            .expect("actionable transition has a current head"),
        checkpoint
            .checkpoint_base_oid
            .as_deref()
            .unwrap_or("<unavailable>"),
        pull_request
            .current_base_oid
            .as_deref()
            .unwrap_or("<unavailable>"),
        if review_request_active {
            "review-requested"
        } else {
            "not-requested"
        },
    ];
    let mut digest = Sha256::new();
    for field in fields {
        digest.update(field.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn validate_actor(actor: &ActorObservation, code: &'static str) -> DecisionResult<()> {
    validate_login(&actor.login, code)?;
    validate_node(&actor.node_id, code)
}

fn validate_reviewer(reviewer: &ReviewerObservation, code: &'static str) -> DecisionResult<()> {
    ensure(reviewer.actor_type == "User", code)?;
    validate_login(&reviewer.login, code)?;
    validate_node(&reviewer.node_id, code)
}

fn validate_repository(
    repository: &RepositoryObservation,
    host: &str,
    code: &'static str,
) -> DecisionResult<()> {
    ensure(valid_repository(&repository.name_with_owner), code)?;
    validate_node(&repository.node_id, code)?;
    ensure(
        repository.url == format!("https://{host}/{}", repository.name_with_owner),
        code,
    )
}

fn validate_host(value: &str, code: &'static str) -> DecisionResult<()> {
    let bytes = value.as_bytes();
    ensure(!bytes.is_empty(), code)?;
    ensure(bytes[0].is_ascii_alphanumeric(), code)?;
    ensure(bytes[bytes.len() - 1].is_ascii_alphanumeric(), code)?;
    ensure(
        bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
        code,
    )
}

fn validate_login(value: &str, code: &'static str) -> DecisionResult<()> {
    let bytes = value.as_bytes();
    ensure((1..=255).contains(&bytes.len()), code)?;
    ensure(bytes[0].is_ascii_alphanumeric(), code)?;
    ensure(bytes[bytes.len() - 1].is_ascii_alphanumeric(), code)?;
    ensure(
        bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')),
        code,
    )
}

fn validate_node(value: &str, code: &'static str) -> DecisionResult<()> {
    let bytes = value.as_bytes();
    ensure((1..=256).contains(&bytes.len()), code)?;
    ensure(
        bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'+' | b'/' | b'=' | b'-')
        }),
        code,
    )
}

fn valid_repository(value: &str) -> bool {
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && !name.contains('/')
        && owner.bytes().all(valid_repository_byte)
        && name.bytes().all(valid_repository_byte)
}

fn valid_repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
}

fn validate_optional_oid(value: &Option<String>, code: &'static str) -> DecisionResult<()> {
    if let Some(value) = value {
        ensure(
            value.len() == 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
            code,
        )?;
    }
    Ok(())
}

fn parse_timestamp(value: &str, code: &'static str) -> DecisionResult<Timestamp> {
    let bytes = value.as_bytes();
    ensure(bytes.last() == Some(&b'Z'), code)?;
    let body = &value[..value.len() - 1];
    let (base, fraction) = match body.split_once('.') {
        Some((base, fraction)) => {
            ensure(!fraction.is_empty(), code)?;
            ensure(fraction.bytes().all(|byte| byte.is_ascii_digit()), code)?;
            (base, fraction)
        }
        None => (body, ""),
    };
    let bytes = base.as_bytes();
    ensure(bytes.len() == 19, code)?;
    ensure(
        bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':',
        code,
    )?;
    for (start, end) in [(0, 4), (5, 7), (8, 10), (11, 13), (14, 16), (17, 19)] {
        ensure(
            bytes[start..end].iter().all(|byte| byte.is_ascii_digit()),
            code,
        )?;
    }
    let component = |start: usize, end: usize| {
        base[start..end]
            .parse::<u32>()
            .map_err(|_| DecisionError(code))
    };
    let year = component(0, 4)?;
    let month = component(5, 7)?;
    let day = component(8, 10)?;
    let hour = component(11, 13)?;
    let minute = component(14, 16)?;
    let second = component(17, 19)?;
    ensure(year > 0 && (1..=12).contains(&month), code)?;
    let month_days = match month {
        2 if leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    ensure((1..=month_days).contains(&day), code)?;
    ensure(hour < 24 && minute < 60 && second < 60, code)?;
    Ok(Timestamp {
        components: [year, month, day, hour, minute, second],
        fraction: fraction.to_owned(),
    })
}

fn leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn compare_fraction(left: &str, right: &str) -> Ordering {
    let width = left.len().max(right.len());
    (0..width)
        .map(|index| {
            let left = left.as_bytes().get(index).copied().unwrap_or(b'0');
            let right = right.as_bytes().get(index).copied().unwrap_or(b'0');
            left.cmp(&right)
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn same_actor(left: &ActorObservation, right: &ActorObservation) -> bool {
    left.node_id == right.node_id && left.login.eq_ignore_ascii_case(&right.login)
}

fn same_reviewer(left: &ReviewerObservation, right: &ReviewerObservation) -> bool {
    left.node_id == right.node_id && left.login.eq_ignore_ascii_case(&right.login)
}

fn exactly_one<'a>(mut values: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn ensure(condition: bool, code: &'static str) -> DecisionResult<()> {
    if condition {
        Ok(())
    } else {
        Err(DecisionError(code))
    }
}
