use std::collections::{BTreeMap, VecDeque};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use stratadiff::doctor::{
    DoctorCollectionStatus, DoctorCollectionSurface, DoctorEvaluationTargetKind,
    DoctorEvaluationTargetResolution, DoctorPolicyKind, DoctorVerdict,
    DoctorWorkflowCollectionStatus, DoctorWorkflowProbeKind, evaluate_pull_request_doctor_v2,
    evaluate_pull_request_doctor_v3, render_pull_request_doctor_v3_markdown,
};
use stratadiff::doctor_workflow::WorkflowTriggerCause;
use stratadiff::readiness_audit::{
    GithubPullRequestDoctorApi, GithubReadinessApi, GithubReadinessApiResponse,
    PullRequestDoctorCollection, collect_pull_request_doctor_snapshot,
    collect_pull_request_doctor_snapshot_v3,
};

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MERGE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";
const QUEUE_BASE_SHA: &str = "dddddddddddddddddddddddddddddddddddddddd";
const QUEUE_SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const WORKFLOW_BLOB_SHA: &str = "1111111111111111111111111111111111111111";
const DUPLICATE_WORKFLOW_BLOB_SHA: &str = "2222222222222222222222222222222222222222";

struct StubApi {
    responses: VecDeque<(String, GithubReadinessApiResponse)>,
    graphql_responses: VecDeque<GithubReadinessApiResponse>,
}

impl StubApi {
    fn new(responses: Vec<(String, GithubReadinessApiResponse)>) -> Self {
        let graphql_response = responses
            .first()
            .map(|(_, response)| graphql_response_for_pull(&response.body));
        Self {
            responses: responses.into(),
            graphql_responses: graphql_response.into_iter().cycle().take(2).collect(),
        }
    }

    fn with_graphql_responses(mut self, responses: Vec<GithubReadinessApiResponse>) -> Self {
        self.graphql_responses = responses.into();
        self
    }

    fn finish(self) {
        assert!(
            self.responses.is_empty(),
            "unused responses: {:?}",
            self.responses
                .iter()
                .map(|(endpoint, _)| endpoint)
                .collect::<Vec<_>>()
        );
    }
}

impl GithubPullRequestDoctorApi for StubApi {
    fn graphql(
        &mut self,
        query: &str,
        variables: &Value,
    ) -> anyhow::Result<GithubReadinessApiResponse> {
        assert!(query.contains("query StrataDiffPullRequestCandidate"));
        assert_eq!(
            variables,
            &json!({"owner": "acme", "name": "widgets", "number": 9})
        );
        Ok(self
            .graphql_responses
            .pop_front()
            .expect("unexpected GraphQL request"))
    }
}

impl GithubReadinessApi for StubApi {
    fn get(&mut self, endpoint: &str) -> anyhow::Result<GithubReadinessApiResponse> {
        let (expected, response) = self.responses.pop_front().expect("unexpected API request");
        assert_eq!(endpoint, expected);
        Ok(response)
    }
}

struct StableWorkflowDoctorApi {
    calls: BTreeMap<String, usize>,
    duplicate_producer: bool,
    drift_producer_on_second_pass: bool,
    graphql_calls: usize,
}

#[derive(Clone, Copy)]
enum PrHeadWorkflowScenario {
    MissingTrigger,
    BranchExcluded,
    PathExcluded,
    ForkApproval,
    ChangedFilesIncomplete,
    ChangedFilesOverLimit,
    ObservedRunOverridesBranchFilter,
    ObservedRunOverridesChangedFileLimit,
    ObservedRunOverridesIncompleteChangedFiles,
    ObservedRunOverridesMissingTrigger,
    PaginatedPathExcluded,
}

struct StablePrHeadWorkflowDoctorApi {
    calls: BTreeMap<String, usize>,
    graphql_calls: usize,
    scenario: PrHeadWorkflowScenario,
}

impl StablePrHeadWorkflowDoctorApi {
    fn new(scenario: PrHeadWorkflowScenario) -> Self {
        Self {
            calls: BTreeMap::new(),
            graphql_calls: 0,
            scenario,
        }
    }

    fn call_count(&self, endpoint: &str) -> usize {
        self.calls.get(endpoint).copied().unwrap_or(0)
    }

    fn pull(&self) -> Value {
        let changed_files = match self.scenario {
            PrHeadWorkflowScenario::ChangedFilesIncomplete
            | PrHeadWorkflowScenario::ObservedRunOverridesIncompleteChangedFiles => 2,
            PrHeadWorkflowScenario::ChangedFilesOverLimit
            | PrHeadWorkflowScenario::ObservedRunOverridesChangedFileLimit => 301,
            PrHeadWorkflowScenario::PaginatedPathExcluded => 101,
            _ => 1,
        };
        let head_repository = if matches!(self.scenario, PrHeadWorkflowScenario::ForkApproval) {
            "contributor/widgets"
        } else {
            "acme/widgets"
        };
        json!({
            "number": 9,
            "html_url": "https://github.com/acme/widgets/pull/9",
            "state": "open",
            "merge_commit_sha": null,
            "changed_files": changed_files,
            "base": {"ref": "release/1.x", "sha": BASE_SHA},
            "head": {
                "ref": "feature/doctor",
                "sha": HEAD_SHA,
                "repo": {"full_name": head_repository}
            }
        })
    }

    fn policy() -> Value {
        json!([{
            "type": "required_status_checks",
            "parameters": {
                "required_status_checks": [
                    {"context": "ci", "integration_id": 15368}
                ]
            },
            "ruleset_source_type": "Repository",
            "ruleset_source": "acme/widgets",
            "ruleset_id": 70
        }])
    }

    fn workflow_content(&self) -> Value {
        let trigger = match self.scenario {
            PrHeadWorkflowScenario::MissingTrigger
            | PrHeadWorkflowScenario::ObservedRunOverridesMissingTrigger => "on: push\n",
            PrHeadWorkflowScenario::BranchExcluded
            | PrHeadWorkflowScenario::ObservedRunOverridesBranchFilter => concat!(
                "on:\n",
                "  pull_request:\n",
                "    branches:\n",
                "      - main\n"
            ),
            PrHeadWorkflowScenario::PathExcluded
            | PrHeadWorkflowScenario::ChangedFilesIncomplete
            | PrHeadWorkflowScenario::ChangedFilesOverLimit
            | PrHeadWorkflowScenario::ObservedRunOverridesChangedFileLimit
            | PrHeadWorkflowScenario::ObservedRunOverridesIncompleteChangedFiles
            | PrHeadWorkflowScenario::PaginatedPathExcluded => concat!(
                "on:\n",
                "  pull_request:\n",
                "    paths:\n",
                "      - src/**\n"
            ),
            PrHeadWorkflowScenario::ForkApproval => "on: pull_request\n",
        };
        let yaml = format!(
            "name: CI\n{trigger}jobs:\n  ci:\n    name: ci\n    runs-on: ubuntu-latest\n    steps: []\n"
        );
        json!({
            "type": "file",
            "encoding": "base64",
            "content": STANDARD.encode(yaml),
            "path": ".github/workflows/ci.yml",
            "sha": WORKFLOW_BLOB_SHA
        })
    }

