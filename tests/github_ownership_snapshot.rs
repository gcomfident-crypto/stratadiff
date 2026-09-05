use std::collections::VecDeque;

use serde_json::{Value, json};
use stratadiff::{
    codeowners::CodeownerIdentity,
    github_ownership::{
        GithubOwnershipApi, GithubOwnershipApiResponse, GithubOwnershipMediaType,
        collect_github_ownership_snapshot,
    },
    ownership::{MAX_TEAM_MEMBERS, RepositoryPermission},
};

const PROVIDER_URL: &str = "https://github.com";
const REPOSITORY: &str = "acme/widget";
const REPOSITORY_ENDPOINT: &str = "repos/acme/widget";
const BASE_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PAGE_SIZE: usize = 100;

#[derive(Debug)]
struct ExpectedCall {
    endpoint: String,
    media_type: GithubOwnershipMediaType,
    body: Vec<u8>,
    link_header: Option<String>,
}

#[derive(Default)]
struct FakeGithubOwnershipApi {
    expected: VecDeque<ExpectedCall>,
    calls: Vec<String>,
}

impl FakeGithubOwnershipApi {
    fn expect_json(&mut self, endpoint: impl Into<String>, value: Value) {
        self.expect_response(endpoint, GithubOwnershipMediaType::Json, value, None);
    }

    fn expect_team_repository(&mut self, endpoint: impl Into<String>, value: Value) {
        self.expect_response(
            endpoint,
            GithubOwnershipMediaType::TeamRepository,
            value,
            None,
        );
    }

    fn expect_page(
        &mut self,
        endpoint: impl Into<String>,
        value: Value,
        link_header: Option<String>,
    ) {
        self.expect_response(endpoint, GithubOwnershipMediaType::Json, value, link_header);
    }

    fn expect_response(
        &mut self,
        endpoint: impl Into<String>,
        media_type: GithubOwnershipMediaType,
        value: Value,
        link_header: Option<String>,
    ) {
        self.expected.push_back(ExpectedCall {
            endpoint: endpoint.into(),
            media_type,
            body: serde_json::to_vec(&value).expect("test JSON must encode"),
            link_header,
        });
    }

    fn assert_finished(&self) {
        assert!(
            self.expected.is_empty(),
            "{} expected API call(s) were not made; next was {:?}",
            self.expected.len(),
            self.expected.front().map(|call| call.endpoint.as_str())
        );
    }
}

impl GithubOwnershipApi for FakeGithubOwnershipApi {
    fn get(
        &mut self,
        endpoint: &str,
        media_type: GithubOwnershipMediaType,
    ) -> anyhow::Result<GithubOwnershipApiResponse> {
        self.calls.push(format!("{media_type:?} {endpoint}"));
        let expected = self
            .expected
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected GitHub API call to {endpoint}"));
        assert_eq!(
            endpoint, expected.endpoint,
            "GitHub API calls occurred in the wrong order"
        );
        assert_eq!(
            media_type, expected.media_type,
            "GitHub API call used the wrong media type for {endpoint}"
        );
        Ok(GithubOwnershipApiResponse {
            body: expected.body,
            link_header: expected.link_header,
        })
    }
}

fn repository() -> Value {
    json!({
        "id": 99,
        "full_name": REPOSITORY,
        "html_url": "https://github.com/acme/widget"
    })
}

fn commit() -> Value {
    json!({ "sha": BASE_COMMIT })
}

fn permission_name(permission: RepositoryPermission) -> &'static str {
    match permission {
        RepositoryPermission::Read | RepositoryPermission::Triage => "read",
        RepositoryPermission::Write | RepositoryPermission::Maintain => "write",
        RepositoryPermission::Admin => "admin",
    }
}

fn permission_bits(permission: RepositoryPermission) -> Value {
    match permission {
        RepositoryPermission::Read => json!({
            "pull": true,
            "triage": false,
            "push": false,
            "maintain": false,
            "admin": false
        }),
        RepositoryPermission::Triage => json!({
            "pull": true,
            "triage": true,
            "push": false,
            "maintain": false,
            "admin": false
        }),
        RepositoryPermission::Write => json!({
            "pull": true,
            "triage": true,
            "push": true,
            "maintain": false,
            "admin": false
        }),
        RepositoryPermission::Maintain => json!({
            "pull": true,
            "triage": true,
            "push": true,
            "maintain": true,
            "admin": false
        }),
        RepositoryPermission::Admin => json!({
            "pull": true,
            "triage": true,
            "push": true,
            "maintain": true,
            "admin": true
        }),
    }
}

