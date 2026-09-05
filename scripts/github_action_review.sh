#!/usr/bin/env bash

set +x
set -euo pipefail
umask 077

target_directory="${RUNNER_TEMP}/stratadiff-target"
case "${STRATADIFF_ACTION_PHASE:-review}" in
  build)
    unset STRATADIFF_GITHUB_TOKEN github_token git_authorization
    (
      cd -- "${GITHUB_ACTION_PATH}"
      cargo build --release --locked --manifest-path Cargo.toml --target-dir "${target_directory}"
    )
    exit 0
    ;;
  review) ;;
  *)
    echo "STRATADIFF_ACTION_PHASE must be build or review" >&2
    exit 1
    ;;
esac

github_token="${STRATADIFF_GITHUB_TOKEN}"
unset STRATADIFF_GITHUB_TOKEN
export -n github_token
git_authorization=
export -n git_authorization

clean_curl_environment=(env -u STRATADIFF_GITHUB_TOKEN)
clean_git_environment=(env -u STRATADIFF_GITHUB_TOKEN)
while IFS= read -r environment_name; do
  case "${environment_name}" in
    GIT_*)
      clean_git_environment+=(-u "${environment_name}")
      ;;
    HTTP_PROXY|HTTPS_PROXY|ALL_PROXY|FTP_PROXY|NO_PROXY|http_proxy|https_proxy|all_proxy|ftp_proxy|no_proxy|CURL_CA_BUNDLE|SSL_CERT_FILE|SSL_CERT_DIR)
      clean_curl_environment+=(-u "${environment_name}")
      clean_git_environment+=(-u "${environment_name}")
      ;;
  esac
done < <(compgen -e)
# Git configuration and transport selection must not inherit state from earlier workflow steps.
clean_git_environment+=(
  GIT_CONFIG_NOSYSTEM=1
  GIT_CONFIG_GLOBAL=/dev/null
  GIT_TERMINAL_PROMPT=0
  GIT_ASKPASS=/bin/false
  GIT_NO_LAZY_FETCH=1
  GIT_NO_REPLACE_OBJECTS=1
)

reviews_path=
response_headers_path=
commit_object_path=
request_headers_path=
curl_config=
provider_workspace=
provider_home=
checkpoint_ref=
resolved_checkpoint=
checkpoint_record_path=
review_delta_path=
review_delta_schema=
review_delta_engine_version=
review_summary_path=
caller_fetch_head=
caller_fetch_head_backup=
caller_fetch_head_existed=false

restore_caller_fetch_head() {
  if [[ -z "${caller_fetch_head}" ]]; then
    return 0
  fi
  if [[ "${caller_fetch_head_existed}" == true ]]; then
    if ! cp -p -- "${caller_fetch_head_backup}" "${caller_fetch_head}"; then
      return 1
    fi
  else
    if ! rm -f -- "${caller_fetch_head}"; then
      return 1
    fi
  fi
  caller_fetch_head=
}

# shellcheck disable=SC2329
cleanup_action() {
  cleanup_status=$?
  trap - EXIT
  if [[ -n "${checkpoint_ref}" ]]; then
    if ! "${clean_git_environment[@]}" \
      HOME="${provider_home}" \
      XDG_CONFIG_HOME="${provider_home}" \
      git -C "${STRATADIFF_REPOSITORY}" update-ref -d "${checkpoint_ref}" "${resolved_checkpoint}"
    then
      echo "failed to remove temporary StrataDiff checkpoint ref ${checkpoint_ref}" >&2
      cleanup_status=1
    fi
  fi
  if ! restore_caller_fetch_head; then
    echo "failed to restore the caller repository's FETCH_HEAD" >&2
    cleanup_status=1
  fi
  for temporary_file in \
    "${reviews_path}" \
    "${response_headers_path}" \
    "${commit_object_path}" \
    "${request_headers_path}" \
    "${curl_config}" \
    "${review_summary_path}"
  do
    if [[ -n "${temporary_file}" ]] && ! rm -f -- "${temporary_file}"; then
      echo "failed to remove temporary StrataDiff action file" >&2
      cleanup_status=1
    fi
  done
  if [[ -n "${provider_workspace}" ]]; then
    if [[ "${provider_workspace}" != "${RUNNER_TEMP}"/stratadiff-provider-*.tmp ]]; then
      echo "refusing to remove an unexpected provider workspace path" >&2
      cleanup_status=1
    elif ! rm -rf -- "${provider_workspace}"; then
      echo "failed to remove temporary StrataDiff provider repository" >&2
      cleanup_status=1
    fi
  fi
  exit "${cleanup_status}"
}
trap cleanup_action EXIT