    fn target_runs(&self) -> Value {
        let conclusion = match self.scenario {
            PrHeadWorkflowScenario::ForkApproval => Some("action_required"),
            PrHeadWorkflowScenario::ObservedRunOverridesBranchFilter
            | PrHeadWorkflowScenario::ObservedRunOverridesChangedFileLimit
            | PrHeadWorkflowScenario::ObservedRunOverridesIncompleteChangedFiles
            | PrHeadWorkflowScenario::ObservedRunOverridesMissingTrigger => None,
            _ => return json!({"total_count": 0, "workflow_runs": []}),
        };
        let status = if conclusion.is_some() {
            "completed"
        } else {
            "queued"
        };
        json!({
            "total_count": 1,
            "workflow_runs": [{
                "id": 702,
                "html_url": "https://github.com/acme/widgets/actions/runs/702",
                "workflow_id": 901,
                "path": ".github/workflows/ci.yml",
                "event": "pull_request",
                "head_sha": HEAD_SHA,
                "check_suite_id": 602,
                "run_attempt": 1,
                "status": status,
                "conclusion": conclusion
            }]
        })
    }
}

impl StableWorkflowDoctorApi {
    fn new(duplicate_producer: bool) -> Self {
        Self {
            calls: BTreeMap::new(),
            duplicate_producer,
            drift_producer_on_second_pass: false,
            graphql_calls: 0,
        }
    }

    fn with_producer_drift(mut self) -> Self {
        self.drift_producer_on_second_pass = true;
        self
    }

    fn call_count(&self, endpoint: &str) -> usize {
        self.calls.get(endpoint).copied().unwrap_or(0)
    }

    fn pull() -> Value {
        json!({
            "number": 9,
            "html_url": "https://github.com/acme/widgets/pull/9",
            "state": "open",
            "merge_commit_sha": null,
            "base": {"ref": "release/1.x", "sha": BASE_SHA},
            "head": {
                "ref": "feature/doctor",
                "sha": HEAD_SHA,
                "repo": {"full_name": "acme/widgets"}
            }
        })
    }

    fn policy() -> Value {
        json!([
            {
                "type": "merge_queue",
                "parameters": {},
                "ruleset_source_type": "Repository",
                "ruleset_source": "acme/widgets",
                "ruleset_id": 70
            },
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [
                        {"context": "ci", "integration_id": 15368}
                    ]
                },
                "ruleset_source_type": "Repository",
                "ruleset_source": "acme/widgets",
                "ruleset_id": 70
            }
        ])
    }

    fn workflow_content(path: &str, sha: &str) -> Value {
        let yaml = concat!(
            "name: CI\n",
            "on: pull_request\n",
            "jobs:\n",
            "  ci:\n",
            "    name: ci\n",
            "    runs-on: ubuntu-latest\n",
            "    steps: []\n"
        );
        json!({
            "type": "file",
            "encoding": "base64",
            "content": STANDARD.encode(yaml),
            "path": path,
            "sha": sha
        })
    }
}

impl GithubPullRequestDoctorApi for StableWorkflowDoctorApi {
    fn graphql(
        &mut self,
        query: &str,
        variables: &Value,
    ) -> anyhow::Result<GithubReadinessApiResponse> {
        assert!(query.contains("query StrataDiffPullRequestCandidate"));
        assert_eq!(
            variables,
            &json!({"owner": "acme", "name": "widgets", "number": 9})
        );
        self.graphql_calls += 1;
        Ok(queued_graphql_response(QUEUE_SHA))
    }
}

impl GithubReadinessApi for StableWorkflowDoctorApi {
    fn get(&mut self, endpoint: &str) -> anyhow::Result<GithubReadinessApiResponse> {
        *self.calls.entry(endpoint.to_owned()).or_default() += 1;

        if endpoint == "repos/acme/widgets/pulls/9" {
            return Ok(response(200, Self::pull()));
        }
        if endpoint == "repos/acme/widgets" {
            return Ok(response(200, repository()));
        }
        if endpoint == "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1" {
            return Ok(response(200, Self::policy()));
        }
        if endpoint == "repos/acme/widgets/branches/release%2F1.x" {
            return Ok(response(
                200,
                json!({"name": "release/1.x", "protected": false}),
            ));
        }
        if endpoint
            == format!(
                "repos/acme/widgets/commits/{QUEUE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            )
        {
            return Ok(response(200, json!({"total_count": 0, "check_runs": []})));
        }
        if endpoint
            == format!("repos/acme/widgets/commits/{QUEUE_SHA}/statuses?per_page=100&page=1")
        {
            return Ok(response(200, json!([])));
        }
        if endpoint == format!("repos/acme/widgets/contents/.github/workflows?ref={QUEUE_SHA}") {
            let mut files = vec![json!({
                "type": "file",
                "path": ".github/workflows/ci.yml",
                "sha": WORKFLOW_BLOB_SHA
            })];
            if self.duplicate_producer {
                files.push(json!({
                    "type": "file",
                    "path": ".github/workflows/duplicate.yml",
                    "sha": DUPLICATE_WORKFLOW_BLOB_SHA
                }));
            }
            return Ok(response(200, Value::Array(files)));
        }
        if endpoint
            == format!("repos/acme/widgets/contents/.github/workflows/ci.yml?ref={QUEUE_SHA}")
        {
            return Ok(response(
                200,
                Self::workflow_content(".github/workflows/ci.yml", WORKFLOW_BLOB_SHA),
            ));
        }
        if endpoint
            == format!(
                "repos/acme/widgets/contents/.github/workflows/duplicate.yml?ref={QUEUE_SHA}"
            )
        {
            return Ok(response(
                200,
                Self::workflow_content(
                    ".github/workflows/duplicate.yml",
                    DUPLICATE_WORKFLOW_BLOB_SHA,
                ),
            ));
        }
        if endpoint
            == format!("repos/acme/widgets/actions/runs?head_sha={QUEUE_SHA}&per_page=100&page=1")
        {
            return Ok(response(
                200,
                json!({"total_count": 0, "workflow_runs": []}),
            ));
        }
        if endpoint
            == format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?check_name=ci&app_id=15368&filter=all&per_page=100&page=1"
            )
        {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 501,
                        "url": "https://api.github.com/repos/acme/widgets/check-runs/501",
                        "html_url": "https://github.com/acme/widgets/actions/runs/701/job/801",
                        "name": "ci",
                        "head_sha": HEAD_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "check_suite": {"id": 601}
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/runs?check_suite_id=601&per_page=100&page=1" {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "workflow_runs": [{
                        "id": 701,
                        "html_url": "https://github.com/acme/widgets/actions/runs/701",
                        "workflow_id": 901,
                        "path": ".github/workflows/ci.yml@refs/pull/9/merge",
                        "event": "pull_request",
                        "head_sha": HEAD_SHA,
                        "check_suite_id": 601,
                        "run_attempt": 2,
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/runs/701/jobs?filter=all&per_page=100&page=1" {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "jobs": [{
                        "id": 801,
                        "html_url": "https://github.com/acme/widgets/actions/runs/701/job/801",
                        "name": "ci",
                        "check_run_url": "https://api.github.com/repos/acme/widgets/check-runs/501",
                        "run_attempt": 2
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/workflows/901" {
            let workflow_url =
                if self.drift_producer_on_second_pass && self.call_count(endpoint) == 2 {
                    "https://github.com/acme/widgets/actions/workflows/renamed.yml"
                } else {
                    "https://github.com/acme/widgets/actions/workflows/ci.yml"
                };
            return Ok(response(
                200,
                json!({
                    "id": 901,
                    "name": "CI",
                    "path": ".github/workflows/ci.yml",
                    "state": "active",
                    "html_url": workflow_url
                }),
            ));
        }

        anyhow::bail!("unexpected API request: {endpoint}")
    }
}

