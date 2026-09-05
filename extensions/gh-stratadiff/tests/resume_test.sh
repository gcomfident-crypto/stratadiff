#!/usr/bin/env bash

set -euo pipefail

extension_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
stubs=${extension_directory}/tests/stubs
temporary_directory="$(mktemp -d "${TMPDIR:-/tmp}/gh-stratadiff-tests-XXXXXX")"
trap 'rm -rf -- "${temporary_directory}"' EXIT

assert_contains() {
  local haystack=$1
  local needle=$2
  [[ "${haystack}" == *"${needle}"* ]] || {
    printf 'expected output to contain: %s\nactual output:\n%s\n' "${needle}" "${haystack}" >&2
    exit 1
  }
}

assert_not_contains() {
  local haystack=$1
  local needle=$2
  [[ "${haystack}" != *"${needle}"* ]] || {
    printf 'expected output not to contain: %s\nactual output:\n%s\n' "${needle}" "${haystack}" >&2
    exit 1
  }
}

run_extension() {
  local name=$1
  shift
  local case_directory=${temporary_directory}/${name}
  local fetch_head_existed=true
  local setting
  for setting in "$@"; do
    if [[ "${setting}" == GH_TEST_FETCH_HEAD_ABSENT=true ]]; then
      fetch_head_existed=false
    fi
  done
  mkdir -p "${case_directory}"
  mkdir -p "${case_directory}/state"
  mkdir -p "${case_directory}/home"
  mkdir -p "${case_directory}/repository"
  : > "${case_directory}/calls.txt"
  if [[ "${fetch_head_existed}" == true ]]; then
    printf 'original fetch head\n' > "${case_directory}/FETCH_HEAD"
  fi
  set +e
  CASE_OUTPUT="$(
    env \
      PATH="${stubs}:${PATH}" \
      HOME="${case_directory}/home" \
      DISPLAY=:99 \
      WAYLAND_DISPLAY=wayland-stratadiff-test \
      XDG_RUNTIME_DIR="${case_directory}/runtime" \
      DBUS_SESSION_BUS_ADDRESS=unix:path=/stratadiff-test-bus \
      XAUTHORITY="${case_directory}/Xauthority" \
      LANG=C.UTF-8 \
      GH_TOKEN=must-not-reach-git \
      GITHUB_TOKEN=must-not-reach-viewer \
      CALLER_SECRET=must-not-reach-viewer \
      github_token=must-not-reach-git \
      git_authorization=must-not-reach-git \
      STRATADIFF_BIN=stratadiff \
      STRATADIFF_EXTENSION_TEST_LOG="${case_directory}/calls.txt" \
      GH_STUB_STATE="${case_directory}/state" \
      GH_STUB_FETCH_HEAD="${case_directory}/FETCH_HEAD" \
      GH_STUB_REPOSITORY_DIRECTORY="${case_directory}/repository" \
      "$@" \
      bash "${extension_directory}/gh-stratadiff" resume 17 \
        --repo-dir "${case_directory}/repository" --no-open 2>&1
  )"
  CASE_STATUS=$?
  set -e
  CASE_LOG="$(< "${case_directory}/calls.txt")"
  CASE_STATE=${case_directory}/state
  CASE_REPOSITORY=${case_directory}/repository
  CASE_VIEWER_ENVIRONMENT=
  if [[ -f "${case_directory}/home/review-environment.txt" ]]; then
    CASE_LOG+=$'\n'"$(< "${case_directory}/home/review-call.txt")"
    CASE_VIEWER_ENVIRONMENT="$(< "${case_directory}/home/review-environment.txt")"
  fi
  if [[ "${fetch_head_existed}" == true ]]; then
    [[ "$(< "${case_directory}/FETCH_HEAD")" == 'original fetch head' ]] || {
      printf 'FETCH_HEAD changed for case %s\n' "${name}" >&2
      exit 1
    }
  elif [[ -e "${case_directory}/FETCH_HEAD" ]]; then
    printf 'FETCH_HEAD was created for case %s\n' "${name}" >&2
    exit 1
  fi
  if compgen -G "${case_directory}/state/ref-*" >/dev/null; then
    printf 'temporary ref state remained for case %s\n' "${name}" >&2
    exit 1
  fi
}

