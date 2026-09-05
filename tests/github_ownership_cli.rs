#![cfg(unix)]

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use stratadiff::ownership::{GithubOwnershipSnapshot, RepositoryPermission};
use tempfile::TempDir;

const REPOSITORY: &str = "acme/widget";
const PROVIDER_URL: &str = "https://github.com";
const ACCEPT_HEADER: &str = "Accept: application/vnd.github+json";
const TEAM_REPOSITORY_ACCEPT_HEADER: &str = "Accept: application/vnd.github.v3.repository+json";
const API_VERSION_HEADER: &str = "X-GitHub-Api-Version: 2022-11-28";

const GH_STUB: &str = r#"#!/bin/sh
set -eu

for argument in "$@"; do
    printf '%s\t' "$argument" >> "$GH_STUB_LOG"
done
printf '\n' >> "$GH_STUB_LOG"

test "$#" -eq 11
test "$1" = api
test "$2" = --include
test "$3" = --method
test "$4" = GET
test "$5" = --hostname
test "$6" = github.com
test "$7" = --header
test "$9" = --header
test "${10}" = 'X-GitHub-Api-Version: 2022-11-28'

endpoint=${11}
if [ "$endpoint" = "orgs/acme/teams/reviewers/repos/acme/widget" ]; then
    test "$8" = 'Accept: application/vnd.github.v3.repository+json'
else
    test "$8" = 'Accept: application/vnd.github+json'
fi

respond() {
    printf 'HTTP/2.0 200 OK\r\n'
    printf 'Content-Type: application/json; charset=utf-8\r\n'
    printf 'X-GitHub-Api-Version-Selected: 2022-11-28\r\n'
    printf '\r\n'
    printf '%s\n' "$1"
}

case "$endpoint" in
    "repos/acme/widget")
        respond '{"id":99,"full_name":"acme/widget","html_url":"https://github.com/acme/widget","url":"https://api.github.com/repos/acme/widget"}'
        ;;
    "repos/acme/widget/git/commits/$GH_STUB_BASE")
        respond "{\"sha\":\"$GH_STUB_BASE\"}"
        ;;
    "orgs/acme/teams/reviewers")
        respond '{"id":23,"slug":"reviewers","privacy":"closed","organization":{"login":"acme"}}'
        ;;
    "orgs/acme/teams/reviewers/repos/acme/widget")
        respond '{"id":99,"full_name":"acme/widget","role_name":"write","permissions":{"pull":true,"triage":true,"push":true,"maintain":false,"admin":false}}'
        ;;
    "orgs/acme/teams/reviewers/members?role=all&per_page=100&page=1")
        respond '[{"id":18,"login":"bob","type":"User","role":"member","inherited":true}]'
        ;;
    "repos/acme/widget/collaborators/alice/permission")
        permission_call=0
        if [ -f "$GH_STUB_COUNTER" ]; then
            IFS= read -r permission_call < "$GH_STUB_COUNTER"
        fi
        permission_call=$((permission_call + 1))
        printf '%s\n' "$permission_call" > "$GH_STUB_COUNTER"

        user_id=17
        if [ "$GH_STUB_DRIFT" = 1 ] && [ "$permission_call" -eq 2 ]; then
            user_id=18
        fi
        respond "{\"permission\":\"write\",\"role_name\":\"security-reviewer\",\"user\":{\"id\":$user_id,\"login\":\"alice\",\"type\":\"User\"}}"
        ;;
    "repos/acme/widget/collaborators/bob/permission")
        respond '{"permission":"write","role_name":"team-reviewer","user":{"id":18,"login":"bob","type":"User"}}'
        ;;
    *)
        printf 'unexpected endpoint: %s\n' "$endpoint" >&2
        exit 64
        ;;
