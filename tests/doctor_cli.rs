#![cfg(unix)]

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use stratadiff::doctor::{DoctorRequirementStatus, DoctorVerdict, PullRequestDoctorReportV3};
use tempfile::TempDir;

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MERGE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";
const ACCEPT_HEADER: &str = "Accept: application/vnd.github+json";
const API_VERSION_HEADER: &str = "X-GitHub-Api-Version: 2022-11-28";

const GH_STUB: &str = r#"#!/bin/sh
set -eu

for argument in "$@"; do
    printf '%s\t' "$argument" >> "$GH_STUB_LOG"
done
printf '\n' >> "$GH_STUB_LOG"

hostname=${GH_STUB_HOSTNAME:-github.com}
provider_url=https://${hostname}

respond() {
    status=$1
    reason=$2
    body=$3
    process_status=$4
    printf 'HTTP/2.0 %s %s\r\n' "$status" "$reason"
    printf 'Content-Type: application/json; charset=utf-8\r\n'
    if [ "${5:-rest}" = rest ]; then
        printf 'X-GitHub-Api-Version-Selected: 2022-11-28\r\n'
    fi
    printf '\r\n'
    printf '%s\n' "$body"
    exit "$process_status"
}

if [ "$2" = graphql ]; then
    test "$#" -eq 13
    test "$3" = --include
    test "$4" = --hostname
    test "$5" = "$hostname"
    test "$6" = --raw-field
    case "$7" in query=*) ;; *) exit 64 ;; esac
    test "$8" = --raw-field
    test "$9" = name=widgets
    test "${10}" = --field
    test "${11}" = number=9
    test "${12}" = --raw-field
    test "${13}" = owner=acme
    graphql_process_status=0
    if [ "$GH_STUB_MODE" = graphql-process-failure ]; then
        graphql_process_status=1
    fi
    respond 200 OK "{\"data\":{\"repository\":{\"nameWithOwner\":\"acme/widgets\",\"url\":\"${provider_url}/acme/widgets\",\"pullRequest\":{\"number\":9,\"url\":\"${provider_url}/acme/widgets/pull/9\",\"state\":\"OPEN\",\"baseRefName\":\"main\",\"baseRefOid\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"headRefOid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"BLOCKED\",\"isMergeQueueEnabled\":false,\"isInMergeQueue\":false,\"potentialMergeCommit\":{\"oid\":\"cccccccccccccccccccccccccccccccccccccccc\"},\"mergeQueueEntry\":null}}}}" "$graphql_process_status" graphql
fi

test "$#" -eq 11
test "$1" = api
test "$2" = --include
test "$3" = --method
test "$4" = GET
test "$5" = --hostname
test "$6" = "$hostname"
test "$7" = --header
test "$8" = 'Accept: application/vnd.github+json'
test "$9" = --header
test "${10}" = 'X-GitHub-Api-Version: 2022-11-28'

endpoint=${11}

case "$endpoint" in
    "repos/acme/widgets/pulls/9")
        respond 200 OK "{\"number\":9,\"html_url\":\"${provider_url}/acme/widgets/pull/9\",\"state\":\"open\",\"merge_commit_sha\":\"cccccccccccccccccccccccccccccccccccccccc\",\"base\":{\"ref\":\"main\",\"sha\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},\"head\":{\"sha\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}}" 0
        ;;
    "repos/acme/widgets")
        respond 200 OK "{\"id\":42,\"full_name\":\"acme/widgets\",\"html_url\":\"${provider_url}/acme/widgets\",\"default_branch\":\"main\"}" 0
        ;;
    "repos/acme/widgets/rules/branches/main?per_page=100&page=1")
        respond 200 OK '[{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"lint","integration_id":15368}]},"ruleset_source_type":"Repository","ruleset_source":"acme/widgets","ruleset_id":7}]' 0
        ;;
    "repos/acme/widgets/branches/main")
        if [ "$GH_STUB_MODE" = partial ]; then
            respond 200 OK '{"name":"main","protected":true}' 0
        fi
        respond 200 OK '{"name":"main","protected":false}' 0
        ;;
    "repos/acme/widgets/branches/main/protection")
        test "$GH_STUB_MODE" = partial
        respond 404 NotFound '{"message":"Not Found"}' 1
        ;;
    "repos/acme/widgets/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/check-runs?filter=latest&per_page=100&page=1")
        if [ "$GH_STUB_MODE" = partial ]; then
            respond 200 OK "{\"total_count\":1,\"check_runs\":[{\"id\":101,\"html_url\":\"${provider_url}/acme/widgets/runs/101\",\"name\":\"lint\",\"head_sha\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"app\":{\"id\":15368,\"slug\":\"github-actions\"},\"status\":\"completed\",\"conclusion\":\"success\"}]}" 0
        fi
        respond 200 OK '{"total_count":0,"check_runs":[]}' 0
        ;;
    "repos/acme/widgets/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/statuses?per_page=100&page=1")
        respond 200 OK '[]' 0
        ;;
    "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/check-runs?filter=latest&per_page=100&page=1")
        if [ "$GH_STUB_MODE" = blocked ]; then
            respond 200 OK "{\"total_count\":1,\"check_runs\":[{\"id\":201,\"html_url\":\"${provider_url}/acme/widgets/runs/201\",\"name\":\"unrelated\",\"head_sha\":\"cccccccccccccccccccccccccccccccccccccccc\",\"app\":{\"id\":15368,\"slug\":\"github-actions\"},\"status\":\"completed\",\"conclusion\":\"success\"}]}" 0
        fi
        respond 200 OK '{"total_count":0,"check_runs":[]}' 0
        ;;
    "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/statuses?per_page=100&page=1")
        respond 200 OK '[]' 0
        ;;
    *)
        printf 'unexpected endpoint: %s\n' "$endpoint" >&2
        exit 64
        ;;
