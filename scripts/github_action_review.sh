#!/usr/bin/env bash

set -euo pipefail
umask 077

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
    if ! env -u STRATADIFF_GITHUB_TOKEN \
      HOME="${provider_home}" \
      XDG_CONFIG_HOME="${provider_home}" \
      GIT_CONFIG_NOSYSTEM=1 \
      GIT_CONFIG_GLOBAL=/dev/null \
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
    "${curl_config}"
  do
    if [[ -n "${temporary_file}" ]] && ! rm -f -- "${temporary_file}"; then
      echo "failed to remove temporary StrataDiff credential or API file" >&2
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

target_directory="${RUNNER_TEMP}/stratadiff-target"
report_path="$(mktemp "${RUNNER_TEMP}/stratadiff-review-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-XXXXXX.json")"
(
  cd -- "${GITHUB_ACTION_PATH}"
  cargo build --release --locked --manifest-path Cargo.toml --target-dir "${target_directory}"
)
stratadiff="${target_directory}/release/stratadiff"
resolved_checkpoint="${STRATADIFF_CHECKPOINT}"
checkpoint_source=none

if [[ -n "${resolved_checkpoint}" ]]; then
  checkpoint_source=explicit
elif [[ -n "${STRATADIFF_REVIEWER}" ]]; then
  if [[ -z "${STRATADIFF_GITHUB_TOKEN}" ]]; then
    echo "reviewer requires the caller's GitHub token" >&2
    exit 1
  fi
  if [[ "${STRATADIFF_GITHUB_TOKEN}" == *$'\n'* || "${STRATADIFF_GITHUB_TOKEN}" == *$'\r'* ]]; then
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
    printf 'Authorization: Bearer %s\n' "${STRATADIFF_GITHUB_TOKEN}"
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
    curl --disable --config "${curl_config}" \
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
      curl --disable --config "${curl_config}" \
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
        | env -u STRATADIFF_GITHUB_TOKEN \
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
        env -u STRATADIFF_GITHUB_TOKEN \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_NOSYSTEM=1 \
          GIT_CONFIG_GLOBAL=/dev/null \
          GIT_TERMINAL_PROMPT=0 \
          git clone --bare --shared --quiet \
            "${STRATADIFF_REPOSITORY}" "${provider_repository}"
        env -u STRATADIFF_GITHUB_TOKEN \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_NOSYSTEM=1 \
          GIT_CONFIG_GLOBAL=/dev/null \
          git --git-dir="${provider_repository}" remote remove origin

        provider_repository_url="${STRATADIFF_GITHUB_SERVER_URL}/${STRATADIFF_GITHUB_REPOSITORY}.git"
        provider_ref="refs/stratadiff/provider/${resolved_checkpoint}"
        git_authorization="$(
          printf 'x-access-token:%s' "${STRATADIFF_GITHUB_TOKEN}" | base64 | tr -d '\n'
        )"
        if ! HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_NOSYSTEM=1 \
          GIT_CONFIG_GLOBAL=/dev/null \
          GIT_CONFIG_COUNT=5 \
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
          GIT_TERMINAL_PROMPT=0 \
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
          env -u STRATADIFF_GITHUB_TOKEN \
            HOME="${provider_home}" \
            XDG_CONFIG_HOME="${provider_home}" \
            GIT_CONFIG_NOSYSTEM=1 \
            GIT_CONFIG_GLOBAL=/dev/null \
            git --git-dir="${provider_repository}" \
              rev-parse --verify "${provider_ref}^{commit}"
        )"
        if [[ "${provider_commit}" != "${resolved_checkpoint}" ]]; then
          echo "provider fetch did not resolve the exact reviewed commit" >&2
          exit 1
        fi

        checkpoint_ref="refs/stratadiff/checkpoints/${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}-${resolved_checkpoint}"
        if env -u STRATADIFF_GITHUB_TOKEN \
          git -C "${STRATADIFF_REPOSITORY}" show-ref --verify --quiet "${checkpoint_ref}"
        then
          echo "temporary StrataDiff checkpoint ref already exists: ${checkpoint_ref}" >&2
          exit 1
        fi
        effective_local_url="$(
          env -u STRATADIFF_GITHUB_TOKEN \
            HOME="${provider_home}" \
            XDG_CONFIG_HOME="${provider_home}" \
            GIT_CONFIG_NOSYSTEM=1 \
            GIT_CONFIG_GLOBAL=/dev/null \
            git -C "${STRATADIFF_REPOSITORY}" \
              ls-remote --get-url "${provider_repository}"
        )"
        if [[ "${effective_local_url}" != "${provider_repository}" ]]; then
          echo "repository Git configuration rewrites the isolated checkpoint source" >&2
          exit 1
        fi
        caller_fetch_head_path="$(
          env -u STRATADIFF_GITHUB_TOKEN \
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
        if ! env -u STRATADIFF_GITHUB_TOKEN \
          HOME="${provider_home}" \
          XDG_CONFIG_HOME="${provider_home}" \
          GIT_CONFIG_NOSYSTEM=1 \
          GIT_CONFIG_GLOBAL=/dev/null \
          GIT_CONFIG_COUNT=3 \
          GIT_CONFIG_KEY_0=http.extraHeader \
          GIT_CONFIG_VALUE_0='' \
          GIT_CONFIG_KEY_1=credential.helper \
          GIT_CONFIG_VALUE_1='' \
          GIT_CONFIG_KEY_2=protocol.file.allow \
          GIT_CONFIG_VALUE_2=always \
          GIT_TERMINAL_PROMPT=0 \
          GIT_ASKPASS=/bin/false \
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
          env -u STRATADIFF_GITHUB_TOKEN \
            git -C "${STRATADIFF_REPOSITORY}" \
              rev-parse --verify "${checkpoint_ref}^{commit}"
        )"
        imported_description="$(
          printf '%s\n' "${resolved_checkpoint}" \
            | env -u STRATADIFF_GITHUB_TOKEN \
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

unset STRATADIFF_GITHUB_TOKEN
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
"${stratadiff}" review "${review_args[@]}"
review_status=$?
set -e
if [[ ! -s "${report_path}" ]]; then
  echo "StrataDiff did not produce a non-empty JSON report" >&2
  exit 1
fi
{
  echo "report=${report_path}"
  echo "checkpoint=${resolved_checkpoint}"
  echo "checkpoint_source=${checkpoint_source}"
  echo "checkpoint_record=${checkpoint_record_path}"
} >> "${GITHUB_OUTPUT}"
exit "${review_status}"