impl GithubPullRequestDoctorApi for StablePrHeadWorkflowDoctorApi {
    fn graphql(
        &mut self,
        query: &str,
        variables: &Value,
    ) -> anyhow::Result<GithubReadinessApiResponse> {
        assert!(query.contains("query StrataDiffPullRequestCandidate"));
        assert_eq!(
            variables,
            &json!({"owner": "acme", "name": "widgets", "number": 9})
        );
        self.graphql_calls += 1;
        Ok(graphql_response_for_pull(
            &serde_json::to_vec(&self.pull()).unwrap(),
        ))
    }
}

impl GithubReadinessApi for StablePrHeadWorkflowDoctorApi {
    fn get(&mut self, endpoint: &str) -> anyhow::Result<GithubReadinessApiResponse> {
        *self.calls.entry(endpoint.to_owned()).or_default() += 1;

        if endpoint == "repos/acme/widgets/pulls/9" {
            return Ok(response(200, self.pull()));
        }
        if endpoint == "repos/acme/widgets" {
            return Ok(response(200, repository()));
        }
        if endpoint == "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1" {
            return Ok(response(200, Self::policy()));
        }
        if endpoint == "repos/acme/widgets/branches/release%2F1.x" {
            return Ok(response(
                200,
                json!({"name": "release/1.x", "protected": false}),
            ));
        }
        if endpoint
            == format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            )
        {
            return Ok(response(200, json!({"total_count": 0, "check_runs": []})));
        }
        if endpoint == format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1")
        {
            return Ok(response(200, json!([])));
        }
        if endpoint == format!("repos/acme/widgets/contents/.github/workflows?ref={HEAD_SHA}") {
            return Ok(response(
                200,
                json!([{
                    "type": "file",
                    "path": ".github/workflows/ci.yml",
                    "sha": WORKFLOW_BLOB_SHA
                }]),
            ));
        }
        if endpoint
            == format!("repos/acme/widgets/contents/.github/workflows/ci.yml?ref={HEAD_SHA}")
        {
            return Ok(response(200, self.workflow_content()));
        }
        if endpoint
            == format!("repos/acme/widgets/actions/runs?head_sha={HEAD_SHA}&per_page=100&page=1")
        {
            return Ok(response(200, self.target_runs()));
        }
        if endpoint == "repos/acme/widgets/pulls/9/files?per_page=100&page=1" {
            if matches!(self.scenario, PrHeadWorkflowScenario::PaginatedPathExcluded) {
                let files = (0..100)
                    .map(|index| json!({"filename": format!("docs/file-{index:03}.md")}))
                    .collect::<Vec<_>>();
                return Ok(response_with_next(200, Value::Array(files)));
            }
            return Ok(response(200, json!([{"filename": "docs/readme.md"}])));
        }
        if endpoint == "repos/acme/widgets/pulls/9/files?per_page=100&page=2"
            && matches!(self.scenario, PrHeadWorkflowScenario::PaginatedPathExcluded)
        {
            return Ok(response(200, json!([{"filename": "docs/final-page.md"}])));
        }
        if endpoint
            == format!(
                "repos/acme/widgets/commits/{BASE_SHA}/check-runs?check_name=ci&app_id=15368&filter=all&per_page=100&page=1"
            )
        {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 501,
                        "url": "https://api.github.com/repos/acme/widgets/check-runs/501",
                        "html_url": "https://github.com/acme/widgets/actions/runs/701/job/801",
                        "name": "ci",
                        "head_sha": BASE_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "check_suite": {"id": 601}
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/runs?check_suite_id=601&per_page=100&page=1" {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "workflow_runs": [{
                        "id": 701,
                        "html_url": "https://github.com/acme/widgets/actions/runs/701",
                        "workflow_id": 901,
                        "path": ".github/workflows/ci.yml",
                        "event": "push",
                        "head_sha": BASE_SHA,
                        "check_suite_id": 601,
                        "run_attempt": 1,
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/runs/701/jobs?filter=all&per_page=100&page=1" {
            return Ok(response(
                200,
                json!({
                    "total_count": 1,
                    "jobs": [{
                        "id": 801,
                        "html_url": "https://github.com/acme/widgets/actions/runs/701/job/801",
                        "name": "ci",
                        "check_run_url": "https://api.github.com/repos/acme/widgets/check-runs/501",
                        "run_attempt": 1
                    }]
                }),
            ));
        }
        if endpoint == "repos/acme/widgets/actions/workflows/901" {
            return Ok(response(
                200,
                json!({
                    "id": 901,
                    "name": "CI",
                    "path": ".github/workflows/ci.yml",
                    "state": "active",
                    "html_url": "https://github.com/acme/widgets/actions/workflows/ci.yml"
                }),
            ));
        }

        anyhow::bail!("unexpected API request: {endpoint}")
    }
}