fn team(id: u64, slug: &str, privacy: &str) -> Value {
    json!({
        "id": id,
        "slug": slug,
        "privacy": privacy,
        "organization": { "login": "acme" }
    })
}

fn team_repository(role_name: &str, permission: RepositoryPermission) -> Value {
    json!({
        "id": 99,
        "full_name": REPOSITORY,
        "role_name": role_name,
        "permissions": permission_bits(permission)
    })
}

fn member(id: u64, login: &str) -> Value {
    json!({
        "id": id,
        "login": login,
        "type": "User",
        "role": "member",
        "inherited": false
    })
}

fn user_permission(
    id: u64,
    login: &str,
    role_name: &str,
    permission: RepositoryPermission,
) -> Value {
    json!({
        "permission": permission_name(permission),
        "role_name": role_name,
        "user": {
            "id": id,
            "login": login,
            "type": "User"
        }
    })
}

fn commit_endpoint() -> String {
    format!("{REPOSITORY_ENDPOINT}/git/commits/{BASE_COMMIT}")
}

fn team_endpoint(slug: &str) -> String {
    format!("orgs/acme/teams/{slug}")
}

fn team_repository_endpoint(slug: &str) -> String {
    format!("{}/repos/{REPOSITORY}", team_endpoint(slug))
}

fn member_page_endpoint(slug: &str, page: usize) -> String {
    format!(
        "{}/members?role=all&per_page={PAGE_SIZE}&page={page}",
        team_endpoint(slug)
    )
}

fn member_page_url(slug: &str, page: usize) -> String {
    format!(
        "https://api.github.com/{}",
        member_page_endpoint(slug, page)
    )
}

fn user_permission_endpoint(login: &str) -> String {
    format!("{REPOSITORY_ENDPOINT}/collaborators/{login}/permission")
}

fn next_link(url: &str) -> String {
    format!("<{url}>; rel=\"next\"")
}

fn expect_observation_start(api: &mut FakeGithubOwnershipApi) {
    api.expect_json(REPOSITORY_ENDPOINT, repository());
    api.expect_json(commit_endpoint(), commit());
}

fn expect_observation_end(api: &mut FakeGithubOwnershipApi) {
    api.expect_json(REPOSITORY_ENDPOINT, repository());
    api.expect_json(commit_endpoint(), commit());
}

fn expect_team(
    api: &mut FakeGithubOwnershipApi,
    id: u64,
    slug: &str,
    role_name: &str,
    members: Vec<Value>,
) {
    api.expect_json(team_endpoint(slug), team(id, slug, "closed"));
    api.expect_team_repository(
        team_repository_endpoint(slug),
        team_repository(role_name, RepositoryPermission::Write),
    );
    api.expect_page(member_page_endpoint(slug, 1), Value::Array(members), None);
}

fn expect_user_permission(
    api: &mut FakeGithubOwnershipApi,
    requested_login: &str,
    response_id: u64,
    response_login: &str,
    role_name: &str,
    permission: RepositoryPermission,
) {
    api.expect_json(
        user_permission_endpoint(requested_login),
        user_permission(response_id, response_login, role_name, permission),
    );
}

fn collect(
    identities: &[CodeownerIdentity],
    api: &mut FakeGithubOwnershipApi,
) -> anyhow::Result<stratadiff::ownership::GithubOwnershipSnapshot> {
    collect_github_ownership_snapshot(PROVIDER_URL, REPOSITORY, BASE_COMMIT, identities, api)
}

fn assert_error_contains(error: &anyhow::Error, expected: &str) {
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains(expected),
        "expected error containing {expected:?}, got {rendered:?}"
    );
}

