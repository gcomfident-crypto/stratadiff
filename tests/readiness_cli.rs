#![cfg(unix)]

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};

use stratadiff::readiness::{
    AuditVerdict, CollectionStatus, CollectionSurface, FindingRule, MergeReadinessAudit,
};
use tempfile::TempDir;

const REPOSITORY: &str = "acme/widgets";
const HOSTNAME: &str = "ghe.example";
const ACCEPT_HEADER: &str = "Accept: application/vnd.github+json";
const API_VERSION_HEADER: &str = "X-GitHub-Api-Version: 2022-11-28";
const ENDPOINTS: [&str; 5] = [
    "repos/acme/widgets",
    "repos/acme/widgets/git/ref/heads/main",
    "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1",
    "repos/acme/widgets/rules/branches/main",
    "repos/acme/widgets/branches?protected=true&per_page=100&page=1",
];

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
test "$6" = ghe.example
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
    if [ "$process_status" -ne 0 ]; then
        printf 'simulated gh failure\n' >&2
    fi
    exit "$process_status"
}

case "$endpoint" in
    "repos/acme/widgets")
        if [ "$GH_STUB_MODE" = ok_process_failure ]; then
            respond 200 OK '{"id":42,"full_name":"acme/widgets","html_url":"https://ghe.example/acme/widgets","default_branch":"main"}' 1
        fi
        respond 200 OK '{"id":42,"full_name":"acme/widgets","html_url":"https://ghe.example/acme/widgets","default_branch":"main"}' 0
        ;;
    "repos/acme/widgets/git/ref/heads/main")
        respond 200 OK '{"object":{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}' 0
        ;;
    "repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1")
        if [ "$GH_STUB_MODE" = forbidden_rulesets ]; then
            respond 403 Forbidden '{"message":"forbidden"}' 1
        fi
        respond 200 OK '[]' 0
        ;;
    "repos/acme/widgets/rules/branches/main")
        respond 200 OK '[]' 0
        ;;
    "repos/acme/widgets/branches?protected=true&per_page=100&page=1")
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
    Normal,
    ForbiddenRulesets,
    OkProcessFailure,
}

impl StubMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::ForbiddenRulesets => "forbidden_rulesets",
            Self::OkProcessFailure => "ok_process_failure",
        }
    }
}

struct Fixture {
    _directory: TempDir,
    command_path: OsString,
    log: PathBuf,
    mode: StubMode,
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
        }
    }

    fn command(&self, format: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
        command
            .args([
                "readiness-audit",
                "--repository",
                REPOSITORY,
                "--hostname",
                HOSTNAME,
                "--limit",
                "1",
                "--format",
                format,
            ])
            .env("PATH", &self.command_path)
            .env("GH_STUB_LOG", &self.log)
            .env("GH_STUB_MODE", self.mode.as_str());
        command
    }

    fn assert_calls(&self, endpoints: &[&str]) {
        let log = fs::read_to_string(&self.log).unwrap();
        let calls = log
            .lines()
            .map(|line| line.split_terminator('\t').collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), endpoints.len(), "calls: {calls:#?}");
        for (call, endpoint) in calls.iter().zip(endpoints) {
            assert_eq!(
                call,
                &vec![
                    "api",
                    "--include",
                    "--method",
                    "GET",
                    "--hostname",
                    HOSTNAME,
                    "--header",
                    ACCEPT_HEADER,
                    "--header",
                    API_VERSION_HEADER,
                    endpoint,
                ]
            );
        }
    }
}

fn parse_report(output: &Output) -> MergeReadinessAudit {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid report JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn json_stdout_remains_parseable_when_fail_on_findings_fires() {
    let fixture = Fixture::new(StubMode::Normal);
    let output = fixture
        .command("json")
        .arg("--fail-on-findings")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report = parse_report(&output);
    assert_eq!(report.scope.provider_url, "https://ghe.example");
    assert_eq!(report.scope.repository, REPOSITORY);
    assert_eq!(report.collection.status, CollectionStatus::Complete);
    assert_eq!(report.collection.api_calls, 5);
    assert_eq!(report.summary.verdict, AuditVerdict::ActionRequired);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.findings[0].rule,
        FindingRule::DefaultBranchUnprotected
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("merge-readiness audit found 1 actionable issue(s)")
    );
    fixture.assert_calls(&ENDPOINTS);
}

#[test]
fn markdown_stdout_contains_real_line_breaks() {
    let fixture = Fixture::new(StubMode::Normal);
    let output = fixture.command("markdown").output().unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let markdown = String::from_utf8(output.stdout).unwrap();
    assert!(
        markdown.starts_with(
            "# StrataDiff Merge Readiness Audit\n\n- Repository: [acme/widgets](<https://ghe.example/acme/widgets>)\n"
        ),
        "markdown:\n{markdown}"
    );
    assert!(
        markdown.contains("\n## Findings\n\n"),
        "markdown:\n{markdown}"
    );
    assert!(
        markdown.contains("\n## Unknowns\n\n"),
        "markdown:\n{markdown}"
    );
    assert!(markdown.ends_with('\n'));
    assert!(!markdown.contains("\\n"), "markdown:\n{markdown}");
    fixture.assert_calls(&ENDPOINTS);
}

#[test]
fn included_http_error_is_forwarded_despite_gh_exit_status() {
    let fixture = Fixture::new(StubMode::ForbiddenRulesets);
    let output = fixture.command("json").output().unwrap();

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report = parse_report(&output);
    assert_eq!(report.collection.status, CollectionStatus::Partial);
    assert_eq!(report.collection.api_calls, 5);
    assert_eq!(report.collection.gaps.len(), 1);
    assert_eq!(
        report.collection.gaps[0].surface,
        CollectionSurface::Rulesets
    );
    assert_eq!(
        report.collection.gaps[0].reason,
        "GitHub returned HTTP 403 for repos/acme/widgets/rulesets?includes_parents=true&per_page=100&page=1"
    );
    fixture.assert_calls(&ENDPOINTS);
}

#[test]
fn successful_http_response_does_not_hide_gh_process_failure() {
    let fixture = Fixture::new(StubMode::OkProcessFailure);
    let output = fixture.command("json").output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("gh api failed for repos/acme/widgets"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("simulated gh failure"), "stderr:\n{stderr}");
    fixture.assert_calls(&ENDPOINTS[..1]);
}