esac
"#;

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    repository: PathBuf,
    base: String,
    output: PathBuf,
    command_path: OsString,
    log: PathBuf,
    counter: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::with_codeowners("* @alice\n")
    }

    fn with_codeowners(codeowners: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_owned();
        let repository = root.join("repository");
        fs::create_dir_all(repository.join(".github")).unwrap();
        git(&repository, &["init", "--quiet"]);
        git(&repository, &["config", "user.name", "StrataDiff Test"]);
        git(
            &repository,
            &["config", "user.email", "stratadiff@example.com"],
        );
        git(&repository, &["config", "commit.gpgsign", "false"]);
        fs::write(repository.join(".github/CODEOWNERS"), codeowners).unwrap();
        git(&repository, &["add", ".github/CODEOWNERS"]);
        git(&repository, &["commit", "--quiet", "-m", "add CODEOWNERS"]);
        let base = String::from_utf8(git(&repository, &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();

        let command_directory = root.join("bin");
        fs::create_dir(&command_directory).unwrap();
        let gh = command_directory.join("gh");
        fs::write(&gh, GH_STUB).unwrap();
        let mut permissions = fs::metadata(&gh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh, permissions).unwrap();

        let mut paths = vec![command_directory];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));

        Self {
            _directory: directory,
            root: root.clone(),
            repository,
            base,
            output: root.join("ownership.json"),
            command_path: env::join_paths(paths).unwrap(),
            log: root.join("gh-argv.log"),
            counter: root.join("permission-count"),
        }
    }

    fn run(&self, drift: bool) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stratadiff"))
            .arg("github-ownership-snapshot")
            .arg(&self.base)
            .arg("--repo")
            .arg(&self.repository)
            .arg("--github-repository")
            .arg(REPOSITORY)
            .arg("--provider-url")
            .arg(PROVIDER_URL)
            .arg("--output")
            .arg(&self.output)
            .env("PATH", &self.command_path)
            .env("GH_STUB_BASE", &self.base)
            .env("GH_STUB_LOG", &self.log)
            .env("GH_STUB_COUNTER", &self.counter)
            .env("GH_STUB_DRIFT", if drift { "1" } else { "0" })
            .output()
            .unwrap()
    }

    fn assert_two_complete_observations(&self) {
        let repository_endpoint = "repos/acme/widget".to_owned();
        let commit_endpoint = format!("repos/acme/widget/git/commits/{}", self.base);
        let permission_endpoint = "repos/acme/widget/collaborators/alice/permission".to_owned();
        let expected_endpoints = vec![
            repository_endpoint.clone(),
            commit_endpoint.clone(),
            permission_endpoint.clone(),
            repository_endpoint.clone(),
            commit_endpoint.clone(),
            repository_endpoint.clone(),
            commit_endpoint.clone(),
            permission_endpoint,
            repository_endpoint,
            commit_endpoint,
        ];
        let calls = self.calls();
        assert_eq!(calls.len(), expected_endpoints.len(), "calls: {calls:#?}");

        for (call, endpoint) in calls.iter().zip(&expected_endpoints) {
            let expected = vec![
                "api",
                "--include",
                "--method",
                "GET",
                "--hostname",
                "github.com",
                "--header",
                ACCEPT_HEADER,
                "--header",
                API_VERSION_HEADER,
                endpoint,
            ];
            assert_eq!(
                call.iter().map(String::as_str).collect::<Vec<_>>(),
                expected
            );
        }
        assert_eq!(fs::read_to_string(&self.counter).unwrap(), "2\n");
        assert!(
            !calls
                .iter()
                .flatten()
                .any(|argument| argument.starts_with("repos/acme/widget/collaborators?")),
            "the collector must not enumerate every repository collaborator"
        );
    }

    fn assert_two_complete_team_observations(&self) {
        let repository_endpoint = "repos/acme/widget".to_owned();
        let commit_endpoint = format!("repos/acme/widget/git/commits/{}", self.base);
        let team_endpoint = "orgs/acme/teams/reviewers".to_owned();
        let team_repository_endpoint = "orgs/acme/teams/reviewers/repos/acme/widget".to_owned();
        let members_endpoint =
            "orgs/acme/teams/reviewers/members?role=all&per_page=100&page=1".to_owned();
        let alice_permission_endpoint =
            "repos/acme/widget/collaborators/alice/permission".to_owned();
        let bob_permission_endpoint = "repos/acme/widget/collaborators/bob/permission".to_owned();
        let one_observation = vec![
            repository_endpoint.clone(),
            commit_endpoint.clone(),
            team_endpoint,
            team_repository_endpoint.clone(),
            members_endpoint,
            alice_permission_endpoint,
            bob_permission_endpoint,
            repository_endpoint,
            commit_endpoint,
        ];
        let expected_endpoints = one_observation
            .iter()
            .chain(one_observation.iter())
            .collect::<Vec<_>>();
        let calls = self.calls();
        assert_eq!(calls.len(), expected_endpoints.len(), "calls: {calls:#?}");

        for (call, endpoint) in calls.iter().zip(expected_endpoints) {
            let accept = if endpoint == &team_repository_endpoint {
                TEAM_REPOSITORY_ACCEPT_HEADER
            } else {
                ACCEPT_HEADER
            };
            let expected = vec![
                "api",
                "--include",
                "--method",
                "GET",
                "--hostname",
                "github.com",
                "--header",
                accept,
                "--header",
                API_VERSION_HEADER,
                endpoint,
            ];
            assert_eq!(
                call.iter().map(String::as_str).collect::<Vec<_>>(),
                expected
            );
        }
        assert_eq!(fs::read_to_string(&self.counter).unwrap(), "2\n");
        assert!(
            !calls
                .iter()
                .flatten()
                .any(|argument| argument.starts_with("repos/acme/widget/collaborators?")),
            "the collector must not enumerate every repository collaborator"
        );
    }

    fn calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| line.split_terminator('\t').map(str::to_owned).collect())
            .collect()
    }

    fn assert_no_snapshot_temporaries(&self) {
        let leftovers = fs::read_dir(&self.root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with(".stratadiff-ownership-"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temporary files remain: {leftovers:?}"
        );
    }
}