esac
"#;

#[derive(Clone, Copy)]
enum StubMode {
    Blocked,
    Partial,
    GraphqlProcessFailure,
}

impl StubMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Partial => "partial",
            Self::GraphqlProcessFailure => "graphql-process-failure",
        }
    }
}

struct Fixture {
    _directory: TempDir,
    command_path: OsString,
    log: PathBuf,
    mode: StubMode,
    hostname: &'static str,
}

impl Fixture {
    fn new(mode: StubMode) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let command_directory = directory.path().join("bin");
        fs::create_dir(&command_directory).unwrap();
        let gh = command_directory.join("gh");
        fs::write(&gh, GH_STUB).unwrap();
        let mut permissions = fs::metadata(&gh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh, permissions).unwrap();
        let mut paths = vec![command_directory];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
        Self {
            log: directory.path().join("gh-argv.log"),
            _directory: directory,
            command_path: env::join_paths(paths).unwrap(),
            mode,
            hostname: "github.com",
        }
    }

    fn with_hostname(mut self, hostname: &'static str) -> Self {
        self.hostname = hostname;
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
        command
            .env("PATH", &self.command_path)
            .env("GH_STUB_LOG", &self.log)
            .env("GH_STUB_MODE", self.mode.as_str())
            .env("GH_STUB_HOSTNAME", self.hostname);
        command
    }

    fn calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| {
                line.split_terminator('\t')
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

fn parse_report(output: &Output) -> PullRequestDoctorReportV3 {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid doctor JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_graphql_call(call: &[String], hostname: &str) {
    assert_eq!(call.len(), 13);
    assert_eq!(call[0], "api");
    assert_eq!(call[1], "graphql");
    assert_eq!(call[2], "--include");
    assert_eq!(call[3], "--hostname");
    assert_eq!(call[4], hostname);
    assert_eq!(call[5], "--raw-field");
    assert!(call[6].starts_with("query=query StrataDiffPullRequestCandidate"));
    assert_eq!(
        &call[7..],
        [
            "--raw-field",
            "name=widgets",
            "--field",
            "number=9",
            "--raw-field",
            "owner=acme"
        ]
    );
}

fn assert_call(call: &[String], endpoint: &str) {
    assert_eq!(
        call,
        [
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
        ]
    );
}

#[test]
fn url_target_writes_parseable_blocked_report_before_require_clear_fails() {
    let fixture = Fixture::new(StubMode::Blocked);
    let output = fixture
        .command()
        .args([
            "doctor",
            "https://github.com/acme/widgets/pull/9",
            "--format",
            "json",
            "--require-clear",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report = parse_report(&output);
    assert_eq!(report.repository, "acme/widgets");
    assert_eq!(report.target.number, 9);
    assert_eq!(report.target.base_sha, BASE_SHA);
    assert_eq!(report.target.head_sha, HEAD_SHA);
    assert_eq!(report.verdict, DoctorVerdict::ChecksBlocked);
    assert_eq!(report.requirements.len(), 1);
    assert_eq!(
        report.requirements[0].status,
        DoctorRequirementStatus::Missing
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("required checks are not proven clear: checks_blocked")
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 13);
    assert_call(&calls[0], "repos/acme/widgets/pulls/9");
    assert_call(&calls[1], "repos/acme/widgets");
    assert_graphql_call(&calls[2], "github.com");
    assert_call(
        &calls[3],
        "repos/acme/widgets/rules/branches/main?per_page=100&page=1",
    );
    assert_call(&calls[4], "repos/acme/widgets/branches/main");
    assert_call(
        &calls[5],
        &format!(
            "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
        ),
    );
    assert_call(
        &calls[6],
        &format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
    );
    assert_call(
        &calls[7],
        "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/check-runs?filter=latest&per_page=100&page=1",
    );
    assert_call(
        &calls[8],
        "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/statuses?per_page=100&page=1",
    );
    assert_call(&calls[9], "repos/acme/widgets/pulls/9");
    assert_graphql_call(&calls[10], "github.com");
    assert_call(
        &calls[11],
        "repos/acme/widgets/rules/branches/main?per_page=100&page=1",
    );
    assert_call(&calls[12], "repos/acme/widgets/branches/main");
}

#[test]
fn ghes_url_binds_every_request_and_report_url_to_the_selected_hostname() {
    let fixture = Fixture::new(StubMode::Blocked).with_hostname("ghe.example");
    let output = fixture
        .command()
        .args([
            "doctor",
            "https://ghe.example/acme/widgets/pull/9",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_report(&output);
    assert_eq!(report.provider_url, "https://ghe.example");
    assert_eq!(report.target.url, "https://ghe.example/acme/widgets/pull/9");
    let calls = fixture.calls();
    assert_eq!(calls.len(), 13);
    for call in &calls {
        if call.get(1).is_some_and(|value| value == "graphql") {
            assert_graphql_call(call, "ghe.example");
        } else {
            assert_eq!(call[5], "ghe.example");
        }
    }
}

#[test]
fn graphql_process_failure_rejects_a_parseable_http_success() {
    let fixture = Fixture::new(StubMode::GraphqlProcessFailure);
    let output = fixture
        .command()
        .args(["doctor", "9", "-R", "acme/widgets", "--format", "json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("gh api graphql failed with exit status: 1")
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 3);
    assert_graphql_call(&calls[2], "github.com");
}

#[test]
fn partial_policy_visibility_is_inconclusive_and_report_is_written_before_failure() {
    let fixture = Fixture::new(StubMode::Partial);
    let output_path = fixture._directory.path().join("doctor.json");
    let output = fixture
        .command()
        .args([
            "doctor",
            "9",
            "--repository",
            "acme/widgets",
            "--hostname",
            "github.com",
            "--format",
            "json",
            "--output",
        ])
        .arg(&output_path)
        .arg("--require-clear")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let report: PullRequestDoctorReportV3 =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(report.summary.satisfied, 1);
    assert_eq!(report.collection.gaps.len(), 1);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("required checks are not proven clear: inconclusive")
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 15);
    assert_call(&calls[5], "repos/acme/widgets/branches/main/protection");
}

#[test]
fn selector_mismatches_fail_before_the_first_provider_request() {
    let fixture = Fixture::new(StubMode::Blocked);
    let cases = [
        vec!["doctor", "9"],
        vec!["doctor", "9", "--repository", "../widgets"],
        vec![
            "doctor",
            "9",
            "--repository",
            "acme/widgets",
            "--hostname",
            "ghe\\evil",
        ],
        vec![
            "doctor",
            "https://github.com/acme/widgets/pull/9",
            "--repository",
            "other/widgets",
        ],
        vec![
            "doctor",
            "https://github.com/acme/widgets/pull/9",
            "--hostname",
            "ghe.example",
        ],
        vec!["doctor", "https://github.com/acme/widgets/pull/09"],
        vec![
            "doctor",
            "https://github.com/acme/widgets/pull/9?diff=split",
        ],
    ];
    for arguments in cases {
        let output = fixture.command().args(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(!Path::new(&fixture.log).exists());
}

#[test]
fn markdown_names_the_exact_scope_and_does_not_claim_mergeability() {
    let fixture = Fixture::new(StubMode::Blocked);
    let output = fixture
        .command()
        .args(["doctor", "9", "-R", "acme/widgets", "--format", "markdown"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let markdown = String::from_utf8(output.stdout).unwrap();
    assert!(markdown.starts_with("# StrataDiff PR Required-Check Doctor\n\n"));
    assert!(markdown.contains(MERGE_SHA));
    assert!(markdown.contains("Evaluation target: <code>test&#95;merge</code>"));
    assert!(markdown.contains("required-check readiness only for the declared"));
    assert!(markdown.contains("does not evaluate mergeability, reviews, conflicts"));
    assert!(!markdown.to_ascii_lowercase().contains("safe to merge"));
}