fn response(status: u16, body: Value) -> GithubReadinessApiResponse {
    GithubReadinessApiResponse {
        status,
        body: serde_json::to_vec(&body).unwrap(),
        link_header: None,
    }
}

fn response_with_next(status: u16, body: Value) -> GithubReadinessApiResponse {
    GithubReadinessApiResponse {
        status,
        body: serde_json::to_vec(&body).unwrap(),
        link_header: Some(
            "<https://api.github.com/resource?per_page=100&page=2>; rel=\"next\"".to_owned(),
        ),
    }
}

fn graphql_response_for_pull(body: &[u8]) -> GithubReadinessApiResponse {
    let pull: Value = serde_json::from_slice(body).unwrap();
    let potential_merge_commit = match &pull["merge_commit_sha"] {
        Value::String(sha) => json!({"oid": sha}),
        Value::Null => Value::Null,
        value => panic!("unexpected merge_commit_sha fixture: {value}"),
    };
    response(
        200,
        json!({
            "data": {
                "repository": {
                    "nameWithOwner": "acme/widgets",
                    "url": "https://github.com/acme/widgets",
                    "pullRequest": {
                        "number": pull["number"],
                        "url": pull["html_url"],
                        "state": "OPEN",
                        "baseRefName": pull["base"]["ref"],
                        "baseRefOid": pull["base"]["sha"],
                        "headRefOid": pull["head"]["sha"],
                        "mergeable": "MERGEABLE",
                        "mergeStateStatus": "BLOCKED",
                        "isMergeQueueEnabled": false,
                        "isInMergeQueue": false,
                        "potentialMergeCommit": potential_merge_commit,
                        "mergeQueueEntry": null
                    }
                }
            }
        }),
    )
}

fn queued_graphql_response(candidate_sha: &str) -> GithubReadinessApiResponse {
    response(
        200,
        json!({
            "data": {
                "repository": {
                    "nameWithOwner": "acme/widgets",
                    "url": "https://github.com/acme/widgets",
                    "pullRequest": {
                        "number": 9,
                        "url": "https://github.com/acme/widgets/pull/9",
                        "state": "OPEN",
                        "baseRefName": "release/1.x",
                        "baseRefOid": BASE_SHA,
                        "headRefOid": HEAD_SHA,
                        "mergeable": "MERGEABLE",
                        "mergeStateStatus": "BLOCKED",
                        "isMergeQueueEnabled": true,
                        "isInMergeQueue": true,
                        "potentialMergeCommit": {"oid": MERGE_SHA},
                        "mergeQueueEntry": {
                            "id": "MQE_fixture_9",
                            "state": "AWAITING_CHECKS",
                            "position": 1,
                            "baseCommit": {"oid": QUEUE_BASE_SHA},
                            "headCommit": {"oid": candidate_sha},
                            "pullRequest": {"number": 9, "headRefOid": HEAD_SHA}
                        }
                    }
                }
            }
        }),
    )
}

fn queued_without_entry_graphql_response() -> GithubReadinessApiResponse {
    let mut body: Value = serde_json::from_slice(&queued_graphql_response(QUEUE_SHA).body).unwrap();
    body["data"]["repository"]["pullRequest"]["mergeQueueEntry"] = Value::Null;
    response(200, body)
}

fn queued_rest_responses() -> Vec<(String, GithubReadinessApiResponse)> {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull.clone()),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(
                200,
                json!([
                    {
                        "type": "merge_queue",
                        "parameters": {},
                        "ruleset_source_type": "Repository",
                        "ruleset_source": "acme/widgets",
                        "ruleset_id": 70
                    },
                    {
                        "type": "required_status_checks",
                        "parameters": {
                            "required_status_checks": [
                                {"context": "ci", "integration_id": 15368}
                            ]
                        },
                        "ruleset_source_type": "Repository",
                        "ruleset_source": "acme/widgets",
                        "ruleset_id": 70
                    }
                ]),
            ),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": false})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{QUEUE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 901,
                        "html_url": "https://github.com/acme/widgets/runs/901",
                        "name": "ci",
                        "head_sha": QUEUE_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ),
        ),
        (
            format!("repos/acme/widgets/commits/{QUEUE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull),
        ),
    ]
}

fn request() -> PullRequestDoctorCollection<'static> {
    PullRequestDoctorCollection {
        provider_url: "https://github.com",
        repository: "acme/widgets",
        captured_at: "2026-09-08T00:00:00Z",
        pull_request_number: 9,
    }
}

fn pull(base_ref: &str, base_sha: &str, head_sha: &str, state: &str) -> Value {
    json!({
        "number": 9,
        "html_url": "https://github.com/acme/widgets/pull/9",
        "state": state,
        "merge_commit_sha": MERGE_SHA,
        "base": {"ref": base_ref, "sha": base_sha},
        "head": {"sha": head_sha}
    })
}

fn repository() -> Value {
    json!({
        "id": 42,
        "full_name": "acme/widgets",
        "html_url": "https://github.com/acme/widgets",
        "default_branch": "main"
    })
}

fn stable_empty_responses(final_pull: Value) -> Vec<(String, GithubReadinessApiResponse)> {
    vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, pull("release/1.x", BASE_SHA, HEAD_SHA, "open")),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": true})),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x/protection".to_owned(),
            response(200, json!({"required_status_checks": null})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{MERGE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{MERGE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, final_pull),
        ),
    ]
}

fn append_policy_revalidation(
    responses: &mut Vec<(String, GithubReadinessApiResponse)>,
    start: usize,
    end: usize,
) {
    let policy = responses[start..end].to_vec();
    responses.extend(policy);
}

