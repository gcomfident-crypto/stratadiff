use std::collections::VecDeque;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use stratadiff::readiness::{
    AuditVerdict, CollectionStatus, FindingRule, evaluate_merge_readiness,
};
use stratadiff::readiness_audit::{
    GithubReadinessApi, GithubReadinessApiResponse, MergeReadinessCollection,
    collect_merge_readiness_snapshot, parse_workflow_definition,
};

struct StubApi {
    responses: VecDeque<(String, GithubReadinessApiResponse)>,
}

impl StubApi {
    fn new(responses: Vec<(String, GithubReadinessApiResponse)>) -> Self {
        Self {
            responses: responses.into(),
        }
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

impl GithubReadinessApi for StubApi {
    fn get(&mut self, endpoint: &str) -> anyhow::Result<GithubReadinessApiResponse> {
        let (expected, response) = self.responses.pop_front().expect("unexpected API request");
        assert_eq!(endpoint, expected);
        Ok(response)
    }
}

fn response(status: u16, body: serde_json::Value) -> GithubReadinessApiResponse {
    GithubReadinessApiResponse {
        status,
        body: serde_json::to_vec(&body).unwrap(),
        link_header: None,
    }
}

fn repository_responses() -> Vec<(String, GithubReadinessApiResponse)> {
    vec![
        (
            "repos/acme/widgets".to_owned(),
            response(
                200,
                json!({
                    "id": 42,
                    "full_name": "acme/widgets",
                    "html_url": "https://github.com/acme/widgets",
                    "default_branch": "main"
                }),
            ),
        ),
        (
            "repos/acme/widgets/git/ref/heads/main".to_owned(),
            response(
                200,
                json!({"object": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}),
            ),
        ),
    ]
}

fn request() -> MergeReadinessCollection<'static> {
    MergeReadinessCollection {
        provider_url: "https://github.com",
        repository: "acme/widgets",
        captured_at: "2026-09-08T00:00:00Z",
        pull_request_limit: 1,
    }
}

#[test]
fn repository_response_cannot_change_the_requested_provider() {
    let mut api = StubApi::new(vec![(
        "repos/acme/widgets".to_owned(),
        response(
            200,
            json!({
                "id": 42,
                "full_name": "acme/widgets",
                "html_url": "https://attacker.example/acme/widgets",
                "default_branch": "main"
            }),
        ),
    )]);

    let error = collect_merge_readiness_snapshot(request(), &mut api).unwrap_err();
    api.finish();
    assert!(
        error
            .to_string()
            .contains("repository URL does not match the requested provider")
    );
}

#[test]
fn parses_supported_workflow_trigger_forms() {
    let scalar = parse_workflow_definition(b"name: CI\non: pull_request\n").unwrap();
    assert!(scalar.pull_request.is_some());
    assert!(scalar.merge_group.is_none());

    let sequence =
        parse_workflow_definition(b"name: CI\non: [pull_request, merge_group]\n").unwrap();
    assert!(sequence.pull_request.is_some());
    assert!(sequence.merge_group.is_some());

    let mapping = parse_workflow_definition(
        b"name: CI\non:\n  pull_request:\n    paths: ['src/**']\n  merge_group:\n    paths-ignore: docs/**\n",
    )
    .unwrap();
    assert_eq!(
        mapping.pull_request.unwrap().paths,
        vec!["src/**".to_owned()]
    );
    assert_eq!(
        mapping.merge_group.unwrap().paths_ignore,
        vec!["docs/**".to_owned()]
    );
}

#[test]
fn live_shape_finds_an_unprotected_default_branch_without_sampling_source() {
    let mut responses = repository_responses();
    responses.extend([
        (
            "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/rules/branches/main".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/branches?protected=true&per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
    ]);
    let mut api = StubApi::new(responses);
    let snapshot = collect_merge_readiness_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, CollectionStatus::Complete);
    assert!(snapshot.pull_requests.is_empty());
    let report = evaluate_merge_readiness(&snapshot).unwrap();
    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(
        report.findings[0].rule,
        FindingRule::DefaultBranchUnprotected
    );
}

#[test]
fn live_shape_maps_a_required_check_to_its_filtered_workflow() {
    let mut responses = repository_responses();
    responses.extend([
        (
            "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1".to_owned(),
            response(
                200,
                json!([{
                    "id": 7,
                    "name": "main protection",
                    "source_type": "Repository",
                    "source": "acme/widgets",
                    "_links": {"html": {"href": "https://github.com/acme/widgets/rules/7"}}
                }]),
            ),
        ),
        (
            "repos/acme/widgets/rules/branches/main".to_owned(),
            response(
                200,
                json!([
                    {
                        "type": "pull_request",
                        "ruleset_source_type": "Repository",
                        "ruleset_source": "acme/widgets",
                        "ruleset_id": 7
                    },
                    {
                        "type": "merge_queue",
                        "ruleset_source_type": "Repository",
                        "ruleset_source": "acme/widgets",
                        "ruleset_id": 7
                    },
                    {
                        "type": "required_status_checks",
                        "parameters": {
                            "required_status_checks": [{"context": "test", "integration_id": 15368}]
                        },
                        "ruleset_source_type": "Repository",
                        "ruleset_source": "acme/widgets",
                        "ruleset_id": 7
                    }
                ]),
            ),
        ),
        (
            "repos/acme/widgets/branches?protected=true&per_page=100&page=1".to_owned(),
            response(200, json!([{"name": "main", "protected": true}])),
        ),
        (
            "repos/acme/widgets/branches/main/protection".to_owned(),
            response(404, json!({"message": "Branch not protected"})),
        ),
        (
            "repos/acme/widgets/pulls?state=all&sort=updated&direction=desc&base=main&per_page=100&page=1".to_owned(),
            response(
                200,
                json!([{
                    "number": 9,
                    "html_url": "https://github.com/acme/widgets/pull/9",
                    "state": "open",
                    "merged_at": null,
                    "base": {"ref": "main"},
                    "head": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
                }]),
            ),
        ),
        (
            "repos/acme/widgets/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/check-runs?filter=all&per_page=100&page=1".to_owned(),
            response(
                200,
                json!({
                    "total_count": 1,
                    "check_runs": [{
                        "id": 101,
                        "html_url": "https://github.com/acme/widgets/runs/101",
                        "name": "test",
                        "app": {"id": 15368, "slug": "github-actions"},
                        "check_suite": {"id": 55},
                        "status": "completed",
                        "conclusion": "success"
                    }]
                }),
            ),
        ),
        (
            "repos/acme/widgets/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/statuses?per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/actions/runs?head_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb&per_page=100&page=1".to_owned(),
            response(
                200,
                json!({
                    "total_count": 1,
                    "workflow_runs": [{
                        "id": 202,
                        "html_url": "https://github.com/acme/widgets/actions/runs/202",
                        "workflow_id": 303,
                        "path": ".github/workflows/ci.yml",
                        "event": "pull_request",
                        "check_suite_id": 55,
                        "run_attempt": 1
                    }]
                }),
            ),
        ),
        (
            "repos/acme/widgets/actions/workflows?per_page=100&page=1".to_owned(),
            response(
                200,
                json!({
                    "total_count": 1,
                    "workflows": [{
                        "id": 303,
                        "name": "CI",
                        "path": ".github/workflows/ci.yml",
                        "state": "active",
                        "html_url": "https://github.com/acme/widgets/blob/main/.github/workflows/ci.yml"
                    }]
                }),
            ),
        ),
        (
            "repos/acme/widgets/contents/.github/workflows/ci.yml?ref=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            response(
                200,
                json!({
                    "type": "file",
                    "encoding": "base64",
                    "content": STANDARD.encode("name: CI\non:\n  pull_request:\n    paths: ['src/**']\n")
                }),
            ),
        ),
    ]);
    let mut api = StubApi::new(responses);
    let snapshot = collect_merge_readiness_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, CollectionStatus::Partial);
    let report = evaluate_merge_readiness(&snapshot).unwrap();
    let rules = report
        .findings
        .iter()
        .map(|finding| finding.rule)
        .collect::<Vec<_>>();
    assert!(rules.contains(&FindingRule::MergeGroupTriggerMissing));
    assert!(rules.contains(&FindingRule::PathFilteredRequiredWorkflow));
}

