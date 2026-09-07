use std::collections::VecDeque;

use serde_json::{Value, json};
use stratadiff::doctor::{
    DoctorCollectionStatus, DoctorCollectionSurface, DoctorPolicyKind, DoctorVerdict,
    evaluate_pull_request_doctor,
};
use stratadiff::readiness_audit::{
    GithubReadinessApi, GithubReadinessApiResponse, PullRequestDoctorCollection,
    collect_pull_request_doctor_snapshot,
};

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MERGE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";

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

#[test]
fn collects_only_the_exact_pr_base_and_head_with_classic_sources() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut api = StubApi::new(vec![
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
    ]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert_eq!(snapshot.target.base_ref, "release/1.x");
    assert_eq!(snapshot.target.base_sha, BASE_SHA);
    assert_eq!(snapshot.target.head_sha, HEAD_SHA);
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
    let mut api = StubApi::new(vec![
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
    ]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert_eq!(snapshot.collection.api_calls, 10);
    assert_eq!(snapshot.requirements.len(), 1);
    assert_eq!(snapshot.requirements[0].context, "lint");
    assert_eq!(snapshot.requirements[0].policies[0].id, "8");
}

#[test]
fn policy_visibility_and_incomplete_signals_are_partial() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut api = StubApi::new(vec![
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
    ]);

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
}

#[test]
fn an_explicitly_unprotected_base_skips_classic_protection_without_a_gap() {
    let initial_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    let mut api = StubApi::new(vec![
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
    ]);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Complete);
    assert!(snapshot.collection.gaps.is_empty());
    assert!(snapshot.requirements.is_empty());
    assert_eq!(snapshot.collection.api_calls, 9);
}

#[test]
fn test_merge_signals_make_the_head_only_diagnosis_inconclusive() {
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
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target && gap.reason.contains("test-merge commit")
    }));
    assert_eq!(
        evaluate_pull_request_doctor(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn a_missing_test_merge_sha_is_inconclusive() {
    let mut no_merge_pull = pull("release/1.x", BASE_SHA, HEAD_SHA, "open");
    no_merge_pull["merge_commit_sha"] = Value::Null;
    let mut responses = stable_empty_responses(no_merge_pull.clone());
    responses[0].1 = response(200, no_merge_pull);
    responses.remove(8);
    responses.remove(7);
    let mut api = StubApi::new(responses);

    let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
    api.finish();

    assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
    assert!(snapshot.collection.gaps.iter().any(|gap| {
        gap.surface == DoctorCollectionSurface::Target
            && gap.reason.contains("did not provide a test-merge SHA")
    }));
    assert_eq!(
        evaluate_pull_request_doctor(&snapshot).unwrap().verdict,
        DoctorVerdict::Inconclusive
    );
}

#[test]
fn merge_queue_and_required_workflow_rules_are_unsupported_targets() {
    for kind in ["merge_queue", "workflows"] {
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
        let mut api = StubApi::new(responses);

        let snapshot = collect_pull_request_doctor_snapshot(request(), &mut api).unwrap();
        api.finish();

        assert_eq!(snapshot.collection.status, DoctorCollectionStatus::Partial);
        assert!(
            snapshot
                .collection
                .gaps
                .iter()
                .any(|gap| gap.surface == DoctorCollectionSurface::Target)
        );
        assert_eq!(
            evaluate_pull_request_doctor(&snapshot).unwrap().verdict,
            DoctorVerdict::Inconclusive
        );
    }
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