#[test]
fn collects_only_the_exact_pr_base_and_head_with_classic_sources() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull.clone()),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(
                200,
                json!([{
                    "type": "required_status_checks",
                    "parameters": {
                        "required_status_checks": [{"context": "lint", "integration_id": 15368}]
                    },
                    "ruleset_source_type": "Repository",
                    "ruleset_source": "acme/widgets",
                    "ruleset_id": 7
                }]),
            ),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": true})),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x/protection".to_owned(),
            response(
                200,
                json!({
                    "required_status_checks": {
                        "strict": true,
                        "contexts": ["legacy", "deploy", "lint"],
                        "checks": [
                            {"context": "deploy", "app_id": 99},
                            {"context": "lint", "app_id": 15368},
                            {"context": "security", "app_id": null}
                        ]
                    }
                }),
            ),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 101,
                        "html_url": "https://github.com/acme/widgets/runs/101",
                        "name": "lint",
                        "head_sha": HEAD_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ),
        ),
        (
            format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
            response(
                200,
                json!([
                    {
                        "id": 302,
                        "url": "https://api.github.com/repos/acme/widgets/statuses/302",
                        "context": "legacy",
                        "creator": {"id": 5, "login": "ci-bot"},
                        "state": "failure"
                    },
                    {
                        "id": 301,
                        "url": "https://api.github.com/repos/acme/widgets/statuses/301",
                        "context": "legacy",
                        "creator": {"id": 5, "login": "ci-bot"},
                        "state": "success"
                    }
                ]),
            ),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{MERGE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{MERGE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull),
        ),
    ];
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert_eq!(snapshot.target.base_ref, "release/1.x");
    assert_eq!(snapshot.target.base_sha, BASE_SHA);
    assert_eq!(snapshot.target.head_sha, HEAD_SHA);
    assert_eq!(snapshot.signal_sha, HEAD_SHA);
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Selected
    );
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::ChecksBlocked
    );
    assert_eq!(snapshot.check_runs.len(), 1);
    assert_eq!(snapshot.check_runs[0].name, "lint");
    assert_eq!(snapshot.statuses.len(), 1);
    assert_eq!(snapshot.statuses[0].id, 302);
    assert_eq!(snapshot.statuses[0].state, "failure");

    let requirement_keys = snapshot
        .requirements
        .iter()
        .map(|requirement| (requirement.context.as_str(), requirement.expected_app_id))
        .collect::<Vec<_>>();
    assert_eq!(
        requirement_keys,
        vec![
            ("deploy", Some(99)),
            ("legacy", None),
            ("lint", Some(15368)),
            ("security", None),
        ]
    );
    let lint = snapshot
        .requirements
        .iter()
        .find(|requirement| requirement.context == "lint")
        .unwrap();
    assert_eq!(lint.policies.len(), 2);
    assert_eq!(lint.policies[0].kind, DoctorPolicyKind::Ruleset);
    assert_eq!(lint.policies[1].kind, DoctorPolicyKind::BranchProtection);
    assert!(!snapshot.requirements.iter().any(
        |requirement| requirement.context == "deploy" && requirement.expected_app_id.is_none()
    ));
}

#[test]
fn follows_effective_rule_pagination_before_calling_collection_complete() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull.clone()),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response_with_next(
                200,
                json!([{
                    "type": "pull_request",
                    "parameters": null,
                    "ruleset_source_type": "Repository",
                    "ruleset_source": "acme/widgets",
                    "ruleset_id": 7
                }]),
            ),
        ),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=2".to_owned(),
            response(
                200,
                json!([{
                    "type": "required_status_checks",
                    "parameters": {
                        "required_status_checks": [{"context": "lint", "integration_id": 15368}]
                    },
                    "ruleset_source_type": "Repository",
                    "ruleset_source": "acme/widgets",
                    "ruleset_id": 8
                }]),
            ),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": false})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 101,
                        "html_url": "https://github.com/acme/widgets/runs/101",
                        "name": "lint",
                        "head_sha": HEAD_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ),
        ),
        (
            format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{MERGE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{MERGE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull),
        ),
    ];
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert_eq!(snapshot.collection.api_calls, 15);
    assert_eq!(snapshot.requirements.len(), 1);
    assert_eq!(snapshot.requirements[0].context, "lint");
    assert_eq!(snapshot.requirements[0].policies[0].id, "8");
}

#[test]
fn policy_visibility_and_incomplete_signals_are_partial() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull.clone()),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(403, json!({"message": "Resource not accessible"})),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(404, json!({"message": "Not Found"})),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x/protection".to_owned(),
            response(404, json!({"message": "Not Found"})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(
                200,
                json!({
                    "total_count": 2,
                    "check_runs": [{
                        "id": 101,
                        "html_url": "https://github.com/acme/widgets/runs/101",
                        "name": "lint",
                        "head_sha": HEAD_SHA,
                        "app": {"id": 15368, "slug": "github-actions"},
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ),
        ),
        (
            format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
            response(403, json!({"message": "Resource not accessible"})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{MERGE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{MERGE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull),
        ),
    ];
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
    assert!(snapshot.requirements.is_empty());
    let surfaces = snapshot
        .collection
        .gaps
        .iter()
        .map(|gap| gap.surface)
        .collect::<Vec<_>>();
    assert_eq!(
        surfaces,
        vec![
            DoctorCollectionSurface::Target,
            DoctorCollectionSurface::Requirements,
            DoctorCollectionSurface::Requirements,
            DoctorCollectionSurface::Requirements,
            DoctorCollectionSurface::CheckRuns,
            DoctorCollectionSurface::CommitStatuses,
        ]
    );
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn an_explicitly_unprotected_base_skips_classic_protection_without_a_gap() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull.clone()),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": false})),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            format!(
                "repos/acme/widgets/commits/{MERGE_SHA}/check-runs?filter=latest&per_page=100&page=1"
            ),
            response(200, json!({"total_count": 0, "check_runs": []})),
        ),
        (
            format!("repos/acme/widgets/commits/{MERGE_SHA}/statuses?per_page=100&page=1"),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, initial_pull),
        ),
    ];
    append_policy_revalidation(&mut responses, 2, 4);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert!(snapshot.collection.gaps.is_empty());
    assert!(snapshot.requirements.is_empty());
    assert_eq!(snapshot.collection.api_calls, 13);
}

#[test]
fn test_merge_signals_select_the_test_merge_evaluation_target() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = stable_empty_responses(initial_pull);
    responses[7].1 = response(
        200,
        json!({
            "total_count": 1,
            "check_runs": [{
                "id": 901,
                "html_url": "https://github.com/acme/widgets/runs/901",
                "name": "merge-test",
                "head_sha": MERGE_SHA,
                "app": {"id": 15368, "slug": "github-actions"},
                "status": "completed",
                "conclusion": "success"
            }]
        }),
    );
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert_eq!(
        snapshot.target.evaluation.kind,
        DoctorEvaluationTargetKind::TestMerge
    );
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Selected
    );
    assert_eq!(snapshot.target.evaluation.sha, MERGE_SHA);
    assert_eq!(snapshot.signal_sha, MERGE_SHA);
    assert_eq!(snapshot.check_runs.len(), 1);
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::ChecksClear
    );
}