report_path="$(mktemp "${RUNNER_TEMP}/stratadiff-review-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-XXXXXX.json")"
review_summary_path="$(mktemp "${RUNNER_TEMP}/stratadiff-review-summary-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-XXXXXX.md")"
stratadiff="${target_directory}/release/stratadiff"
if [[ ! -x "${stratadiff}" ]]; then
  echo "StrataDiff action binary is missing; run the credential-free build phase first" >&2
  exit 1
fi
resolved_checkpoint="${STRATADIFF_CHECKPOINT}"
checkpoint_source=none

if [[ -n "${resolved_checkpoint}" ]]; then
  checkpoint_source=explicit
elif [[ -n "${STRATADIFF_REVIEWER}" ]]; then
  if [[ -z "${github_token}" ]]; then
    echo "reviewer requires the caller's GitHub token" >&2
    exit 1
  fi
  if [[ "${github_token}" == *$'\n'* || "${github_token}" == *$'\r'* ]]; then
    echo "github-token must not contain a line break" >&2
    exit 1
  fi
  pull_request="${STRATADIFF_PULL_REQUEST_INPUT:-${STRATADIFF_EVENT_PULL_REQUEST}}"
  if [[ ! "${pull_request}" =~ ^[1-9][0-9]*$ ]]; then
    echo "reviewer requires a pull request event or pull-request-number" >&2
    exit 1
  fi
  if [[ ! "${STRATADIFF_GITHUB_REPOSITORY}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    echo "github.repository is not a valid owner/repository name" >&2
    exit 1
  fi
  if [[ ! "${STRATADIFF_GITHUB_API_URL}" =~ ^https://[A-Za-z0-9][A-Za-z0-9.-]*(:[1-9][0-9]{0,4})?(/[A-Za-z0-9._~/-]*)?$ ]]; then
    echo "github.api_url must be an HTTPS URL without credentials, query, or fragment" >&2
    exit 1
  fi
  if [[ ! "${STRATADIFF_GITHUB_SERVER_URL}" =~ ^https://[A-Za-z0-9][A-Za-z0-9.-]*(:[1-9][0-9]{0,4})?$ ]]; then
    echo "github.server_url must be an HTTPS origin without credentials or a path" >&2
    exit 1
  fi

  reviews_path="$(mktemp "${RUNNER_TEMP}/stratadiff-reviews-XXXXXX.json")"
  response_headers_path="$(mktemp "${RUNNER_TEMP}/stratadiff-review-headers-XXXXXX.txt")"
  commit_object_path="$(mktemp "${RUNNER_TEMP}/stratadiff-commit-object-XXXXXX.json")"
  request_headers_path="$(mktemp "${RUNNER_TEMP}/stratadiff-request-headers-XXXXXX.txt")"
  curl_config="$(mktemp "${RUNNER_TEMP}/stratadiff-curl-XXXXXX.conf")"
  {
    printf 'Authorization: Bearer %s\n' "${github_token}"
    printf 'Accept: application/vnd.github+json\n'
    printf 'X-GitHub-Api-Version: 2022-11-28\n'
  } > "${request_headers_path}"
  {
    printf 'silent\n'
    printf 'show-error\n'
    printf 'fail-with-body\n'
    printf 'connect-timeout = 15\n'
    printf 'max-time = 60\n'
    printf 'proto = "=https"\n'
    printf 'header = "@%s"\n' "${request_headers_path}"
  } > "${curl_config}"

  reviews_status="$(
    "${clean_curl_environment[@]}" curl --disable --config "${curl_config}" \
      --max-filesize 8388608 \
      --dump-header "${response_headers_path}" \
      --output "${reviews_path}" \
      --write-out '%{http_code}' \
      "${STRATADIFF_GITHUB_API_URL}/repos/${STRATADIFF_GITHUB_REPOSITORY}/pulls/${pull_request}/reviews?per_page=100"
  )"
  if [[ "${reviews_status}" != 200 ]]; then
    echo "GitHub reviews endpoint returned HTTP ${reviews_status}, expected 200" >&2
    exit 1
  fi
  if grep -qi 'rel="next"' "${response_headers_path}"; then
    echo "pull request has more than 100 reviews; pass an explicit checkpoint" >&2
    exit 1
  fi

  checkpoint_record_path="$(mktemp "${RUNNER_TEMP}/stratadiff-checkpoint-record-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-XXXXXX.json")"
  "${stratadiff}" github-checkpoint "${reviews_path}" \
    --reviewer "${STRATADIFF_REVIEWER}" \
    --format json \
    --output "${checkpoint_record_path}"
  if [[ ! -s "${checkpoint_record_path}" ]]; then
    echo "StrataDiff did not produce a non-empty GitHub checkpoint selection record" >&2
    exit 1
  fi
  resolved_checkpoint="$(
    "${stratadiff}" github-checkpoint "${reviews_path}" --reviewer "${STRATADIFF_REVIEWER}"
  )"
  if [[ -n "${resolved_checkpoint}" ]]; then
    checkpoint_source=github_review
    commit_status="$(
      "${clean_curl_environment[@]}" curl --disable --config "${curl_config}" \
        --max-filesize 1048576 \
        --output "${commit_object_path}" \
        --write-out '%{http_code}' \
        "${STRATADIFF_GITHUB_API_URL}/repos/${STRATADIFF_GITHUB_REPOSITORY}/git/commits/${resolved_checkpoint}"
    )"
    if [[ "${commit_status}" != 200 ]]; then
      echo "GitHub commit-object endpoint returned HTTP ${commit_status}, expected 200" >&2
      exit 1
    fi
    verified_checkpoint="$(
      "${stratadiff}" github-commit-object "${commit_object_path}" \
        --expected "${resolved_checkpoint}"
    )"
    if [[ "${verified_checkpoint}" != "${resolved_checkpoint}" ]]; then
      echo "GitHub commit-object verification returned an unexpected object ID" >&2
      exit 1
    fi

    object_description="$(
      printf '%s\n' "${resolved_checkpoint}" \
        | "${clean_git_environment[@]}" \
          git -C "${STRATADIFF_REPOSITORY}" \
            cat-file --batch-check='%(objectname) %(objecttype)'
    )"
    case "${object_description}" in
      "${resolved_checkpoint} commit") ;;
      "${resolved_checkpoint} missing")
        provider_workspace="$(mktemp -d "${RUNNER_TEMP}/stratadiff-provider-XXXXXX.tmp")"
        provider_repository="${provider_workspace}/repository.git"
        provider_home="${provider_workspace}/home"
        mkdir -m 700 "${provider_home}"
        "${clean_git_environment[@]}" \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_COUNT=2 \
          GIT_CONFIG_KEY_0=protocol.allow \
          GIT_CONFIG_VALUE_0=never \
          GIT_CONFIG_KEY_1=protocol.file.allow \
          GIT_CONFIG_VALUE_1=always \
          git clone --bare --shared --quiet \
            "${STRATADIFF_REPOSITORY}" "${provider_repository}"
        "${clean_git_environment[@]}" \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          git --git-dir="${provider_repository}" remote remove origin

        provider_repository_url="${STRATADIFF_GITHUB_SERVER_URL}/${STRATADIFF_GITHUB_REPOSITORY}.git"
        provider_ref="refs/stratadiff/provider/${resolved_checkpoint}"
        git_authorization="$(
          printf 'x-access-token:%s' "${github_token}" | base64 | tr -d '\n'
        )"
        if ! "${clean_git_environment[@]}" \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_COUNT=10 \
          GIT_CONFIG_KEY_0=http.extraHeader \
          GIT_CONFIG_VALUE_0='' \
          GIT_CONFIG_KEY_1=http.extraHeader \
          GIT_CONFIG_VALUE_1="AUTHORIZATION: basic ${git_authorization}" \
          GIT_CONFIG_KEY_2=http.followRedirects \
          GIT_CONFIG_VALUE_2=false \
          GIT_CONFIG_KEY_3=http.sslVerify \
          GIT_CONFIG_VALUE_3=true \
          GIT_CONFIG_KEY_4=credential.helper \
          GIT_CONFIG_VALUE_4='' \
          GIT_CONFIG_KEY_5=protocol.allow \
          GIT_CONFIG_VALUE_5=never \
          GIT_CONFIG_KEY_6=protocol.https.allow \
          GIT_CONFIG_VALUE_6=always \
          GIT_CONFIG_KEY_7=protocol.file.allow \
          GIT_CONFIG_VALUE_7=never \
          GIT_CONFIG_KEY_8=http.proxy \
          GIT_CONFIG_VALUE_8='' \
          GIT_CONFIG_KEY_9=fetch.fsckObjects \
          GIT_CONFIG_VALUE_9=true \
          GIT_TRACE=0 \
          GIT_TRACE_CURL=0 \
          GIT_TRACE_PACKET=0 \
          GIT_TRACE_REDACT=1 \
          git --git-dir="${provider_repository}" fetch \
            --no-tags \
            --no-recurse-submodules \
            "${provider_repository_url}" \
            "${resolved_checkpoint}:${provider_ref}"
        then
          echo "GitHub no longer serves review checkpoint ${resolved_checkpoint}; submit a new review or materialize that exact commit and pass checkpoint explicitly" >&2
          exit 1
        fi
        git_authorization=

        provider_commit="$(
          "${clean_git_environment[@]}" \
            HOME="${provider_home}" \
            XDG_CONFIG_HOME="${provider_home}" \
            git --git-dir="${provider_repository}" \
              rev-parse --verify "${provider_ref}^{commit}"
        )"
        if [[ "${provider_commit}" != "${resolved_checkpoint}" ]]; then
          echo "provider fetch did not resolve the exact reviewed commit" >&2
          exit 1
        fi

        checkpoint_ref="refs/stratadiff/checkpoints/${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-${resolved_checkpoint}"
        if "${clean_git_environment[@]}" \
          git -C "${STRATADIFF_REPOSITORY}" show-ref --verify --quiet "${checkpoint_ref}"
        then
          echo "temporary StrataDiff checkpoint ref already exists: ${checkpoint_ref}" >&2
          exit 1
        fi
        effective_local_url="$(
          "${clean_git_environment[@]}" \
            HOME="${provider_home}" \
            XDG_CONFIG_HOME="${provider_home}" \
            git -C "${STRATADIFF_REPOSITORY}" \
              ls-remote --get-url "${provider_repository}"
        )"
        if [[ "${effective_local_url}" != "${provider_repository}" ]]; then
          echo "repository Git configuration rewrites the isolated checkpoint source" >&2
          exit 1
        fi
        caller_fetch_head_path="$(
          "${clean_git_environment[@]}" \
            git -C "${STRATADIFF_REPOSITORY}" rev-parse --git-path FETCH_HEAD
        )"
        if [[ "${caller_fetch_head_path}" == /* ]]; then
          caller_fetch_head="${caller_fetch_head_path}"
        else
          caller_fetch_head="${STRATADIFF_REPOSITORY%/}/${caller_fetch_head_path}"
        fi
        caller_fetch_head_backup="${provider_workspace}/FETCH_HEAD.before"
        if [[ -e "${caller_fetch_head}" ]]; then
          caller_fetch_head_existed=true
          cp -p -- "${caller_fetch_head}" "${caller_fetch_head_backup}"
        fi
        if ! "${clean_git_environment[@]}" \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_COUNT=5 \
          GIT_CONFIG_KEY_0=http.extraHeader \
          GIT_CONFIG_VALUE_0='' \
          GIT_CONFIG_KEY_1=credential.helper \
          GIT_CONFIG_VALUE_1='' \
          GIT_CONFIG_KEY_2=protocol.allow \
          GIT_CONFIG_VALUE_2=never \
          GIT_CONFIG_KEY_3=protocol.file.allow \
          GIT_CONFIG_VALUE_3=always \
          GIT_CONFIG_KEY_4=protocol.https.allow \
          GIT_CONFIG_VALUE_4=never \
          git -C "${STRATADIFF_REPOSITORY}" fetch \
            --no-tags \
            --no-recurse-submodules \
            "${provider_repository}" \
            "${provider_ref}:${checkpoint_ref}"
        then
          echo "failed to import the verified review checkpoint from the isolated repository" >&2
          exit 1
        fi
        if ! restore_caller_fetch_head; then
          echo "failed to restore the caller repository's FETCH_HEAD" >&2
          exit 1
        fi
        imported_commit="$(
          "${clean_git_environment[@]}" \
            git -C "${STRATADIFF_REPOSITORY}" \
              rev-parse --verify "${checkpoint_ref}^{commit}"
        )"
        imported_description="$(
          printf '%s\n' "${resolved_checkpoint}" \
            | "${clean_git_environment[@]}" \
              git -C "${STRATADIFF_REPOSITORY}" \
                cat-file --batch-check='%(objectname) %(objecttype)'
        )"
        if [[ "${imported_commit}" != "${resolved_checkpoint}" || "${imported_description}" != "${resolved_checkpoint} commit" ]]; then
          echo "imported checkpoint is not the exact reviewed commit object" >&2
          exit 1
        fi
        ;;
      *)
        echo "review checkpoint exists locally but is not a commit object" >&2
        exit 1
        ;;
    esac
  fi
fi

unset -v github_token git_authorization
if [[ -n "${request_headers_path}" ]]; then
  rm -f -- "${request_headers_path}"
  request_headers_path=
fi
if [[ -n "${curl_config}" ]]; then
  rm -f -- "${curl_config}"
  curl_config=
fi
if [[ -n "${resolved_checkpoint}" && ! "${resolved_checkpoint}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "checkpoint must resolve to a full lowercase SHA-1 commit ID" >&2
  exit 1
fi

review_args=(
  "--repo=${STRATADIFF_REPOSITORY}"
  --format json
  "--output=${report_path}"
  --github-summary
)
if [[ -n "${resolved_checkpoint}" ]]; then
  review_args+=("--checkpoint=${resolved_checkpoint}")
  review_delta_path="$(mktemp "${RUNNER_TEMP}/stratadiff-review-delta-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-XXXXXX.json")"
  review_args+=("--review-delta-output=${review_delta_path}")
fi
case "${STRATADIFF_FAIL_ON_REVIEW_RESIDUE}" in
  true) review_args+=(--github-annotations --fail-on-review-residue) ;;
  false) ;;
  *)
    echo "fail-on-review-residue must be true or false" >&2
    exit 1
    ;;
esac
review_args+=(-- "${STRATADIFF_BASE}" "${STRATADIFF_HEAD}")
set +e
GITHUB_STEP_SUMMARY="${review_summary_path}" "${stratadiff}" review "${review_args[@]}"
review_status=$?
set -e
if [[ ! -s "${report_path}" ]]; then
  echo "StrataDiff did not produce a non-empty JSON report" >&2
  exit 1
fi
if [[ ! -s "${review_summary_path}" ]]; then
  echo "StrataDiff did not produce a non-empty review-v1 step summary" >&2
  exit 1
fi
if [[ -n "${review_delta_path}" && ! -s "${review_delta_path}" ]]; then
  echo "StrataDiff did not produce a non-empty review delta" >&2
  exit 1
fi
if [[ -n "${review_delta_path}" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to validate and summarize the review delta" >&2
    exit 1
  fi
  review_delta_metadata="$(
    python3 - \
      "${review_delta_path}" \
      "${review_summary_path}" \
      "${GITHUB_STEP_SUMMARY}" \
      "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json" <<'PY'
import json
import re
import sys
from pathlib import Path

delta_path = Path(sys.argv[1])
legacy_summary_path = Path(sys.argv[2])
github_summary_path = Path(sys.argv[3])
expected_schema = sys.argv[4]

with delta_path.open(encoding="utf-8") as delta_file:
    delta = json.load(delta_file)

schema = delta["schema"]
engine_version = delta["engine_version"]
comparison = delta["comparison"]
summary = delta["summary"]
entries = delta["entries"]
unresolved_retired_changes = delta["unresolved_retired_changes"]

if schema != expected_schema:
    raise SystemExit(
        f"review delta schema mismatch: expected {expected_schema}, found {schema}"
    )
if not isinstance(engine_version, str) or not re.fullmatch(
    r"[0-9A-Za-z][0-9A-Za-z.+-]*", engine_version
):
    raise SystemExit("review delta engine_version is not a valid non-empty version string")
if comparison not in {"checkpoint_to_head", "per_file_review_baseline_to_head"}:
    raise SystemExit(f"unsupported review delta comparison: {comparison}")
if not isinstance(entries, list):
    raise SystemExit("review delta entries must be an array")
if not isinstance(unresolved_retired_changes, list):
    raise SystemExit("review delta unresolved_retired_changes must be an array")
displayable_files = summary["displayable_files"]
unresolved_retired_count = summary["unresolved_retired_changes"]
needs_review_files = summary["needs_review_files"]
gate_passed = summary["gate_passed"]
for name, value in {
    "displayable_files": displayable_files,
    "unresolved_retired_changes": unresolved_retired_count,
    "needs_review_files": needs_review_files,
}.items():
    if type(value) is not int or value < 0:
        raise SystemExit(f"review delta summary.{name} must be a non-negative integer")
if type(gate_passed) is not bool:
    raise SystemExit("review delta summary.gate_passed must be a boolean")
if displayable_files != len(entries):
    raise SystemExit(
        "review delta summary.displayable_files does not match the number of entries"
    )
if unresolved_retired_count != len(unresolved_retired_changes):
    raise SystemExit(
        "review delta summary.unresolved_retired_changes does not match the array"
    )
if needs_review_files != displayable_files + unresolved_retired_count:
    raise SystemExit("review delta summary.needs_review_files is not the queue total")
if gate_passed != (needs_review_files == 0):
    raise SystemExit("review delta summary.gate_passed disagrees with needs_review_files")

basis_counts = {
    "checkpoint_snapshot": 0,
    "current_base_no_checkpoint_change": 0,
    "reconstructed_review_baseline": 0,
    "current_base_fallback": 0,
    "checkpoint_head_fallback": 0,
}
for entry in entries:
    basis = entry["baseline_basis"]
    if basis not in basis_counts:
        raise SystemExit(f"unsupported review delta baseline_basis: {basis}")
    basis_counts[basis] += 1
for unresolved in unresolved_retired_changes:
    if not isinstance(unresolved["path"], str) or not unresolved["path"]:
        raise SystemExit("unresolved retired change path must be a non-empty string")
    if unresolved["path_encoding"] not in {"utf8", "git_bytes_percent_encoded"}:
        raise SystemExit("unsupported unresolved retired change path_encoding")
    if unresolved["reason"] != "non_utf8_git_path":
        raise SystemExit("unsupported unresolved retired change reason")

file_word = "file" if needs_review_files == 1 else "files"
comparison_label = {
    "checkpoint_to_head": "checkpoint snapshot → head",
    "per_file_review_baseline_to_head": "per-file review baseline → head",
}[comparison]
delta_path_code = str(delta_path).replace("`", "&#96;")
legacy_summary = legacy_summary_path.read_text(encoding="utf-8")

def markdown_cell(value):
    return (
        str(value)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\\", "\\\\")
        .replace("`", "&#96;")
        .replace("|", "\\|")
        .replace("\r", " ")
        .replace("\n", " ")
    )


def bounded_cell(value, limit):
    value = markdown_cell(value)
    return value if len(value) <= limit else f"{value[: limit - 1]}…"


def display_path(file):
    before = file["before_path"] if "before_path" in file else None
    after = file["after_path"] if "after_path" in file else None
    if before is not None and after is not None and before != after:
        return f"{before} → {after}"
    if after is not None:
        return after
    if before is not None:
        return before
    return "<unknown>"


delta_header = f"""# StrataDiff exact review delta

> **Gate source of truth.** This queue includes author changes after the checkpoint, dropped or reverted checkpoint changes, conservative fallbacks, and unresolved retired paths. The review-v1 section below remains available for compatibility and current-range diagnostics.

- Needs review: **{needs_review_files} {file_word}** = **{displayable_files}** displayed delta entries + **{unresolved_retired_count}** unresolved retired changes
- Gate: **{'passed' if gate_passed else 'blocked'}**
- Comparison: **{comparison_label}**
- Evidence basis: **{basis_counts['reconstructed_review_baseline']}** reconstructed review baselines; **{basis_counts['checkpoint_snapshot']}** checkpoint snapshots; **{basis_counts['current_base_no_checkpoint_change']}** current-base additions; **{basis_counts['current_base_fallback']}** current-base fallbacks; **{basis_counts['checkpoint_head_fallback']}** checkpoint-to-head fallbacks
- Contract: `review-delta-v1` (`{schema}`)
- Engine: `{engine_version}`
- Runner-local JSON: `{delta_path_code}` (not uploaded automatically)
"""
queue_rows = []
for entry in entries:
    file = entry["file"]
    fallback = entry["fallback_reason"] if "fallback_reason" in entry else "—"
    queue_rows.append(
        f"| `{bounded_cell(display_path(file), 180)}` | "
        f"`{entry['baseline_basis']}` | `{fallback}` | "
        f"{bounded_cell(file['reason'], 240)} |"
    )
for unresolved in unresolved_retired_changes:
    queue_rows.append(
        f"| `{bounded_cell(unresolved['path'], 180)}` | `unresolved_retired` | "
        f"`{unresolved['reason']}` | Path encoding: "
        f"`{unresolved['path_encoding']}`; no displayable delta could be reconstructed. |"
    )

shown_rows = queue_rows[:64]
if shown_rows:
    delta_header += (
        "\n## Exact queue\n\n"
        "| Path | Baseline basis | Fallback | Why |\n"
        "|---|---|---|---|\n"
        + "\n".join(shown_rows)
        + "\n"
    )
if len(queue_rows) > len(shown_rows):
    delta_header += (
        f"\n_{len(queue_rows) - len(shown_rows)} additional queue entries omitted; "
        "use the runner-local review delta JSON for the complete list._\n"
    )

delta_summary = f"""{delta_header}

<details>
<summary>review-v1 compatibility and current-range diagnostics</summary>

{legacy_summary}
</details>
"""
encoded_summary = delta_summary.encode("utf-8")
existing_size = github_summary_path.stat().st_size if github_summary_path.exists() else 0
if existing_size + len(encoded_summary) > 1024 * 1024:
    raise SystemExit("combined GitHub step summary would exceed 1 MiB")
with github_summary_path.open("ab") as github_summary_file:
    github_summary_file.write(encoded_summary)

print(schema)
print(engine_version)
PY
  )"
  if [[ "${review_delta_metadata}" != *$'\n'* ]]; then
    echo "review delta metadata renderer returned an invalid response" >&2
    exit 1
  fi
  review_delta_schema="${review_delta_metadata%%$'\n'*}"
  review_delta_engine_version="${review_delta_metadata#*$'\n'}"
else
  existing_summary_bytes=0
  if [[ -e "${GITHUB_STEP_SUMMARY}" ]]; then
    existing_summary_bytes="$(wc -c < "${GITHUB_STEP_SUMMARY}")"
  fi
  review_summary_bytes="$(wc -c < "${review_summary_path}")"
  if ((existing_summary_bytes + review_summary_bytes > 1024 * 1024)); then
    echo "combined GitHub step summary would exceed 1 MiB" >&2
    exit 1
  fi
  cat -- "${review_summary_path}" >> "${GITHUB_STEP_SUMMARY}"
fi
{
  echo "report=${report_path}"
  echo "review_delta=${review_delta_path}"
  echo "review_delta_schema=${review_delta_schema}"
  echo "review_delta_engine_version=${review_delta_engine_version}"
  echo "checkpoint=${resolved_checkpoint}"
  echo "checkpoint_source=${checkpoint_source}"
  echo "checkpoint_record=${checkpoint_record_path}"
} >> "${GITHUB_OUTPUT}"
exit "${review_status}"