fn git(repository: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn writes_a_valid_private_snapshot_after_two_complete_observations() {
    let fixture = Fixture::new();

    let output = fixture.run(false);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("wrote stable GitHub ownership snapshot")
    );
    let snapshot: GithubOwnershipSnapshot =
        serde_json::from_slice(&fs::read(&fixture.output).unwrap()).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.provider_url, PROVIDER_URL);
    assert_eq!(snapshot.repository_id, 99);
    assert_eq!(snapshot.base_commit, fixture.base);
    assert_eq!(snapshot.api_version, "2022-11-28");
    assert_eq!(snapshot.users.len(), 1);
    assert_eq!(snapshot.users[0].id, 17);
    assert_eq!(snapshot.users[0].login, "alice");
    assert_eq!(
        snapshot.users[0].repository_permission,
        RepositoryPermission::Write
    );
    assert!(snapshot.teams.is_empty());
    assert_eq!(
        fs::metadata(&fixture.output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fixture.assert_two_complete_observations();
    fixture.assert_no_snapshot_temporaries();
}

#[test]
fn uses_team_specific_media_type_and_resolves_inherited_members_individually() {
    let fixture = Fixture::with_codeowners("* @alice @acme/reviewers\n");

    let output = fixture.run(false);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let snapshot: GithubOwnershipSnapshot =
        serde_json::from_slice(&fs::read(&fixture.output).unwrap()).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.users.len(), 2);
    assert_eq!(snapshot.users[0].id, 17);
    assert_eq!(snapshot.users[0].login, "alice");
    assert_eq!(
        snapshot.users[0].repository_permission,
        RepositoryPermission::Write
    );
    assert_eq!(snapshot.users[1].id, 18);
    assert_eq!(snapshot.users[1].login, "bob");
    assert_eq!(
        snapshot.users[1].repository_permission,
        RepositoryPermission::Write
    );
    assert_eq!(snapshot.teams.len(), 1);
    assert_eq!(snapshot.teams[0].id, 23);
    assert_eq!(snapshot.teams[0].organization_login, "acme");
    assert_eq!(snapshot.teams[0].slug, "reviewers");
    assert_eq!(snapshot.teams[0].members.len(), 1);
    assert_eq!(snapshot.teams[0].members[0].user_id, 18);
    assert!(snapshot.teams[0].members[0].inherited);
    fixture.assert_two_complete_team_observations();
    fixture.assert_no_snapshot_temporaries();
}

#[test]
fn second_observation_drift_preserves_the_old_output_and_leaves_no_temporary() {
    let fixture = Fixture::new();
    let old_output = b"preexisting ownership snapshot\n";
    fs::write(&fixture.output, old_output).unwrap();

    let output = fixture.run(true);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("changed between two consecutive observations"),
        "stderr:\n{stderr}"
    );
    assert_eq!(fs::read(&fixture.output).unwrap(), old_output);
    fixture.assert_two_complete_observations();
    fixture.assert_no_snapshot_temporaries();
}