#[test]
fn reordered_observations_use_sorted_principal_calls_and_snapshot_rows() {
    let identities = vec![
        CodeownerIdentity::Team {
            organization: "ACME".to_owned(),
            slug: "Payments".to_owned(),
        },
        CodeownerIdentity::User {
            login: "ZARA".to_owned(),
        },
        CodeownerIdentity::Team {
            organization: "acme".to_owned(),
            slug: "core".to_owned(),
        },
        CodeownerIdentity::User {
            login: "adam".to_owned(),
        },
    ];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    expect_team(
        &mut api,
        300,
        "core",
        "release_engineer",
        vec![
            member(80, "CORE-HIGH"),
            member(20, "core-low"),
            member(50, "ADAM"),
        ],
    );
    expect_team(&mut api, 100, "payments", "write", vec![member(60, "pay")]);
    expect_user_permission(
        &mut api,
        "adam",
        50,
        "ADAM",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "core-high",
        80,
        "core-high",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "core-low",
        20,
        "CORE-LOW",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "pay",
        60,
        "PAY",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "zara",
        70,
        "zara",
        "deployment",
        RepositoryPermission::Write,
    );
    expect_observation_end(&mut api);

    expect_observation_start(&mut api);
    expect_team(
        &mut api,
        300,
        "core",
        "release_engineer",
        vec![
            member(50, "adam"),
            member(20, "CORE-LOW"),
            member(80, "core-high"),
        ],
    );
    expect_team(&mut api, 100, "payments", "write", vec![member(60, "PAY")]);
    expect_user_permission(
        &mut api,
        "adam",
        50,
        "adam",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "core-high",
        80,
        "CORE-HIGH",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "core-low",
        20,
        "core-low",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "pay",
        60,
        "pay",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "zara",
        70,
        "ZARA",
        "deployment",
        RepositoryPermission::Write,
    );
    expect_observation_end(&mut api);

    let snapshot = collect(&identities, &mut api).expect("stable observations must be accepted");

    api.assert_finished();
    snapshot
        .validate()
        .expect("collected snapshot must validate");
    assert_eq!(
        snapshot
            .users
            .iter()
            .map(|user| user.id)
            .collect::<Vec<_>>(),
        vec![20, 50, 60, 70, 80]
    );
    assert_eq!(
        snapshot
            .teams
            .iter()
            .map(|team| team.id)
            .collect::<Vec<_>>(),
        vec![100, 300]
    );
    assert_eq!(
        snapshot.teams[1]
            .members
            .iter()
            .map(|membership| membership.user_id)
            .collect::<Vec<_>>(),
        vec![20, 50, 80]
    );
    assert_eq!(
        snapshot.users[3].repository_permission,
        RepositoryPermission::Write
    );
    assert_eq!(
        snapshot.teams[1].repository_permission,
        RepositoryPermission::Write
    );
}

#[test]
fn permission_drift_between_observations_fails_closed() {
    let identities = vec![CodeownerIdentity::User {
        login: "alice".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    expect_user_permission(
        &mut api,
        "alice",
        11,
        "alice",
        "write",
        RepositoryPermission::Write,
    );
    expect_observation_end(&mut api);

    expect_observation_start(&mut api);
    expect_user_permission(
        &mut api,
        "alice",
        11,
        "alice",
        "read",
        RepositoryPermission::Read,
    );
    expect_observation_end(&mut api);

    let error = collect(&identities, &mut api).expect_err("permission drift must fail");

    api.assert_finished();
    assert_error_contains(
        &error,
        "ownership facts changed between two consecutive observations",
    );
}

#[test]
fn team_membership_drift_between_observations_fails_closed() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    expect_team(&mut api, 21, "payments", "write", vec![member(11, "alice")]);
    expect_user_permission(
        &mut api,
        "alice",
        11,
        "alice",
        "write",
        RepositoryPermission::Write,
    );
    expect_observation_end(&mut api);

    expect_observation_start(&mut api);
    expect_team(
        &mut api,
        21,
        "payments",
        "write",
        vec![member(12, "bob"), member(11, "alice")],
    );
    expect_user_permission(
        &mut api,
        "alice",
        11,
        "alice",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "bob",
        12,
        "bob",
        "write",
        RepositoryPermission::Write,
    );
    expect_observation_end(&mut api);

    let error = collect(&identities, &mut api).expect_err("membership drift must fail");

    api.assert_finished();
    assert_error_contains(
        &error,
        "ownership facts changed between two consecutive observations",
    );
}

#[test]
fn valid_next_link_collects_the_second_member_page() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    for _ in 0..2 {
        expect_observation_start(&mut api);
        api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
        api.expect_team_repository(
            team_repository_endpoint("payments"),
            team_repository("write", RepositoryPermission::Write),
        );
        let first_page = (1..=PAGE_SIZE)
            .map(|id| member(id as u64, &format!("member-{id:05}")))
            .collect();
        api.expect_page(
            member_page_endpoint("payments", 1),
            Value::Array(first_page),
            Some(next_link(&member_page_url("payments", 2))),
        );
        api.expect_page(
            member_page_endpoint("payments", 2),
            json!([member(101, "member-00101")]),
            None,
        );
        for id in 1..=101 {
            let login = format!("member-{id:05}");
            expect_user_permission(
                &mut api,
                &login,
                id as u64,
                &login,
                "write",
                RepositoryPermission::Write,
            );
        }
        expect_observation_end(&mut api);
    }

    let snapshot = collect(&identities, &mut api).expect("valid next link must be followed");

    api.assert_finished();
    assert_eq!(snapshot.users.len(), 101);
    assert_eq!(snapshot.teams[0].members.len(), 101);
    assert_eq!(snapshot.users[0].id, 1);
    assert_eq!(snapshot.users[100].id, 101);
}