#[test]
fn candidate_second_read_error_never_leaves_a_selected_target() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let initial_graphql = graphql_response_for_pull(&serde_json::to_vec(&initial_pull).unwrap());
    let mut responses = stable_empty_responses(initial_pull);
    responses[7].1 = response(
        200,
        json!({
            "total_count": 1,
            "check_runs": [{
                "id": 901,
                "html_url": "https://github.com/acme/widgets/runs/901",
                "name": "merge-test",
                "head_sha": MERGE_SHA,
                "app": {"id": 15368, "slug": "github-actions"},
                "status": "completed",
                "conclusion": "success"
            }]
        }),
    );
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses).with_graphql_responses(vec![
        initial_graphql,
        response(
            200,
            json!({
                "data": null,
                "errors": [{"message": "candidate temporarily unavailable"}]
            }),
        ),
    ]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Provisional
    );
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target
            && gap.reason.contains("could not revalidate")
    }));
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn policy_drift_requires_a_fresh_snapshot() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = stable_empty_responses(initial_pull);
    responses.extend([
        (
            "repos/acme/widgets/rules/branches/release%2F1.x?per_page=100&page=1".to_owned(),
            response(
                200,
                json!([{
                    "type": "required_status_checks",
                    "parameters": {
                        "required_status_checks": [{"context": "new-ruleset-check", "integration_id": 15368}]
                    },
                    "ruleset_source_type": "Repository",
                    "ruleset_source": "acme/widgets",
                    "ruleset_id": 70
                }]),
            ),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x".to_owned(),
            response(200, json!({"name": "release/1.x", "protected": true})),
        ),
        (
            "repos/acme/widgets/branches/release%2F1.x/protection".to_owned(),
            response(
                200,
                json!({
                    "required_status_checks": {
                        "contexts": ["new-classic-check"],
                        "checks": []
                    }
                }),
            ),
        ),
    ]);
    let mut api = StubApi::new(responses);

    let error = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap_err();
    api.finish();

    assert!(
        error
            .to_string()
            .contains("policy or its visibility changed during collection")
    );
    assert!(error.to_string().contains("retry the doctor command"));
}

#[test]
fn a_missing_test_merge_sha_is_inconclusive() {
    let mut no_merge_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    no_merge_pull["merge_commit_sha"] = Value::Null;
    let mut responses = stable_empty_responses(no_merge_pull.clone());
    responses[0].1 = response(200, no_merge_pull);
    responses.remove(8);
    responses.remove(7);
    append_policy_revalidation(&mut responses, 2, 5);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Provisional
    );
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target
            && gap.reason.contains("did not provide a test-merge SHA")
    }));
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn only_required_workflow_rules_remain_an_unsupported_target() {
    for (kind, expected_partial, expected_verdict) in [
        ("merge_queue", false, DoctorVerdict::ChecksClear),
        ("workflows", true, DoctorVerdict::Inconclusive),
    ] {
        let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
        let mut responses = stable_empty_responses(initial_pull);
        responses[2].1 = response(
            200,
            json!([{
            "type": kind,
            "parameters": {},
            "ruleset_source_type": "Repository",
            "ruleset_source": "acme/widgets",
            "ruleset_id": 70
            }]),
        );
        append_policy_revalidation(&mut responses, 2, 5);
        let mut api = StubApi::new(responses);

        let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
        api.finish();

        assert_eq!(
            snapshot.collection.status == DoctorCollectionStatus::Partial,
            expected_partial
        );
        assert_eq!(
            snapshot
                .collection
                .gaps
                .iter()
                .any(|gap| gap.surface == DoctorCollectionSurface::Target),
            expected_partial
        );
        assert_eq!(
            evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
            expected_verdict
        );
    }
}

#[test]
fn v3_live_collection_proves_a_unique_exact_sha_merge_group_trigger_gap() {
    let mut api = StableWorkflowDoctorApi::new(false);

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();
    let markdown = render_pull_request_doctor_v3_markdown(&report);

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Complete
    );
    assert_eq!(
        snapshot.workflow_collection.inventory.as_ref().unwrap().sha,
        QUEUE_SHA
    );
    assert_eq!(
        report.workflow_trigger_diagnoses[0]
            .diagnosis
            .as_ref()
            .unwrap()
            .cause_code,
        WorkflowTriggerCause::MergeGroupTriggerMissing
    );
    assert_eq!(api.graphql_calls, 6);
    assert!(markdown.find("## Answer").unwrap() < markdown.find("## Claim boundary").unwrap());
    assert!(markdown.contains("unique static workflow-job producer in the exact-SHA inventory"));
    assert!(markdown.contains("does not subscribe to <code>merge&#95;group</code>"));
    assert!(markdown.contains("then verify that the required check appears"));
    assert_eq!(
        api.call_count(&format!(
            "repos/acme/widgets/contents/.github/workflows?ref={QUEUE_SHA}"
        )),
        2
    );
    assert_eq!(
        api.call_count(&format!(
            "repos/acme/widgets/contents/.github/workflows/ci.yml?ref={QUEUE_SHA}"
        )),
        2
    );
    assert_eq!(
        api.call_count(&format!(
            "repos/acme/widgets/actions/runs?head_sha={QUEUE_SHA}&per_page=100&page=1"
        )),
        2
    );
    assert_eq!(
        api.call_count(&format!(
            "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?check_name=ci&app_id=15368&filter=all&per_page=100&page=1"
        )),
        2
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/actions/runs?check_suite_id=601&per_page=100&page=1"),
        2
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/actions/runs/701/jobs?filter=all&per_page=100&page=1"),
        2
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/actions/workflows/901"),
        2
    );
}

#[test]
fn v3_live_collection_rejects_second_pass_producer_drift() {
    let mut api = StableWorkflowDoctorApi::new(false).with_producer_drift();

    let error = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workflow producer evidence changed during diagnosis")
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/actions/workflows/901"),
        2
    );
}

#[test]
fn v3_live_collection_abstains_when_exact_sha_inventory_has_two_producers() {
    let mut api = StableWorkflowDoctorApi::new(true);

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Partial
    );
    assert!(snapshot.workflow_collection.gaps.iter().any(|gap| {
        gap.code == "workflow_producer_not_unique" && gap.requirement.context == "ci"
    }));
    assert!(
        snapshot.workflow_trigger_investigations[0]
            .producer
            .is_some()
    );
    assert!(snapshot.workflow_trigger_investigations[0].input.is_none());
    assert!(report.workflow_trigger_diagnoses[0].diagnosis.is_none());
    assert_eq!(
        api.call_count(&format!(
            "repos/acme/widgets/contents/.github/workflows/duplicate.yml?ref={QUEUE_SHA}"
        )),
        2
    );
}

