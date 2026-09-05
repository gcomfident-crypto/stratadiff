#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

const REVIEW_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-v1.schema.json";
const REVIEW_DELTA_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json";

#[test]
fn action_manifest_exposes_the_versioned_review_delta_contract() {
    let action = include_str!("../action.yml");
    assert!(action.contains("review_delta:\n"));
    assert!(action.contains("review_delta_schema:\n"));
    assert!(action.contains("review_delta_engine_version:\n"));
    assert!(action.contains("runner-local review-delta-v1 JSON artifact"));
    assert!(action.contains("checkpoint mode requires Python 3"));
    assert!(action.contains("including dropped or unresolved checkpoint changes"));
    assert!(action.contains("GitHub token used after the build"));
    let build_step = action
        .find("- name: Build StrataDiff without credentials")
        .unwrap();
    let review_step = action.find("- name: Analyze the Git range").unwrap();
    let token_environment = action.find("STRATADIFF_GITHUB_TOKEN:").unwrap();
    assert!(build_step < review_step && review_step < token_environment);
    assert_eq!(action.matches("BASH_ENV: ''").count(), 2);
    assert_eq!(action.matches("STRATADIFF_ACTION_PHASE:").count(), 2);
}

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(repository: &Path, message: &str) -> String {
    git(repository, &["add", "--all"]);
    git(repository, &["commit", "-q", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn command_path(command: &str) -> PathBuf {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command}"))
        .output()
        .unwrap();
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

#[test]
fn action_materializes_a_force_pushed_review_commit_without_exposing_the_token() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let provider = directory.path().join("provider.git");
    let caller = directory.path().join("caller");
    let runner = directory.path().join("runner");
    let fake_bin = directory.path().join("fake-bin");
    let malicious_home = directory.path().join("malicious-home");
    for path in [&source, &runner, &fake_bin, &malicious_home] {
        fs::create_dir(path).unwrap();
    }

    git(&source, &["init", "-q"]);
    git(&source, &["config", "user.name", "StrataDiff Test"]);
    git(
        &source,
        &["config", "user.email", "stratadiff@example.test"],
    );
    fs::write(source.join("review.py"), "value = 0\n").unwrap();
    let base = commit(&source, "base");

    git(&source, &["checkout", "-q", "-b", "reviewed"]);
    fs::write(source.join("review.py"), "value = 1\n").unwrap();
    let checkpoint = commit(&source, "reviewed checkpoint");

    git(&source, &["checkout", "-q", "-b", "current", &base]);
    fs::write(source.join("review.py"), "value = 1\n").unwrap();
    let head = commit(&source, "force-pushed equivalent");

    git(&source, &["checkout", "-q", "-b", "unrelated", &base]);
    fs::write(source.join("unrelated.bin"), vec![b'x'; 256 * 1024]).unwrap();
    let unrelated = commit(&source, "unrelated provider history");

    let initialized = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(&provider)
        .output()
        .unwrap();
    assert!(initialized.status.success());
    git(
        &source,
        &[
            "push",
            "-q",
            provider.to_str().unwrap(),
            &format!("{checkpoint}:refs/heads/reviewed"),
            &format!("{unrelated}:refs/heads/unrelated"),
        ],
    );

    fs::create_dir(&caller).unwrap();
    git(&caller, &["init", "-q"]);
    git(
        &caller,
        &[
            "fetch",
            "-q",
            source.to_str().unwrap(),
            &format!("{head}:refs/heads/current"),
        ],
    );
    git(&caller, &["checkout", "-q", "current"]);
    git(
        &caller,
        &[
            "config",
            "url.https://attacker.invalid/.insteadOf",
            "https://github.example/",
        ],
    );
    let fetch_head_before = fs::read(caller.join(".git/FETCH_HEAD")).unwrap();
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(&caller)
            .args(["cat-file", "-e", &format!("{checkpoint}^{{commit}}")])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(&caller)
            .args(["cat-file", "-e", &format!("{unrelated}^{{commit}}")])
            .output()
            .unwrap()
            .status
            .success()
    );

    fs::write(
        malicious_home.join(".gitconfig"),
        "[url \"https://attacker.invalid/\"]\n\tinsteadOf = https://github.example/\n",
    )
    .unwrap();
    fs::write(
        malicious_home.join(".curlrc"),
        "url = \"https://attacker.invalid/token\"\n",
    )
    .unwrap();

    let reviews = directory.path().join("reviews.json");
    fs::write(
        &reviews,
        format!(
            r#"[{{"id":17,"user":{{"login":"alice","type":"User"}},"state":"APPROVED","html_url":"https://github.example/owner/repo/pull/7#pullrequestreview-17","commit_id":"{checkpoint}","submitted_at":"2026-09-05T10:00:00Z","author_association":"MEMBER"}}]"#
        ),
    )
    .unwrap();
    let commit_object = directory.path().join("commit.json");
    fs::write(&commit_object, format!(r#"{{"sha":"{checkpoint}"}}"#)).unwrap();
    let git_markers = directory.path().join("git-markers");
    fs::create_dir(&git_markers).unwrap();
    let curl_log = directory.path().join("curl.log");

    let rustc_wrapper = fake_bin.join("rustc-wrapper");
    write_executable(
        &rustc_wrapper,
        r#"#!/usr/bin/env bash
set -euo pipefail
[[ -z "${STRATADIFF_GITHUB_TOKEN+x}" ]]
[[ -z "${github_token+x}" ]]
: > "${TEST_GIT_MARKER_DIRECTORY}/build-wrapper-token-absent"
"#,
    );

    write_executable(
        &fake_bin.join("cargo"),
        r#"#!/usr/bin/env bash
set -euo pipefail
[[ -z "${STRATADIFF_GITHUB_TOKEN+x}" ]]
[[ -z "${github_token+x}" ]]
: > "${TEST_GIT_MARKER_DIRECTORY}/cargo-token-absent"
if [[ -n "${RUSTC_WRAPPER:-}" ]]; then
  "${RUSTC_WRAPPER}" --stratadiff-token-probe
fi
target=
while (($#)); do
  if [[ "$1" == --target-dir ]]; then
    target="$2"
    shift 2
  else
    shift
  fi
done
[[ -n "${target}" ]]
mkdir -p "${target}/release"
cp "${TEST_STRATADIFF_BINARY}" "${target}/release/stratadiff"
"#,
    );
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --disable ]]
[[ -z "${STRATADIFF_GITHUB_TOKEN+x}" ]]
[[ -z "${github_token+x}" ]]
for poisoned_name in HTTPS_PROXY https_proxy ALL_PROXY CURL_CA_BUNDLE SSL_CERT_FILE; do
  [[ -z "${!poisoned_name:-}" ]]
done
printf '%s\n' "$1" >> "${TEST_CURL_LOG}"
shift
output=
headers=
url=
while (($#)); do
  case "$1" in
    --config|--max-filesize|--output|--write-out|--dump-header)
      option="$1"
      value="$2"
      shift 2
      case "${option}" in
        --output) output="${value}" ;;
        --dump-header) headers="${value}" ;;
      esac
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done
if [[ -n "${headers}" ]]; then
  printf 'HTTP/1.1 200 OK\r\n\r\n' > "${headers}"
fi
case "${url}" in
  */pulls/7/reviews?per_page=100) cp "${TEST_REVIEWS_JSON}" "${output}" ;;
  */git/commits/*) cp "${TEST_COMMIT_JSON}" "${output}" ;;
  *) exit 92 ;;
esac
printf 200
"#,
    );
    write_executable(
        &fake_bin.join("git"),
        r#"#!/usr/bin/env bash
set -euo pipefail
arguments=("$@")
network_fetch=false
local_import=false
for poisoned_name in \
  GIT_CONFIG_PARAMETERS \
  GIT_EXEC_PATH \
  GIT_PROXY_COMMAND \
  GIT_TRACE2 \
  GIT_TRACE2_EVENT \
  GIT_TRACE2_PERF \
  GIT_TRACE2_CONFIG_PARAMS \
  GIT_TRACE2_ENV_VARS
do
  [[ -z "${!poisoned_name:-}" ]]
done
[[ -z "${STRATADIFF_GITHUB_TOKEN+x}" ]]
[[ -z "${github_token+x}" ]]
[[ "${GIT_NO_LAZY_FETCH:-}" == 1 ]]
[[ "${GIT_NO_REPLACE_OBJECTS:-}" == 1 ]]
for ((index = 0; index < ${#arguments[@]}; index++)); do
  if [[ "${arguments[index]}" == "${TEST_PROVIDER_URL}" ]]; then
    arguments[index]="${TEST_PROVIDER_REPOSITORY}"
    network_fetch=true
  elif [[ "${arguments[index]}" == */stratadiff-provider-*.tmp/repository.git ]]; then
    local_import=true
  fi