#[test]
fn malformed_member_next_links_fail_closed() {
    let path = member_page_endpoint("payments", 2);
    let cases = [
        (
            "cross-origin",
            format!("<https://evil.example/{path}>; rel=\"next\""),
        ),
        (
            "wrong-path",
            next_link(
                "https://api.github.com/orgs/acme/teams/other/members?role=all&per_page=100&page=2",
            ),
        ),
        (
            "changed-role",
            next_link(
                "https://api.github.com/orgs/acme/teams/payments/members?role=maintainer&per_page=100&page=2",
            ),
        ),
        (
            "changed-page-size",
            next_link(
                "https://api.github.com/orgs/acme/teams/payments/members?role=all&per_page=99&page=2",
            ),
        ),
        (
            "skipped-page",
            next_link(
                "https://api.github.com/orgs/acme/teams/payments/members?role=all&per_page=100&page=3",
            ),
        ),
    ];

    for (case, link_header) in cases {
        let identities = vec![CodeownerIdentity::Team {
            organization: "acme".to_owned(),
            slug: "payments".to_owned(),
        }];
        let mut api = FakeGithubOwnershipApi::default();
        let first_page = (1..=PAGE_SIZE)
            .map(|id| member(id as u64, &format!("member-{id}")))
            .collect();

        expect_observation_start(&mut api);
        api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
        api.expect_team_repository(
            team_repository_endpoint("payments"),
            team_repository("write", RepositoryPermission::Write),
        );
        api.expect_page(
            member_page_endpoint("payments", 1),
            Value::Array(first_page),
            Some(link_header),
        );

        assert!(
            collect(&identities, &mut api).is_err(),
            "{case} pagination link must fail"
        );
        api.assert_finished();
    }
}

#[test]
fn short_member_page_with_next_link_fails_closed() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
    api.expect_team_repository(
        team_repository_endpoint("payments"),
        team_repository("write", RepositoryPermission::Write),
    );
    api.expect_page(
        member_page_endpoint("payments", 1),
        json!([member(11, "alice")]),
        Some(format!(
            "<{}>; rel=\"Next\"",
            member_page_url("payments", 2)
        )),
    );

    collect(&identities, &mut api).expect_err("short page with next link must fail");
    api.assert_finished();
}

#[test]
fn full_member_page_without_link_probes_the_next_page() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();
    let first_page = (1..=PAGE_SIZE as u64)
        .map(|id| member(id, &format!("member-{id}")))
        .collect();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
    api.expect_team_repository(
        team_repository_endpoint("payments"),
        team_repository("write", RepositoryPermission::Write),
    );
    api.expect_page(
        member_page_endpoint("payments", 1),
        Value::Array(first_page),
        None,
    );
    api.expect_page(
        member_page_endpoint("payments", 2),
        json!([member(1, "member-1")]),
        None,
    );

    let error = collect(&identities, &mut api).expect_err("duplicate member ID must fail");

    api.assert_finished();
    assert_error_contains(&error, "returned duplicate member ID 1");
}