#[test]
fn inaccessible_policy_surfaces_are_inconclusive_not_unprotected() {
    let mut responses = repository_responses();
    responses.extend([
        (
            "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1".to_owned(),
            response(403, json!({"message": "Resource not accessible"})),
        ),
        (
            "repos/acme/widgets/rules/branches/main".to_owned(),
            response(403, json!({"message": "Resource not accessible"})),
        ),
        (
            "repos/acme/widgets/branches?protected=true&per_page=100&page=1".to_owned(),
            response(403, json!({"message": "Resource not accessible"})),
        ),
    ]);
    let mut api = StubApi::new(responses);
    let snapshot = collect_merge_readiness_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, CollectionStatus::Partial);
    let report = evaluate_merge_readiness(&snapshot).unwrap();
    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
}

#[test]
fn provider_protected_branch_without_visible_policy_is_inconclusive() {
    let mut responses = repository_responses();
    responses.extend([
        (
            "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/rules/branches/main".to_owned(),
            response(200, json!([])),
        ),
        (
            "repos/acme/widgets/branches?protected=true&per_page=100&page=1".to_owned(),
            response(200, json!([{"name": "main", "protected": true}])),
        ),
        (
            "repos/acme/widgets/branches/main/protection".to_owned(),
            response(404, json!({"message": "Not Found"})),
        ),
    ]);
    let mut api = StubApi::new(responses);
    let snapshot = collect_merge_readiness_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert!(!snapshot.branch_policy.branch_protected);
    assert_eq!(snapshot.collection.status, CollectionStatus::Partial);
    let report = evaluate_merge_readiness(&snapshot).unwrap();
    assert_eq!(report.summary.verdict, AuditVerdict::Inconclusive);
    assert!(report.findings.is_empty());
}