done
if [[ "${network_fetch}" == true ]]; then
  for poisoned_name in HTTPS_PROXY https_proxy ALL_PROXY CURL_CA_BUNDLE SSL_CERT_FILE; do
    [[ -z "${!poisoned_name:-}" ]]
  done
  [[ "${HOME}" == */stratadiff-provider-*.tmp/home ]]
  [[ "${GIT_CONFIG_VALUE_1:-}" == "AUTHORIZATION: basic "* ]]
  [[ "${GIT_CONFIG_VALUE_5:-}" == never ]]
  [[ "${GIT_CONFIG_VALUE_6:-}" == always ]]
  [[ "${GIT_CONFIG_VALUE_7:-}" == never ]]
  [[ -z "${GIT_CONFIG_VALUE_8:-}" ]]
  [[ "${GIT_CONFIG_VALUE_9:-}" == true ]]
  GIT_CONFIG_VALUE_7=always
  : > "${TEST_GIT_MARKER_DIRECTORY}/provider-fetch"
fi
if [[ "${local_import}" == true ]] && [[ " ${arguments[*]} " == *" fetch "* ]]; then
  : > "${TEST_GIT_MARKER_DIRECTORY}/tokenless-local-import"
fi
if [[ " ${arguments[*]} " == *" cat-file "* ]]; then
  : > "${TEST_GIT_MARKER_DIRECTORY}/cat-file-no-lazy-fetch"