#[test]
fn v3_live_collection_diagnoses_pr_head_trigger_failures_from_base_bound_producers() {
    let cases = [
        (
            PrHeadWorkflowScenario::MissingTrigger,
            WorkflowTriggerCause::PullRequestTriggerMissing,
        ),
        (
            PrHeadWorkflowScenario::BranchExcluded,
            WorkflowTriggerCause::WorkflowBranchFilterExcluded,
        ),
        (
            PrHeadWorkflowScenario::PathExcluded,
            WorkflowTriggerCause::WorkflowPathFilterExcluded,
        ),
        (
            PrHeadWorkflowScenario::ForkApproval,
            WorkflowTriggerCause::ForkApprovalRequired,
        ),
    ];

    for (scenario, expected_cause) in cases {
        let mut api = StablePrHeadWorkflowDoctorApi::new(scenario);

        let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
        let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

        assert_eq!(
            snapshot.target.evaluation.kind,
            DoctorEvaluationTargetKind::PrHead
        );
        assert_eq!(
            snapshot.target.evaluation.resolution,
            DoctorEvaluationTargetResolution::Provisional
        );
        assert_eq!(
            snapshot.workflow_collection.status,
            DoctorWorkflowCollectionStatus::Complete
        );
        assert_eq!(snapshot.workflow_collection.probes.len(), 1);
        assert_eq!(
            snapshot.workflow_collection.probes[0].kind,
            DoctorWorkflowProbeKind::PullRequestBase
        );
        assert_eq!(snapshot.workflow_collection.probes[0].sha, BASE_SHA);
        let investigation = &snapshot.workflow_trigger_investigations[0];
        assert_eq!(
            investigation.producer.as_ref().unwrap().source_sha,
            BASE_SHA
        );
        assert_eq!(
            investigation.input.as_ref().unwrap().changed_files.paths,
            ["docs/readme.md"]
        );
        assert_eq!(
            report.workflow_trigger_diagnoses[0]
                .diagnosis
                .as_ref()
                .unwrap()
                .cause_code,
            expected_cause
        );
        assert_eq!(api.graphql_calls, 6);
        assert_eq!(
            api.call_count("repos/acme/widgets/pulls/9/files?per_page=100&page=1"),
            2
        );
        assert_eq!(
            api.call_count(&format!(
                "repos/acme/widgets/commits/{BASE_SHA}/check-runs?check_name=ci&app_id=15368&filter=all&per_page=100&page=1"
            )),
            2
        );
    }
}

#[test]
fn v3_pr_head_observed_run_overrides_a_static_branch_exclusion() {
    let mut api = StablePrHeadWorkflowDoctorApi::new(
        PrHeadWorkflowScenario::ObservedRunOverridesBranchFilter,
    );

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Complete
    );
    let diagnosis = report.workflow_trigger_diagnoses[0]
        .diagnosis
        .as_ref()
        .unwrap();
    assert_eq!(diagnosis.cause_code, WorkflowTriggerCause::None);
    assert_eq!(
        diagnosis.evidence,
        [
            "base:release/1.x",
            "head:feature/doctor",
            "branches:main",
            "run:queued",
        ]
    );
}

#[test]
fn v3_pr_head_abstains_when_changed_files_are_incomplete() {
    let mut api =
        StablePrHeadWorkflowDoctorApi::new(PrHeadWorkflowScenario::ChangedFilesIncomplete);

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Partial
    );
    assert!(snapshot.workflow_collection.gaps.iter().any(|gap| {
        gap.code == "changed_files_incomplete" && gap.reason.contains("declared total")
    }));
    assert!(snapshot.workflow_trigger_investigations[0].input.is_none());
    assert!(report.workflow_trigger_diagnoses[0].diagnosis.is_none());
}

#[test]
fn v3_pr_head_collects_every_changed_file_page_before_path_diagnosis() {
    let mut api = StablePrHeadWorkflowDoctorApi::new(PrHeadWorkflowScenario::PaginatedPathExcluded);

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    let input = snapshot.workflow_trigger_investigations[0]
        .input
        .as_ref()
        .unwrap();
    assert!(input.changed_files.complete);
    assert_eq!(input.changed_files.total, 101);
    assert_eq!(input.changed_files.paths.len(), 101);
    assert_eq!(
        report.workflow_trigger_diagnoses[0]
            .diagnosis
            .as_ref()
            .unwrap()
            .cause_code,
        WorkflowTriggerCause::WorkflowPathFilterExcluded
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/pulls/9/files?per_page=100&page=1"),
        2
    );
    assert_eq!(
        api.call_count("repos/acme/widgets/pulls/9/files?per_page=100&page=2"),
        2
    );
}

#[test]
fn v3_pr_head_abstains_beyond_githubs_path_filter_file_limit() {
    let mut api = StablePrHeadWorkflowDoctorApi::new(PrHeadWorkflowScenario::ChangedFilesOverLimit);

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Partial
    );
    assert!(
        snapshot.workflow_collection.gaps.iter().any(|gap| {
            gap.code == "changed_files_incomplete" && gap.reason.contains("first 300")
        })
    );
    assert!(report.workflow_trigger_diagnoses[0].diagnosis.is_none());
    assert_eq!(
        api.call_count("repos/acme/widgets/pulls/9/files?per_page=100&page=1"),
        0
    );
}

#[test]
fn v3_pr_head_observed_run_overrides_a_missing_static_trigger() {
    let mut api = StablePrHeadWorkflowDoctorApi::new(
        PrHeadWorkflowScenario::ObservedRunOverridesMissingTrigger,
    );

    let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
    let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

    assert_eq!(
        snapshot.workflow_collection.status,
        DoctorWorkflowCollectionStatus::Complete
    );
    assert!(snapshot.workflow_collection.gaps.is_empty());
    let diagnosis = report.workflow_trigger_diagnoses[0]
        .diagnosis
        .as_ref()
        .unwrap();
    assert_eq!(diagnosis.cause_code, WorkflowTriggerCause::None);
    assert_eq!(
        diagnosis.evidence,
        [
            "workflow:.github/workflows/ci.yml",
            "event:pull_request",
            "run_head:exact",
        ]
    );
}

