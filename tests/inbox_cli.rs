use std::{fs, path::Path, process::Command};

use serde_json::Value;
use stratadiff::inbox_event::{InboxEventEnvelope, InboxEventTrigger};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn write_gh_stub(path: &Path) {
    let script = r###"#!/usr/bin/env bash
set -euo pipefail

joined=" $* "
host=${STRATADIFF_TEST_HOST:-github.com}
request_login=reviewer
request_id=U_reviewer
request_type=User
if [[ "${STRATADIFF_TEST_OTHER_REVIEW_REQUEST:-}" == 1 ]]; then
  request_login=other
  request_id=U_other
fi
if [[ "${STRATADIFF_TEST_BOT_REVIEW_REQUEST:-}" == 1 ]]; then
  request_login='dependabot[bot]'
  request_id=B_dependabot
  request_type=Bot
fi
request_has_next=false
request_total=1
if [[ "${STRATADIFF_TEST_INCOMPLETE_REVIEW_REQUESTS:-}" == 1 ]]; then
  request_has_next=true
  request_total=2
fi
if [[ "$joined" == *" user "* || "$joined" == *" users/reviewer "* ]]; then
  printf '%s\n' '{"login":"reviewer","id":42,"node_id":"U_reviewer","type":"User"}'
  exit 0
fi

if [[ "$joined" != *" graphql "* ]]; then
  printf 'unexpected gh invocation: %s\n' "$*" >&2
  exit 2
fi

if [[ "$joined" == *"StrataDiffReviewInboxRepository"* ]]; then
  repository='{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://'"${host}"'/acme/widget"}'
  if [[ "${STRATADIFF_TEST_MISSING_REPO:-}" == 1 ]]; then
    repository=null
  fi
  cat <<JSON
{"data":{"viewer":{"id":"U_reviewer","login":"reviewer"},"repository":${repository},"rateLimit":{"cost":1,"remaining":4999,"resetAt":"2026-09-06T01:00:00Z"}}}
JSON
  exit 0
fi

if [[ "$joined" == *"StrataDiffReviewInboxSearch"* ]]; then
  issue_count=3
  has_next=false
  if [[ "${STRATADIFF_TEST_TRUNCATED:-}" == 1 ]]; then
    issue_count=4
    has_next=true
  fi
  head17='"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"'
  base17='"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"'
  all_reviews17=2
  if [[ "${STRATADIFF_TEST_MISSING_HEAD:-}" == 1 ]]; then
    head17=null
  fi
  if [[ "${STRATADIFF_TEST_REVIEW_LIMIT:-}" == 1 ]]; then
    all_reviews17=10001
  fi
  if [[ "${STRATADIFF_TEST_MISSING_BASE:-}" == 1 ]]; then
    base17=null
  fi
  cat <<JSON
{"data":{"viewer":{"id":"U_reviewer","login":"reviewer"},"search":{"issueCount":${issue_count},"pageInfo":{"hasNextPage":${has_next},"endCursor":"cursor-3"},"nodes":[
{"id":"PR_actionable","number":17,"state":"OPEN","url":"https://${host}/acme/widget/pull/17","isDraft":false,"updatedAt":"2026-09-06T00:00:03Z","baseRefOid":${base17},"headRefOid":${head17},"repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget"},"allReviews":{"totalCount":${all_reviews17}},"reviewRequests":{"totalCount":${request_total},"pageInfo":{"hasNextPage":${request_has_next},"endCursor":"request-17"},"nodes":[{"requestedReviewer":{"__typename":"${request_type}","id":"${request_id}","login":"${request_login}"}}]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-17"},"nodes":[{"id":"PRR_17","fullDatabaseId":"1701","state":"APPROVED","submittedAt":"2026-09-05T00:00:00Z","url":"https://${host}/acme/widget/pull/17#pullrequestreview-1701","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]}},
{"id":"PR_current","number":18,"state":"OPEN","url":"https://${host}/acme/widget/pull/18","isDraft":false,"updatedAt":"2026-09-06T00:00:02Z","baseRefOid":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","headRefOid":"cccccccccccccccccccccccccccccccccccccccc","repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget"},"allReviews":{"totalCount":1},"reviewRequests":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-18"},"nodes":[{"id":"PRR_18","fullDatabaseId":"1801","state":"CHANGES_REQUESTED","submittedAt":"2026-09-05T00:00:01Z","url":"https://${host}/acme/widget/pull/18#pullrequestreview-1801","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"cccccccccccccccccccccccccccccccccccccccc"}}]}},
{"id":"PR_comment","number":19,"state":"OPEN","url":"https://${host}/acme/widget/pull/19","isDraft":false,"updatedAt":"2026-09-06T00:00:01Z","baseRefOid":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","headRefOid":"dddddddddddddddddddddddddddddddddddddddd","repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget"},"allReviews":{"totalCount":1},"reviewRequests":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-19"},"nodes":[{"id":"PRR_19","fullDatabaseId":null,"state":"COMMENTED","submittedAt":"2026-09-05T00:00:02Z","url":"https://${host}/acme/widget/pull/19#pullrequestreview-1901","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"dddddddddddddddddddddddddddddddddddddddd"}}]}}
]},"rateLimit":{"cost":1,"remaining":4999,"resetAt":"2026-09-06T01:00:00Z"}}}
JSON
  exit 0
fi

if [[ "$joined" == *"StrataDiffReviewInboxRevalidate"* ]]; then
  if [[ "$joined" == *" number=17 "* ]]; then
    head='"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"'
    base='"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"'
    all_reviews=2
    viewer=U_reviewer
    if [[ "${STRATADIFF_TEST_DRIFT:-}" == 1 ]]; then
      head='"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"'
    fi
    if [[ "${STRATADIFF_TEST_ACTOR_DRIFT:-}" == 1 ]]; then
      viewer=U_other
    fi
    if [[ "${STRATADIFF_TEST_MISSING_HEAD:-}" == 1 ]]; then
      head=null
    fi
    if [[ "${STRATADIFF_TEST_REVIEW_LIMIT:-}" == 1 ]]; then
      all_reviews=10001
    fi
    if [[ "${STRATADIFF_TEST_MISSING_BASE:-}" == 1 ]]; then
      base=null
    fi
    cat <<JSON
{"data":{"viewer":{"id":"${viewer}","login":"reviewer"},"repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget","pullRequest":{"id":"PR_actionable","number":17,"state":"OPEN","url":"https://${host}/acme/widget/pull/17","isDraft":false,"updatedAt":"2026-09-06T00:00:03Z","baseRefOid":${base},"headRefOid":${head},"allReviews":{"totalCount":${all_reviews}},"reviewRequests":{"totalCount":${request_total},"pageInfo":{"hasNextPage":${request_has_next},"endCursor":"request-17"},"nodes":[{"requestedReviewer":{"__typename":"${request_type}","id":"${request_id}","login":"${request_login}"}}]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-17"},"nodes":[{"id":"PRR_17","fullDatabaseId":"1701","state":"APPROVED","submittedAt":"2026-09-05T00:00:00Z","url":"https://${host}/acme/widget/pull/17#pullrequestreview-1701","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]}}},"rateLimit":{"cost":1,"remaining":4998,"resetAt":"2026-09-06T01:00:00Z"}}}
JSON
    exit 0
  fi
  if [[ "$joined" == *" number=18 "* ]]; then
    if [[ -n "${STRATADIFF_TEST_REMOVE_OUTPUT_PARENT:-}" ]]; then
      rmdir -- "${STRATADIFF_TEST_REMOVE_OUTPUT_PARENT}"
    fi
    cat <<JSON
{"data":{"viewer":{"id":"U_reviewer","login":"reviewer"},"repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget","pullRequest":{"id":"PR_current","number":18,"state":"OPEN","url":"https://${host}/acme/widget/pull/18","isDraft":false,"updatedAt":"2026-09-06T00:00:02Z","baseRefOid":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","headRefOid":"cccccccccccccccccccccccccccccccccccccccc","allReviews":{"totalCount":1},"reviewRequests":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-18"},"nodes":[{"id":"PRR_18","fullDatabaseId":"1801","state":"CHANGES_REQUESTED","submittedAt":"2026-09-05T00:00:01Z","url":"https://${host}/acme/widget/pull/18#pullrequestreview-1801","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"cccccccccccccccccccccccccccccccccccccccc"}}]}}},"rateLimit":{"cost":1,"remaining":4997,"resetAt":"2026-09-06T01:00:00Z"}}}
JSON
    exit 0
  fi
  if [[ "$joined" == *" number=19 "* ]]; then
    cat <<JSON
{"data":{"viewer":{"id":"U_reviewer","login":"reviewer"},"repository":{"id":"R_widget","nameWithOwner":"acme/widget","url":"https://${host}/acme/widget","pullRequest":{"id":"PR_comment","number":19,"state":"OPEN","url":"https://${host}/acme/widget/pull/19","isDraft":false,"updatedAt":"2026-09-06T00:00:01Z","baseRefOid":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","headRefOid":"dddddddddddddddddddddddddddddddddddddddd","allReviews":{"totalCount":1},"reviewRequests":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},"reviews":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"review-19"},"nodes":[{"id":"PRR_19","fullDatabaseId":null,"state":"COMMENTED","submittedAt":"2026-09-05T00:00:02Z","url":"https://${host}/acme/widget/pull/19#pullrequestreview-1901","authorAssociation":"MEMBER","author":{"__typename":"User","login":"reviewer","id":"U_reviewer"},"commit":{"oid":"dddddddddddddddddddddddddddddddddddddddd"}}]}}},"rateLimit":{"cost":1,"remaining":4996,"resetAt":"2026-09-06T01:00:00Z"}}}
JSON
    exit 0
  fi
fi

printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 2
"###;
    fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

fn inbox_command_with_format(format: &str) -> (tempfile::TempDir, Command) {
    let directory = tempfile::tempdir().unwrap();
    let gh = directory.path().join("gh");
    write_gh_stub(&gh);
    let inherited_path = std::env::var_os("PATH").unwrap();
    let mut paths = vec![directory.path().to_path_buf()];
    paths.extend(std::env::split_paths(&inherited_path));
    let path = std::env::join_paths(paths).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
    command
        .args([
            "inbox",
            "--reviewer",
            "reviewer",
            "--limit",
            "3",
            "--format",
            format,
        ])
        .env("PATH", path);
    (directory, command)
}

fn inbox_command() -> (tempfile::TempDir, Command) {
    inbox_command_with_format("json")
}

#[test]
fn global_inbox_emits_only_exact_revalidated_completed_review_drift() {
    let (_directory, mut command) = inbox_command();
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../schema/review-inbox-v3.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let errors = validator
        .iter_errors(&inbox)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(inbox["schema"], "stratadiff-review-inbox-v3");
    assert_eq!(inbox["collection"]["search_candidates"], 3);
    assert_eq!(inbox["collection"]["revalidated_review_prs"], 3);
    assert_eq!(inbox["summary"]["completed_review_prs"], 2);
    assert_eq!(inbox["summary"]["resume_available_prs"], 1);
    assert_eq!(inbox["summary"]["up_to_date_prs"], 0);
    assert_eq!(inbox["summary"]["no_completed_review_prs"], 1);
    assert_eq!(inbox["summary"]["unobservable_review_prs"], 1);
    assert_eq!(inbox["unobservable"][0]["number"], 18);
    assert_eq!(
        inbox["unobservable"][0]["reason"],
        "checkpoint_base_oid_unavailable"
    );
    assert_eq!(
        inbox["scope"]["authenticated_actor"]["node_id"],
        "U_reviewer"
    );
    assert_eq!(inbox["privacy"]["source_collected"], false);
    assert_eq!(inbox["privacy"]["pr_text_collected"], false);
    assert_eq!(inbox["privacy"]["review_text_collected"], false);
    assert_eq!(
        inbox["privacy"]["authenticated_actor_identity_persisted"],
        true
    );
    assert_eq!(inbox["actionable"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["actionable"][0]["repository"], "acme/widget");
    assert_eq!(inbox["actionable"][0]["number"], 17);
    assert_eq!(
        inbox["actionable"][0]["event_id"].as_str().unwrap().len(),
        64
    );
    assert_eq!(inbox["actionable"][0]["checkpoint_base_oid"], Value::Null);
    assert_eq!(
        inbox["actionable"][0]["current_base_oid"],
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );
    assert_eq!(inbox["actionable"][0]["review_request_active"], true);
    assert_eq!(
        inbox["actionable"][0]["triggers"],
        serde_json::json!(["head_changed", "review_re_requested"])
    );
    let inbox_event = inbox["actionable"][0]["inbox_event"].as_str().unwrap();
    let envelope = InboxEventEnvelope::from_token(inbox_event).unwrap();
    assert_eq!(envelope.event_id, inbox["actionable"][0]["event_id"]);
    assert_eq!(envelope.provider_host, "github.com");
    assert_eq!(envelope.repository, "acme/widget");
    assert_eq!(envelope.repository_node_id, "R_widget");
    assert_eq!(envelope.pull_request_number, 17);
    assert_eq!(envelope.pull_request_node_id, "PR_actionable");
    assert_eq!(envelope.reviewer_login, "reviewer");
    assert_eq!(envelope.reviewer_node_id, "U_reviewer");
    assert_eq!(envelope.review_database_id, 1701);
    assert_eq!(envelope.review_state, "approved");
    assert_eq!(envelope.review_node_id, "PRR_17");
    assert_eq!(
        envelope.checkpoint_oid,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(envelope.checkpoint_base_oid, None);
    assert_eq!(
        envelope.current_base_oid.as_deref(),
        Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    );
    assert_eq!(
        envelope.head_oid,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert!(envelope.review_request_active);
    assert_eq!(
        envelope.triggers,
        [
            InboxEventTrigger::HeadChanged,
            InboxEventTrigger::ReviewReRequested,
        ]
    );
    assert_eq!(
        inbox["actionable"][0]["resume_argv"],
        serde_json::json!([
            "stratadiff",
            "resume",
            "https://github.com/acme/widget/pull/17",
            "--reviewer",
            "reviewer",
            "--inbox-event",
            inbox_event
        ])
    );

    let mut missing_action = inbox.clone();
    missing_action["actionable"] = serde_json::json!([]);
    assert!(!validator.is_valid(&missing_action));

    let mut contradictory_count = inbox.clone();
    contradictory_count["summary"]["resume_available_prs"] = serde_json::json!(0);
    assert!(!validator.is_valid(&contradictory_count));

    let mut false_clean = inbox.clone();
    false_clean["summary"]["status"] = serde_json::json!("no_eligible_reviews");
    assert!(!validator.is_valid(&false_clean));

    let mut mislabeled_truncation = inbox.clone();
    mislabeled_truncation["collection"]["truncated"] = serde_json::json!(true);
    assert!(!validator.is_valid(&mislabeled_truncation));
}

#[test]
fn global_inbox_refuses_a_head_that_moves_during_revalidation() {
    let (_directory, mut command) = inbox_command();
    let output = command.env("STRATADIFF_TEST_DRIFT", "1").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("candidate_changed_during_revalidation"));
    assert!(output.stdout.is_empty());
}

#[test]
fn global_inbox_refuses_an_authenticated_actor_that_changes_mid_scan() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_ACTOR_DRIFT", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("authenticated actor changed"));
    assert!(output.stdout.is_empty());
}

#[test]
fn repository_scope_fails_closed_when_the_repository_is_not_accessible() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .args(["-R", "acme/widget"])
        .env("STRATADIFF_TEST_MISSING_REPO", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist or is not accessible"));
    assert!(output.stdout.is_empty());
}

#[test]
fn enterprise_repository_selector_emits_an_executable_host_bound_resume() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .args(["-R", "ghe.example/acme/widget"])
        .env("STRATADIFF_TEST_HOST", "ghe.example")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox["scope"]["provider_url"], "https://ghe.example");
    assert_eq!(inbox["scope"]["repository"], "acme/widget");
    let argv = inbox["actionable"][0]["resume_argv"].as_array().unwrap();
    assert_eq!(argv.len(), 9);
    assert_eq!(argv[0], "stratadiff");
    assert_eq!(argv[1], "resume");
    assert_eq!(argv[2], "https://ghe.example/acme/widget/pull/17");
    assert_eq!(argv[3], "--reviewer");
    assert_eq!(argv[4], "reviewer");
    assert_eq!(argv[5], "--inbox-event");
    assert_eq!(argv[6], inbox["actionable"][0]["inbox_event"]);
    assert_eq!(argv[7], "-R");
    assert_eq!(argv[8], "ghe.example/acme/widget");
}

#[test]
fn truncated_global_scan_is_partial_and_never_reports_a_clean_queue() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_TRUNCATED", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../schema/review-inbox-v3.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    assert!(validator.is_valid(&inbox));
    assert_eq!(inbox["collection"]["status"], "partial");
    assert_eq!(inbox["collection"]["truncated"], true);
    assert_eq!(inbox["summary"]["status"], "partial");

    let mut false_complete = inbox.clone();
    false_complete["collection"]["status"] = serde_json::json!("complete");
    assert!(!validator.is_valid(&false_complete));
}

#[test]
fn missing_head_markdown_reports_insufficient_evidence_instead_of_clean() {
    let (_directory, mut command) = inbox_command_with_format("markdown");
    let output = command
        .env("STRATADIFF_TEST_MISSING_HEAD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No safe Resume action was emitted"));
    assert!(stdout.contains("could not be compared safely"));
    assert!(!stdout.contains("No completed review checkpoint currently differs"));
}

#[test]
fn review_limit_markdown_reports_insufficient_evidence_instead_of_clean() {
    let (_directory, mut command) = inbox_command_with_format("markdown");
    let output = command
        .env("STRATADIFF_TEST_REVIEW_LIMIT", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No safe Resume action was emitted"));
    assert!(stdout.contains("could not be compared safely"));
    assert!(!stdout.contains("No completed review checkpoint currently differs"));
}

#[test]
fn missing_current_base_never_emits_an_unexecutable_resume() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_MISSING_BASE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox["summary"]["status"], "insufficient_evidence");
    assert_eq!(inbox["summary"]["resume_available_prs"], 0);
    assert_eq!(inbox["summary"]["unobservable_review_prs"], 2);
    assert!(inbox["actionable"].as_array().unwrap().is_empty());
    assert_eq!(
        inbox["unobservable"][0]["reason"],
        "current_base_oid_unavailable"
    );
}

#[test]
fn active_review_request_is_bound_to_the_exact_reviewer() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_OTHER_REVIEW_REQUEST", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox["actionable"][0]["review_request_active"], false);
    assert_eq!(
        inbox["actionable"][0]["triggers"],
        serde_json::json!(["head_changed"])
    );
    let token = inbox["actionable"][0]["inbox_event"].as_str().unwrap();
    let event = InboxEventEnvelope::from_token(token).unwrap();
    assert!(!event.review_request_active);
    assert_eq!(event.triggers, [InboxEventTrigger::HeadChanged]);
}

#[test]
fn unrelated_bracketed_bot_request_keeps_resume_action_executable() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_BOT_REVIEW_REQUEST", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox["summary"]["resume_available_prs"], 1);
    assert_eq!(inbox["actionable"][0]["review_request_active"], false);
    assert_eq!(
        inbox["actionable"][0]["triggers"],
        serde_json::json!(["head_changed"])
    );
    let resume_argv = inbox["actionable"][0]["resume_argv"].as_array().unwrap();
    assert_eq!(resume_argv[0], "stratadiff");
    assert_eq!(resume_argv[1], "resume");
    let token = inbox["actionable"][0]["inbox_event"].as_str().unwrap();
    assert_eq!(resume_argv[6], token);
    let event = InboxEventEnvelope::from_token(token).unwrap();
    assert!(!event.review_request_active);
    assert_eq!(event.triggers, [InboxEventTrigger::HeadChanged]);
}

#[test]
fn incomplete_review_request_evidence_fails_closed() {
    let (_directory, mut command) = inbox_command();
    let output = command
        .env("STRATADIFF_TEST_INCOMPLETE_REVIEW_REQUESTS", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("review_request_pagination_incomplete")
    );
}

#[test]
fn terminal_unsafe_value_log_paths_fail_before_output() {
    let (json_directory, mut json_command) = inbox_command();
    let format_control = '\u{202e}';
    let json_log = json_directory
        .path()
        .join(format!("value-{format_control}-funnel.jsonl"));
    let json_output = json_command
        .arg("--value-log")
        .arg(&json_log)
        .output()
        .unwrap();
    assert!(!json_output.status.success());
    assert!(json_output.stdout.is_empty());
    let json_error = String::from_utf8(json_output.stderr).unwrap();
    assert!(!json_error.contains(format_control));
    assert!(json_error.contains("terminal-unsafe"));

    let (markdown_directory, mut markdown_command) = inbox_command_with_format("markdown");
    let markdown_log = markdown_directory
        .path()
        .join(format!("value-{format_control}-funnel.jsonl"));
    let markdown_output = markdown_command
        .arg("--value-log")
        .arg(&markdown_log)
        .output()
        .unwrap();
    assert!(!markdown_output.status.success());
    assert!(markdown_output.stdout.is_empty());
    let markdown_error = String::from_utf8(markdown_output.stderr).unwrap();
    assert!(!markdown_error.contains(format_control));
    assert!(markdown_error.contains("terminal-unsafe"));
}

#[test]
fn inbox_output_success_message_is_terminal_safe() {
    let (directory, mut command) = inbox_command();
    let output_path = directory.path().join("inbox-\u{1b}[31m-\u{202e}.json");
    let output = command.arg("--output").arg(&output_path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains('\u{202e}'));
    assert!(stderr.contains("\\u001b"));
    assert!(stderr.contains("\\u202e"));
    let inbox: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(inbox["schema"], "stratadiff-review-inbox-v3");
}

#[test]
fn value_report_rejects_terminal_unsafe_output_paths() {
    let (directory, mut inbox_command) = inbox_command();
    let log = directory.path().join("value-funnel.jsonl");
    let inbox_output = inbox_command.arg("--value-log").arg(&log).output().unwrap();
    assert!(
        inbox_output.status.success(),
        "{}",
        String::from_utf8_lossy(&inbox_output.stderr)
    );

    let report_path = directory.path().join("report-\u{202e}.json");
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .args(["value-report", log.to_str().unwrap(), "--format", "json"])
        .arg("--output")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains('\u{202e}'));
    assert!(stderr.contains("terminal-unsafe"));
    assert!(!report_path.exists());
}

#[test]
fn opted_in_value_log_is_private_verifiable_and_aggregates_without_identity() {
    let (directory, mut command) = inbox_command();
    let log = directory.path().join("value-funnel.jsonl");
    let output = command.arg("--value-log").arg(&log).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    let event_id = inbox["actionable"][0]["event_id"].as_str().unwrap();
    let log_text = fs::read_to_string(&log).unwrap();
    assert!(!log_text.contains("acme/widget"));
    assert!(!log_text.contains("reviewer"));
    let lines = log_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    let schema: Value =
        serde_json::from_str(include_str!("../schema/value-funnel-event-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let events = lines
        .iter()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for event in &events {
        assert!(validator.is_valid(event));
    }
    assert_eq!(events[0]["kind"], "baseline");
    assert_eq!(events[1]["kind"], "gap_discovery");
    assert_eq!(events[2]["kind"], "inbox_delivery");
    assert_eq!(
        events[0]["payload"]["scan_id"],
        events[1]["payload"]["scan_id"]
    );
    assert_eq!(
        events[0]["payload"]["scan_id"],
        events[2]["payload"]["scan_id"]
    );
    let argv = inbox["actionable"][0]["resume_argv"].as_array().unwrap();
    assert_eq!(argv[5], "--inbox-event");
    assert_eq!(argv[6], inbox["actionable"][0]["inbox_event"]);
    assert_eq!(argv[7], "--value-log");
    assert_eq!(argv[8], log.to_str().unwrap());
    assert_eq!(argv[9], "--transition-id");
    let transition_id = argv[10].as_str().unwrap();
    assert_ne!(transition_id, event_id);
    assert!(log_text.contains(transition_id));

    #[cfg(unix)]
    assert_eq!(fs::metadata(&log).unwrap().permissions().mode() & 0o077, 0);

    let report = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .args(["value-report", log.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    let report_schema: Value =
        serde_json::from_str(include_str!("../schema/value-funnel-report-v1.schema.json")).unwrap();
    assert!(
        jsonschema::draft202012::new(&report_schema)
            .unwrap()
            .is_valid(&report)
    );
    assert_eq!(report["summary"]["scans"], 1);
    assert_eq!(report["summary"]["completed_review_checkpoints"], 2);
    assert_eq!(report["summary"]["covered_transitions"], 0);
    assert_eq!(report["summary"]["unique_gap_transitions"], 1);
    assert_eq!(report["summary"]["delivery_confirmed_scans"], 1);
    assert_eq!(report["summary"]["delivery_unconfirmed_scans"], 0);
    assert_eq!(report["summary"]["delivered_gap_discoveries"], 1);
    assert_eq!(report["summary"]["unique_delivered_gap_transitions"], 1);
    assert_eq!(report["summary"]["unique_resumed_transitions"], 0);
    assert_eq!(report["summary"]["resume_attempts"], 0);
    assert_eq!(
        report["claim_boundary"]["clean_install_success_supported"],
        false
    );
    assert_eq!(report["privacy"]["automatic_upload"], false);
    assert!(
        !String::from_utf8(report.to_string().into_bytes())
            .unwrap()
            .contains(transition_id)
    );
}

#[test]
fn value_transition_identity_changes_with_bound_review_request_state() {
    let (requested_directory, mut requested_command) = inbox_command();
    let requested_log = requested_directory.path().join("requested.jsonl");
    let requested_output = requested_command
        .arg("--value-log")
        .arg(&requested_log)
        .output()
        .unwrap();
    assert!(
        requested_output.status.success(),
        "{}",
        String::from_utf8_lossy(&requested_output.stderr)
    );
    let requested: Value = serde_json::from_slice(&requested_output.stdout).unwrap();

    let (plain_directory, mut plain_command) = inbox_command();
    let plain_log = plain_directory.path().join("plain.jsonl");
    let plain_output = plain_command
        .env("STRATADIFF_TEST_OTHER_REVIEW_REQUEST", "1")
        .arg("--value-log")
        .arg(&plain_log)
        .output()
        .unwrap();
    assert!(
        plain_output.status.success(),
        "{}",
        String::from_utf8_lossy(&plain_output.stderr)
    );
    let plain: Value = serde_json::from_slice(&plain_output.stdout).unwrap();

    let requested_argv = requested["actionable"][0]["resume_argv"]
        .as_array()
        .unwrap();
    let plain_argv = plain["actionable"][0]["resume_argv"].as_array().unwrap();
    assert_ne!(requested_argv[10], plain_argv[10]);
    assert_ne!(
        requested["actionable"][0]["event_id"],
        plain["actionable"][0]["event_id"]
    );
}

#[test]
fn failed_inbox_output_keeps_discovery_but_never_confirms_delivery() {
    let (directory, mut command) = inbox_command();
    let log = directory.path().join("value-funnel.jsonl");
    let output_parent = directory.path().join("removed-output-parent");
    fs::create_dir(&output_parent).unwrap();
    let output_path = output_parent.join("inbox.json");

    let output = command
        .arg("--value-log")
        .arg(&log)
        .arg("--output")
        .arg(&output_path)
        .env("STRATADIFF_TEST_REMOVE_OUTPUT_PARENT", &output_parent)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let events = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["kind"], "baseline");
    assert_eq!(events[1]["kind"], "gap_discovery");
    assert!(events.iter().all(|event| event["kind"] != "inbox_delivery"));

    let report = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .args(["value-report", log.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["summary"]["gap_discoveries"], 1);
    assert_eq!(report["summary"]["delivery_confirmed_scans"], 0);
    assert_eq!(report["summary"]["delivery_unconfirmed_scans"], 1);
    assert_eq!(report["summary"]["delivered_gap_discoveries"], 0);
    assert_eq!(
        report["conversion"]["delivered_gap_to_resume"]["status"],
        "undefined"
    );
}

#[test]
fn value_log_append_failure_does_not_publish_an_unusable_inbox() {
    let (directory, mut stdout_command) = inbox_command();
    let log = directory.path().join("corrupt-value-funnel.jsonl");
    fs::write(&log, b"not a value-funnel event\n").unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&log).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&log, permissions).unwrap();
    }

    let stdout_result = stdout_command
        .arg("--value-log")
        .arg(&log)
        .output()
        .unwrap();
    assert!(!stdout_result.status.success());
    assert!(stdout_result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&stdout_result.stderr).contains("failed to decode value event 1")
    );

    let (_second_directory, mut file_command) = inbox_command();
    let report = directory.path().join("review-inbox.json");
    let previous = b"previous report\n";
    fs::write(&report, previous).unwrap();
    let file_result = file_command
        .arg("--value-log")
        .arg(&log)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert!(!file_result.status.success());
    assert!(file_result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&file_result.stderr).contains("failed to decode value event 1")
    );
    assert_eq!(fs::read(&report).unwrap(), previous);
}

#[test]
fn value_log_and_command_output_must_not_alias() {
    let (directory, mut command) = inbox_command();
    let shared = directory.path().join("shared.jsonl");
    let output = command
        .arg("--value-log")
        .arg(&shared)
        .arg("--output")
        .arg(&shared)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("value log and command output must use different files")
    );
    assert!(!shared.exists());
}

#[test]
fn relative_inbox_and_value_report_outputs_are_supported() {
    let (directory, mut command) = inbox_command();
    let output = command
        .current_dir(directory.path())
        .args([
            "--value-log",
            "value-funnel.jsonl",
            "--output",
            "inbox.json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("inbox.json").is_file());
    let log = directory.path().join("value-funnel.jsonl");
    let before = fs::read(&log).unwrap();

    let alias = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .current_dir(directory.path())
        .args([
            "value-report",
            "value-funnel.jsonl",
            "--output",
            "value-funnel.jsonl",
        ])
        .output()
        .unwrap();
    assert!(!alias.status.success());
    assert_eq!(fs::read(&log).unwrap(), before);

    let report = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .current_dir(directory.path())
        .args([
            "value-report",
            "value-funnel.jsonl",
            "--format",
            "json",
            "--output",
            "value-report.json",
        ])
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(directory.path().join("value-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["summary"]["scans"], 1);
    assert_eq!(report["summary"]["delivery_confirmed_scans"], 1);
    assert_eq!(report["summary"]["delivery_unconfirmed_scans"], 0);
}
