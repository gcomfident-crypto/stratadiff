#![cfg(unix)]

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use stratadiff::doctor::{DoctorRequirementStatus, DoctorVerdict, PullRequestDoctorReport};
use tempfile::TempDir;

const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
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

respond() {
    status=$1
    reason=$2
    body=$3
    process_status=$4
    printf 'HTTP/2.0 %s %s\r\n' "$status" "$reason"
    printf 'Content-Type: application/json; charset=utf-8\r\n'
    printf 'X-GitHub-Api-Version-Selected: 2022-11-28\r\n'
    printf '\r\n'
    printf '%s\n' "$body"
    exit "$process_status"
}

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
}

impl StubMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Partial => "partial",
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

fn parse_report(output: &Output) -> PullRequestDoctorReport {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid doctor JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
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
    assert_eq!(calls.len(), 9);
    assert_call(&calls[0], "repos/acme/widgets/pulls/9");
    assert_call(&calls[1], "repos/acme/widgets");
    assert_call(
        &calls[2],
        "repos/acme/widgets/rules/branches/main?per_page=100&page=1",
    );
    assert_call(&calls[3], "repos/acme/widgets/branches/main");
    assert_call(
        &calls[4],
        &format!(
            "repos/acme/widgets/commits/{HEAD_SHA}/check-runs?filter=latest&per_page=100&page=1"
        ),
    );
    assert_call(
        &calls[5],
        &format!("repos/acme/widgets/commits/{HEAD_SHA}/statuses?per_page=100&page=1"),
    );
    assert_call(
        &calls[6],
        "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/check-runs?filter=latest&per_page=100&page=1",
    );
    assert_call(
        &calls[7],
        "repos/acme/widgets/commits/cccccccccccccccccccccccccccccccccccccccc/statuses?per_page=100&page=1",
    );
    assert_call(&calls[8], "repos/acme/widgets/pulls/9");
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
    assert_eq!(calls.len(), 9);
    assert!(calls.iter().all(|call| call[5] == "ghe.example"));
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
    let report: PullRequestDoctorReport =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(report.verdict, DoctorVerdict::Inconclusive);
    assert_eq!(report.summary.satisfied, 1);
    assert_eq!(report.collection.gaps.len(), 1);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("required checks are not proven clear: inconclusive")
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 10);
    assert_call(&calls[4], "repos/acme/widgets/branches/main/protection");
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
    assert!(markdown.contains(HEAD_SHA));
    assert!(markdown.contains("required-check readiness for this exact head SHA"));
    assert!(markdown.contains("test-merge commit had no status signals"));
    assert!(markdown.contains("does not evaluate reviews, conflicts, deployment policy"));
    assert!(!markdown.to_ascii_lowercase().contains("safe to merge"));
}