fi
"${TEST_REAL_GIT}" "${arguments[@]}"
"#,
    );

    let output_file = directory.path().join("github-output.txt");
    let summary_file = directory.path().join("github-summary.md");
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let build_output = Command::new("bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/github_action_review.sh"
        ))
        .env("PATH", &path)
        .env("RUNNER_TEMP", &runner)
        .env("GITHUB_ACTION_PATH", env!("CARGO_MANIFEST_DIR"))
        .env("STRATADIFF_ACTION_PHASE", "build")
        .env("STRATADIFF_GITHUB_TOKEN", "must-not-reach-build")
        .env("github_token", "must-not-reach-build")
        .env("RUSTC_WRAPPER", &rustc_wrapper)
        .env("TEST_STRATADIFF_BINARY", env!("CARGO_BIN_EXE_stratadiff"))
        .env("TEST_GIT_MARKER_DIRECTORY", &git_markers)
        .output()
        .unwrap();
    assert!(
        build_output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build_output.stdout),
        String::from_utf8_lossy(&build_output.stderr)
    );
    assert!(git_markers.join("cargo-token-absent").is_file());
    assert!(git_markers.join("build-wrapper-token-absent").is_file());

    let output = Command::new("bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/github_action_review.sh"
        ))
        .env("PATH", &path)
        .env("HOME", &malicious_home)
        .env("CURL_HOME", &malicious_home)
        .env("RUNNER_TEMP", &runner)
        .env("GITHUB_RUN_ID", "123")
        .env("GITHUB_RUN_ATTEMPT", "1")
        .env("GITHUB_ACTION_PATH", env!("CARGO_MANIFEST_DIR"))
        .env("GITHUB_OUTPUT", &output_file)
        .env("GITHUB_STEP_SUMMARY", &summary_file)
        .env("STRATADIFF_BASE", &base)
        .env("STRATADIFF_HEAD", &head)
        .env("STRATADIFF_CHECKPOINT", "")
        .env("STRATADIFF_REVIEWER", "alice")
        .env("STRATADIFF_GITHUB_TOKEN", "secret-token")
        .env("github_token", "attacker-exported-placeholder")
        .env("STRATADIFF_PULL_REQUEST_INPUT", "7")
        .env("STRATADIFF_FAIL_ON_REVIEW_RESIDUE", "true")
        .env("STRATADIFF_REPOSITORY", &caller)
        .env("STRATADIFF_GITHUB_API_URL", "https://api.github.example")
        .env("STRATADIFF_GITHUB_SERVER_URL", "https://github.example")
        .env("STRATADIFF_GITHUB_REPOSITORY", "owner/repo")
        .env("STRATADIFF_EVENT_PULL_REQUEST", "")
        .env(
            "GIT_CONFIG_PARAMETERS",
            "'url.https://attacker.invalid/.insteadOf=https://github.example/' 'http.sslVerify=false'",
        )
        .env("GIT_EXEC_PATH", &fake_bin)
        .env("GIT_PROXY_COMMAND", fake_bin.join("proxy-command"))
        .env("GIT_TRACE2", git_markers.join("trace2-normal"))
        .env("GIT_TRACE2_EVENT", git_markers.join("trace2-event"))
        .env("GIT_TRACE2_PERF", git_markers.join("trace2-perf"))
        .env("GIT_TRACE2_CONFIG_PARAMS", "http.extraheader")
        .env(
            "GIT_TRACE2_ENV_VARS",
            "GIT_CONFIG_VALUE_1,STRATADIFF_GITHUB_TOKEN",
        )
        .env("GIT_NO_LAZY_FETCH", "0")
        .env("HTTPS_PROXY", "http://attacker.invalid:8080")
        .env("https_proxy", "http://attacker.invalid:8081")
        .env("ALL_PROXY", "socks5://attacker.invalid:1080")
        .env("CURL_CA_BUNDLE", fake_bin.join("attacker-ca.pem"))
        .env("SSL_CERT_FILE", fake_bin.join("attacker-cert.pem"))
        .env("RUSTC_WRAPPER", &rustc_wrapper)
        .env("TEST_STRATADIFF_BINARY", env!("CARGO_BIN_EXE_stratadiff"))
        .env("TEST_REVIEWS_JSON", &reviews)
        .env("TEST_COMMIT_JSON", &commit_object)
        .env("TEST_PROVIDER_URL", "https://github.example/owner/repo.git")
        .env("TEST_PROVIDER_REPOSITORY", &provider)
        .env("TEST_REAL_GIT", command_path("git"))
        .env("TEST_GIT_MARKER_DIRECTORY", &git_markers)
        .env("TEST_CURL_LOG", &curl_log)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let action_outputs = fs::read_to_string(&output_file).unwrap();
    assert!(action_outputs.contains(&format!("checkpoint={checkpoint}\n")));
    assert!(action_outputs.contains("checkpoint_source=github_review\n"));
    assert!(action_outputs.contains(&format!("review_delta_schema={REVIEW_DELTA_SCHEMA}\n")));
    assert!(action_outputs.contains(&format!(
        "review_delta_engine_version={}\n",
        env!("CARGO_PKG_VERSION")
    )));
    let checkpoint_record_path = action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("checkpoint_record="))
        .unwrap();
    let checkpoint_record: Value =
        serde_json::from_slice(&fs::read(checkpoint_record_path).unwrap()).unwrap();
    assert_eq!(
        checkpoint_record["schema"],
        "stratadiff-github-review-checkpoint-v1"
    );
    assert_eq!(checkpoint_record["requested_reviewer"], "alice");
    assert_eq!(checkpoint_record["checkpoint"]["review_id"], 17);
    assert_eq!(checkpoint_record["checkpoint"]["commit_id"], checkpoint);
    let report_path = action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("report="))
        .unwrap();
    let report: Value = serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["schema"], REVIEW_SCHEMA);
    assert_eq!(report["checkpoint"]["commit"], checkpoint);
    assert_eq!(report["summary"]["checkpoint"]["needs_review_now_files"], 0);
    let review_delta_path = action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("review_delta="))
        .unwrap();
    let review_delta: Value =
        serde_json::from_slice(&fs::read(review_delta_path).unwrap()).unwrap();
    assert_eq!(review_delta["schema"], REVIEW_DELTA_SCHEMA);
    assert_eq!(review_delta["engine_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(review_delta["comparison"], "checkpoint_to_head");
    assert_eq!(review_delta["summary"]["displayable_files"], 0);
    assert_eq!(review_delta["summary"]["unresolved_retired_changes"], 0);
    assert_eq!(review_delta["summary"]["needs_review_files"], 0);
    assert_eq!(review_delta["summary"]["gate_passed"], true);
    assert_eq!(review_delta["entries"].as_array().unwrap().len(), 0);
    assert_eq!(
        review_delta["unresolved_retired_changes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let step_summary = fs::read_to_string(&summary_file).unwrap();
    let delta_heading = step_summary
        .find("# StrataDiff exact review delta")
        .unwrap();
    let compatibility_heading = step_summary.find("# StrataDiff review focus").unwrap();
    assert!(delta_heading < compatibility_heading, "{step_summary}");
    assert!(
        step_summary.contains(
            "Needs review: **0 files** = **0** displayed delta entries + **0** unresolved retired changes"
        ),
        "{step_summary}"
    );
    assert!(
        step_summary.contains(&format!(
            "Contract: `review-delta-v1` (`{REVIEW_DELTA_SCHEMA}`)"
        )),
        "{step_summary}"
    );
    assert!(
        step_summary.contains("review-v1 compatibility and current-range diagnostics"),
        "{step_summary}"
    );

    let dropped_output_file = directory.path().join("github-output-dropped.txt");
    let dropped_summary_file = directory.path().join("github-summary-dropped.md");
    let dropped_output = Command::new("bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/github_action_review.sh"
        ))
        .env("PATH", &path)
        .env("HOME", &malicious_home)
        .env("CURL_HOME", &malicious_home)
        .env("RUNNER_TEMP", &runner)
        .env("GITHUB_RUN_ID", "123")
        .env("GITHUB_RUN_ATTEMPT", "2")
        .env("GITHUB_ACTION_PATH", env!("CARGO_MANIFEST_DIR"))
        .env("GITHUB_OUTPUT", &dropped_output_file)
        .env("GITHUB_STEP_SUMMARY", &dropped_summary_file)
        .env("STRATADIFF_BASE", &base)
        .env("STRATADIFF_HEAD", &base)
        .env("STRATADIFF_CHECKPOINT", &checkpoint)
        .env("STRATADIFF_REVIEWER", "")
        .env("STRATADIFF_GITHUB_TOKEN", "")
        .env("STRATADIFF_PULL_REQUEST_INPUT", "")
        .env("STRATADIFF_FAIL_ON_REVIEW_RESIDUE", "true")
        .env("STRATADIFF_REPOSITORY", &caller)
        .env("STRATADIFF_GITHUB_API_URL", "https://api.github.example")
        .env("STRATADIFF_GITHUB_SERVER_URL", "https://github.example")
        .env("STRATADIFF_GITHUB_REPOSITORY", "owner/repo")
        .env("STRATADIFF_EVENT_PULL_REQUEST", "")
        .env("TEST_STRATADIFF_BINARY", env!("CARGO_BIN_EXE_stratadiff"))
        .env("TEST_REVIEWS_JSON", &reviews)
        .env("TEST_COMMIT_JSON", &commit_object)
        .env("TEST_PROVIDER_URL", "https://github.example/owner/repo.git")
        .env("TEST_PROVIDER_REPOSITORY", &provider)
        .env("TEST_REAL_GIT", command_path("git"))
        .env("TEST_GIT_MARKER_DIRECTORY", &git_markers)
        .env("TEST_CURL_LOG", &curl_log)
        .output()
        .unwrap();
    assert!(!dropped_output.status.success());
    assert!(
        String::from_utf8_lossy(&dropped_output.stderr)
            .contains("review delta gate is open: 1 file needs review"),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&dropped_output.stdout),
        String::from_utf8_lossy(&dropped_output.stderr)
    );
    let dropped_action_outputs = fs::read_to_string(&dropped_output_file).unwrap();
    assert!(dropped_action_outputs.contains("checkpoint_source=explicit\n"));
    assert!(
        dropped_action_outputs.contains(&format!("review_delta_schema={REVIEW_DELTA_SCHEMA}\n"))
    );
    let dropped_review_delta_path = dropped_action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("review_delta="))
        .unwrap();
    let dropped_review_delta: Value =
        serde_json::from_slice(&fs::read(dropped_review_delta_path).unwrap()).unwrap();
    assert_eq!(dropped_review_delta["summary"]["displayable_files"], 1);
    assert_eq!(
        dropped_review_delta["summary"]["unresolved_retired_changes"],
        0
    );
    assert_eq!(dropped_review_delta["summary"]["needs_review_files"], 1);
    assert_eq!(dropped_review_delta["summary"]["gate_passed"], false);
    assert_eq!(dropped_review_delta["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        dropped_review_delta["entries"][0]["baseline_basis"],
        "checkpoint_snapshot"
    );
    assert_eq!(
        dropped_review_delta["unresolved_retired_changes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let dropped_summary = fs::read_to_string(&dropped_summary_file).unwrap();
    assert!(
        dropped_summary.contains(
            "Needs review: **1 file** = **1** displayed delta entries + **0** unresolved retired changes"
        ),
        "{dropped_summary}"
    );
    assert!(
        dropped_summary.contains("**1** checkpoint snapshots"),
        "{dropped_summary}"
    );
    assert!(
        dropped_summary.contains("| `review.py` | `checkpoint_snapshot` | `—` |"),
        "{dropped_summary}"
    );
    assert!(
        dropped_summary.contains(
            "Review coverage: **0** of 0 current files need review; **0** carried (**0** exact-identity, **0** four-way); **1** checkpoint changes retired"
        ),
        "{dropped_summary}"
    );

    write_executable(&fake_bin.join("python3"), "#!/usr/bin/env bash\nexit 97\n");
    let first_pass_output_file = directory.path().join("github-output-first-pass.txt");
    let first_pass_summary_file = directory.path().join("github-summary-first-pass.md");
    let first_pass_output = Command::new("bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/github_action_review.sh"
        ))
        .env("PATH", &path)
        .env("HOME", &malicious_home)
        .env("CURL_HOME", &malicious_home)
        .env("RUNNER_TEMP", &runner)
        .env("GITHUB_RUN_ID", "123")
        .env("GITHUB_RUN_ATTEMPT", "3")
        .env("GITHUB_ACTION_PATH", env!("CARGO_MANIFEST_DIR"))
        .env("GITHUB_OUTPUT", &first_pass_output_file)
        .env("GITHUB_STEP_SUMMARY", &first_pass_summary_file)
        .env("STRATADIFF_BASE", &base)
        .env("STRATADIFF_HEAD", &head)
        .env("STRATADIFF_CHECKPOINT", "")
        .env("STRATADIFF_REVIEWER", "")
        .env("STRATADIFF_GITHUB_TOKEN", "")
        .env("STRATADIFF_PULL_REQUEST_INPUT", "")
        .env("STRATADIFF_FAIL_ON_REVIEW_RESIDUE", "false")
        .env("STRATADIFF_REPOSITORY", &caller)
        .env("STRATADIFF_GITHUB_API_URL", "https://api.github.example")
        .env("STRATADIFF_GITHUB_SERVER_URL", "https://github.example")
        .env("STRATADIFF_GITHUB_REPOSITORY", "owner/repo")
        .env("STRATADIFF_EVENT_PULL_REQUEST", "")
        .env("TEST_STRATADIFF_BINARY", env!("CARGO_BIN_EXE_stratadiff"))
        .env("TEST_REVIEWS_JSON", &reviews)
        .env("TEST_COMMIT_JSON", &commit_object)
        .env("TEST_PROVIDER_URL", "https://github.example/owner/repo.git")
        .env("TEST_PROVIDER_REPOSITORY", &provider)
        .env("TEST_REAL_GIT", command_path("git"))
        .env("TEST_GIT_MARKER_DIRECTORY", &git_markers)
        .env("TEST_CURL_LOG", &curl_log)
        .output()
        .unwrap();
    assert!(
        first_pass_output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first_pass_output.stdout),
        String::from_utf8_lossy(&first_pass_output.stderr)
    );
    let first_pass_action_outputs = fs::read_to_string(&first_pass_output_file).unwrap();
    assert!(first_pass_action_outputs.contains("review_delta=\n"));
    assert!(first_pass_action_outputs.contains("review_delta_schema=\n"));
    assert!(first_pass_action_outputs.contains("review_delta_engine_version=\n"));
    assert!(first_pass_action_outputs.contains("checkpoint=\n"));
    assert!(first_pass_action_outputs.contains("checkpoint_source=none\n"));
    let first_pass_report_path = first_pass_action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("report="))
        .unwrap();
    let first_pass_report: Value =
        serde_json::from_slice(&fs::read(first_pass_report_path).unwrap()).unwrap();
    assert_eq!(first_pass_report["schema"], REVIEW_SCHEMA);
    assert!(first_pass_report["checkpoint"].is_null());
    let first_pass_summary = fs::read_to_string(&first_pass_summary_file).unwrap();
    assert!(first_pass_summary.starts_with("# StrataDiff review focus\n"));
    assert!(!first_pass_summary.contains("StrataDiff exact review delta"));
    assert_eq!(git(&caller, &["merge-base", &base, &checkpoint]), base);
    assert_eq!(git(&caller, &["cat-file", "-t", &checkpoint]), "commit");
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(&caller)
            .args(["cat-file", "-e", &format!("{unrelated}^{{commit}}")])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        git(&caller, &["for-each-ref", "refs/stratadiff/checkpoints"]).is_empty(),
        "temporary checkpoint ref was not cleaned up"
    );
    assert_eq!(
        fs::read(caller.join(".git/FETCH_HEAD")).unwrap(),
        fetch_head_before
    );
    assert!(git_markers.join("provider-fetch").is_file());
    assert!(git_markers.join("tokenless-local-import").is_file());
    assert!(git_markers.join("cat-file-no-lazy-fetch").is_file());
    assert!(git_markers.join("cargo-token-absent").is_file());
    assert!(git_markers.join("build-wrapper-token-absent").is_file());
    assert!(!git_markers.join("trace2-normal").exists());
    assert!(!git_markers.join("trace2-event").exists());
    assert!(!git_markers.join("trace2-perf").exists());
    assert_eq!(
        fs::read_to_string(&curl_log).unwrap(),
        "--disable\n--disable\n"
    );
    let sensitive_prefixes = [
        "stratadiff-provider-",
        "stratadiff-reviews-",
        "stratadiff-review-headers-",
        "stratadiff-commit-object-",
        "stratadiff-request-headers-",
        "stratadiff-curl-",
        "stratadiff-review-summary-",
    ];
    assert!(fs::read_dir(&runner).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        sensitive_prefixes
            .iter()
            .all(|prefix| !name.starts_with(prefix))
    }));
}