run_extension happy
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Resuming @alice review of acme/widget#17 at exact checkpoint cccccccccccccccccccccccccccccccccccccccc.'
assert_contains "${CASE_LOG}" 'gh api --include --hostname github.com repos/acme/widget/pulls/17/reviews?per_page=100&page=1'
assert_not_contains "${CASE_LOG}" 'page=101'
assert_not_contains "${CASE_LOG}" '--paginate'
assert_not_contains "${CASE_LOG}" '--slurp'
assert_contains "${CASE_LOG}" 'stratadiff github-checkpoint'
assert_contains "${CASE_LOG}" '--reviewer alice'
assert_contains "${CASE_LOG}" '--gh-included-response'
assert_contains "${CASE_LOG}" 'stratadiff github-commit-object'
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_REPOSITORY} --checkpoint cccccccccccccccccccccccccccccccccccccccc --workbench --port 0 --no-open"
assert_contains "${CASE_LOG}" 'gh pr view 17 --repo github.com/acme/widget'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'DISPLAY=:99'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'LANG=C.UTF-8'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_CONFIG_GLOBAL=/dev/null'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_NO_LAZY_FETCH=1'
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GH_TOKEN='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GITHUB_TOKEN='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'CALLER_SECRET='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'STRATADIFF_EXTENSION_TEST_LOG='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GH_STUB_STATE='

run_extension missing-current GH_STUB_MISSING_BASE=true GH_STUB_MISSING_HEAD=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
assert_contains "${CASE_LOG}" 'gh auth token --hostname github.com'
assert_contains "${CASE_LOG}" 'git --git-dir='
assert_contains "${CASE_LOG}" 'fetch --quiet --no-tags --no-recurse-submodules https://github.com/acme/widget.git aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:refs/stratadiff/provider/base-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'fetch --quiet --no-tags --no-recurse-submodules https://github.com/acme/widget.git bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:refs/stratadiff/provider/head-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
assert_contains "${CASE_LOG}" 'fetch-pack --no-progress'
assert_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_contains "${CASE_LOG}" 'update-ref --no-deref -d refs/stratadiff/resume/'
assert_contains "${CASE_LOG}" '/base aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" '/head bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
assert_not_contains "${CASE_LOG}" 'rev-parse --git-path FETCH_HEAD'
assert_not_contains "${CASE_LOG}" ' checkout '
assert_not_contains "${CASE_LOG}" ' switch '
assert_not_contains "${CASE_LOG}" ' reset '
assert_not_contains "${CASE_LOG}" 'stub-github-token'

run_extension missing-checkpoint GH_STUB_MISSING_CHECKPOINT=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Resuming @alice review of acme/widget#17 at exact checkpoint cccccccccccccccccccccccccccccccccccccccc.'
assert_contains "${CASE_LOG}" 'fetch --quiet --no-tags --no-recurse-submodules https://github.com/acme/widget.git cccccccccccccccccccccccccccccccccccccccc:refs/stratadiff/provider/checkpoint-cccccccccccccccccccccccccccccccccccccccc'
assert_contains "${CASE_LOG}" 'fetch-pack --no-progress'
assert_contains "${CASE_LOG}" 'update-ref --no-deref -d refs/stratadiff/resume/'
assert_contains "${CASE_LOG}" '/checkpoint cccccccccccccccccccccccccccccccccccccccc'
assert_not_contains "${CASE_LOG}" ' checkout '
assert_not_contains "${CASE_LOG}" 'stub-github-token'

run_extension provider-missing GH_STUB_PROVIDER_MISSING=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'GitHub no longer exposes exact review checkpoint cccccccccccccccccccccccccccccccccccccccc; the provider cannot materialize that commit'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension fetch-missing-checkpoint GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_MISSING_CHECKPOINT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'GitHub verified review checkpoint cccccccccccccccccccccccccccccccccccccccc, but no longer serves that exact commit over Git HTTPS'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension moved-head GH_STUB_HEAD_MOVES=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'pull request base or head changed while review coverage was being resolved'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension no-review GH_STUB_NO_REVIEW=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '@alice has no eligible completed review on acme/widget#17'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension too-many-reviews GH_STUB_TOO_MANY_REVIEWS=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'GitHub review count limit exceeded: observed at least 101, limit 100'
assert_contains "${CASE_LOG}" 'stratadiff github-checkpoint'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension local-import-failure GH_STUB_MISSING_CHECKPOINT=true GH_STUB_LOCAL_IMPORT_FAIL=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'failed to import verified review checkpoint'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension wrong-fetch-pack-record GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_WRONG_RECORD=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension extra-fetch-pack-record GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_EXTRA_RECORD=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension update-ref-collision GH_STUB_MISSING_CHECKPOINT=true GH_STUB_UPDATE_REF_COLLISION=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'temporary StrataDiff ref already exists or changed concurrently'
[[ -f "${CASE_STATE}/colliding-ref" ]]
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref -d refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension absent-fetch-head GH_TEST_FETCH_HEAD_ABSENT=true GH_STUB_MISSING_CHECKPOINT=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'fetch-pack --no-progress'

printf 'gh-stratadiff resume tests passed\n'