#[test]
fn missing_team_member_inherited_field_fails_decoding() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
    api.expect_team_repository(
        team_repository_endpoint("payments"),
        team_repository("write", RepositoryPermission::Write),
    );
    api.expect_page(
        member_page_endpoint("payments", 1),
        json!([{
            "id": 11,
            "login": "alice",
            "type": "User",
            "role": "member"
        }]),
        None,
    );

    let error = collect(&identities, &mut api).expect_err("missing inherited must fail");

    api.assert_finished();
    assert_error_contains(&error, "missing field `inherited`");
}

#[test]
fn missing_team_repository_permissions_field_fails_decoding() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
    api.expect_team_repository(
        team_repository_endpoint("payments"),
        json!({
            "id": 99,
            "full_name": REPOSITORY,
            "role_name": "write"
        }),
    );

    let error = collect(&identities, &mut api).expect_err("missing permissions must fail");

    api.assert_finished();
    assert_error_contains(&error, "missing field `permissions`");
}

#[test]
fn missing_principal_permission_field_fails_decoding() {
    let identities = vec![CodeownerIdentity::User {
        login: "alice".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(
        user_permission_endpoint("alice"),
        json!({
            "role_name": "write",
            "user": {
                "id": 11,
                "login": "alice",
                "type": "User"
            }
        }),
    );

    let error = collect(&identities, &mut api).expect_err("missing permission must fail");

    api.assert_finished();
    assert_error_contains(&error, "missing field `permission`");
}

#[test]
fn secret_codeowner_team_fails_closed() {
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "secret"));

    let error = collect(&identities, &mut api).expect_err("secret team must fail");

    api.assert_finished();
    assert_error_contains(&error, "team @acme/payments is secret");
}

#[test]
fn email_codeowner_fails_before_any_api_call() {
    let identities = vec![CodeownerIdentity::Email {
        address: "reviewer@example.com".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    let error = collect(&identities, &mut api).expect_err("email owner must fail");

    assert!(
        api.calls.is_empty(),
        "email validation must precede API access"
    );
    api.assert_finished();
    assert_error_contains(
        &error,
        "cannot be collected as a stable GitHub reviewer identity",
    );
}

#[test]
fn duplicate_user_id_across_principal_permission_responses_fails() {
    let identities = vec![
        CodeownerIdentity::User {
            login: "bob".to_owned(),
        },
        CodeownerIdentity::User {
            login: "alice".to_owned(),
        },
    ];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    expect_user_permission(
        &mut api,
        "alice",
        11,
        "alice",
        "write",
        RepositoryPermission::Write,
    );
    expect_user_permission(
        &mut api,
        "bob",
        11,
        "bob",
        "write",
        RepositoryPermission::Write,
    );

    let error = collect(&identities, &mut api).expect_err("duplicate user ID must fail");

    api.assert_finished();
    assert_error_contains(&error, "duplicate");
}

#[test]
fn team_member_limit_plus_one_fails() {
    assert_eq!(MAX_TEAM_MEMBERS % PAGE_SIZE, 0);
    let identities = vec![CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: "payments".to_owned(),
    }];
    let mut api = FakeGithubOwnershipApi::default();

    expect_observation_start(&mut api);
    api.expect_json(team_endpoint("payments"), team(21, "payments", "closed"));
    api.expect_team_repository(
        team_repository_endpoint("payments"),
        team_repository("write", RepositoryPermission::Write),
    );
    for page in 1..=MAX_TEAM_MEMBERS / PAGE_SIZE {
        let first_id = (page - 1) * PAGE_SIZE + 1;
        let members = (first_id..first_id + PAGE_SIZE)
            .map(|id| member(id as u64, &format!("member-{id}")))
            .collect();
        api.expect_page(
            member_page_endpoint("payments", page),
            Value::Array(members),
            None,
        );
    }
    api.expect_page(
        member_page_endpoint("payments", MAX_TEAM_MEMBERS / PAGE_SIZE + 1),
        json!([member(
            (MAX_TEAM_MEMBERS + 1) as u64,
            "member-limit-plus-one"
        )]),
        None,
    );

    let error = collect(&identities, &mut api).expect_err("member limit plus one must fail");

    api.assert_finished();
    assert_error_contains(
        &error,
        &format!(
            "count limit exceeded: observed at least {}, limit {MAX_TEAM_MEMBERS}",
            MAX_TEAM_MEMBERS + 1
        ),
    );
}