#[test]
fn v3_pr_head_observed_run_does_not_require_changed_file_completeness() {
    let cases = [
        (
            PrHeadWorkflowScenario::ObservedRunOverridesIncompleteChangedFiles,
            2,
            false,
        ),
        (
            PrHeadWorkflowScenario::ObservedRunOverridesChangedFileLimit,
            301,
            true,
        ),
    ];

    for (scenario, expected_total, expected_limit) in cases {
        let mut api = StablePrHeadWorkflowDoctorApi::new(scenario);

        let snapshot = collect_pull_request_doctor_snapshot_v3(request(), &mut api).unwrap();
        let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap();

        assert_eq!(
            snapshot.workflow_collection.status,
            DoctorWorkflowCollectionStatus::Complete
        );
        assert!(snapshot.workflow_collection.gaps.is_empty());
        let input = snapshot.workflow_trigger_investigations[0]
            .input
            .as_ref()
            .unwrap();
        assert!(!input.changed_files.complete);
        assert_eq!(input.changed_files.total, expected_total);
        assert_eq!(
            input.changed_files.github_filter_file_limit_reached,
            expected_limit
        );
        let diagnosis = report.workflow_trigger_diagnoses[0]
            .diagnosis
            .as_ref()
            .unwrap();
        assert_eq!(diagnosis.cause_code, WorkflowTriggerCause::None);
        assert_eq!(
            diagnosis.evidence,
            [
                "workflow:.github/workflows/ci.yml",
                "event:pull_request",
                "run_head:exact",
            ]
        );
    }
}

#[test]
fn queued_pull_uses_the_graphql_queue_candidate_and_fails_closed_on_provenance() {
    let graphql = queued_graphql_response(QUEUE_SHA);
    let mut responses = queued_rest_responses();
    append_policy_revalidation(&mut responses, 2, 4);
    let mut api = StubApi::new(responses).with_graphql_responses(vec![graphql.clone(), graphql]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(
        snapshot.target.evaluation.kind,
        DoctorEvaluationTargetKind::MergeGroup
    );
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Provisional
    );
    assert_eq!(snapshot.target.evaluation.sha, QUEUE_SHA);
    assert_eq!(snapshot.signal_sha, QUEUE_SHA);
    assert_eq!(
        snapshot.target.evaluation.base_sha.as_deref(),
        Some(QUEUE_BASE_SHA)
    );
    assert_eq!(
        snapshot.target.evaluation.queue_entry_id.as_deref(),
        Some("MQE_fixture_9")
    );
    assert_eq!(snapshot.check_runs.len(), 1);
    assert_eq!(snapshot.check_runs[0].name, "ci");
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target && gap.reason.contains("merge_group webhook")
    }));
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn queued_candidate_drift_requires_a_fresh_snapshot() {
    let mut api = StubApi::new(queued_rest_responses()).with_graphql_responses(vec![
        queued_graphql_response(QUEUE_SHA),
        queued_graphql_response("ffffffffffffffffffffffffffffffffffffffff"),
    ]);

    let error = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap_err();
    api.finish();

    assert!(
        error
            .to_string()
            .contains("evaluation candidate changed during collection")
    );
}

#[test]
fn queued_pull_without_an_entry_uses_only_a_provisional_diagnostic_sha() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut responses = stable_empty_responses(initial_pull);
    responses[2].1 = response(
        200,
        json!([{
            "type": "merge_queue",
            "parameters": {},
            "ruleset_source_type": "Repository",
            "ruleset_source": "acme/widgets",
            "ruleset_id": 70
        }]),
    );
    responses.remove(8);
    responses.remove(7);
    append_policy_revalidation(&mut responses, 2, 5);
    let graphql = queued_without_entry_graphql_response();
    let mut api = StubApi::new(responses).with_graphql_responses(vec![graphql.clone(), graphql]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(
        snapshot.target.evaluation.kind,
        DoctorEvaluationTargetKind::PrHead
    );
    assert_eq!(
        snapshot.target.evaluation.resolution,
        DoctorEvaluationTargetResolution::Provisional
    );
    assert_eq!(snapshot.signal_sha, HEAD_SHA);
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target
            && gap
                .reason
                .contains("did not expose its queue entry candidate")
    }));
    assert_eq!(
        evaluate_pull_request_doctor_v2(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn rejects_a_check_run_from_any_sha_other_than_the_exact_head() {
    let mut responses = stable_empty_responses(pull("release/1.x", BASE_SHA, HEAD_SHA, "open"));
    responses.truncate(5);
    responses.push((
        format!(
            "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
        ),
        response(
            200,
            json!({
                "total_count": 1,
                "check_runs": [{
                    "id": 101,
                    "html_url": "https://github.com/acme/widgets/runs/101",
                    "name": "lint",
                    "head_sha": "cccccccccccccccccccccccccccccccccccccccc",
                    "app": {"id": 15368, "slug": "github-actions"},
                    "status": "completed",
                    "conclusion": "success"
                }]
            }),
        ),
    ));
    let mut api = StubApi::new(responses);

    let error = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap_err();
    api.finish();
    assert!(error.to_string().contains("different head SHA"));
}

#[test]
fn reread_rejects_target_identity_or_state_drift() {
    let mut merge_drift = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    merge_drift["merge_commit_sha"] = json!("dddddddddddddddddddddddddddddddddddddddd");
    let drifted_pulls = [
        pull("main", BASE_SHA, HEAD_SHA, "open"),
        pull(
            "release/1.x",
            "cccccccccccccccccccccccccccccccccccccccc",
            HEAD_SHA,
            "open",
        ),
        pull(
            "release/1.x",
            BASE_SHA,
            "dddddddddddddddddddddddddddddddddddddddd",
            "open",
        ),
        pull("release/1.x", BASE_SHA, HEAD_SHA, "closed"),
        merge_drift,
    ];
    for drifted_pull in drifted_pulls {
        let mut api = StubApi::new(stable_empty_responses(drifted_pull));
        let error = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap_err();
        api.finish();
        assert!(
            error
                .to_string()
                .contains("pull request target changed during collection")
        );
    }
}

#[test]
fn rejects_closed_pulls_and_noncanonical_head_ids_before_policy_requests() {
    let invalid_pulls = [
        pull("release/1.x", BASE_SHA, HEAD_SHA, "closed"),
        pull(
            "release/1.x",
            BASE_SHA,
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
            "open",
        ),
    ];
    for invalid_pull in invalid_pulls {
        let mut api = StubApi::new(vec![
            (
                "repos/acme/widgets/pulls/9".to_owned(),
                response(200, invalid_pull),
            ),
            ("repos/acme/widgets".to_owned(), response(200, repository())),
        ]);
        assert!(collect_pull_request_doctor_snapshot(request(), &mut api).is_err());
        api.finish();
    }
}

#[test]
fn rejects_a_pull_url_outside_the_bound_provider_repository() {
    let mut hostile_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    hostile_pull["html_url"] = json!("https://attacker.example/acme/widgets/pull/9");
    let mut api = StubApi::new(vec![
        (
            "repos/acme/widgets/pulls/9".to_owned(),
            response(200, hostile_pull),
        ),
        ("repos/acme/widgets".to_owned(), response(200, repository())),
    ]);

    let error = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap_err();
    api.finish();
    assert!(
        error
            .to_string()
            .contains("pull request URL does not match")
    );
}
