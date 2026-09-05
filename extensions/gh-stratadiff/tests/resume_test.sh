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

run_case() {
  local name=$1
  local command=$2
  shift 2
  local case_directory=${temporary_directory}/${name}
  local fetch_head_existed=true
  local github_repository=github.com/acme/widget
  local audit_case=default
  local inbox_case=default
  local resume_case=repo-dir-only
  local setting
  for setting in "$@"; do
    case "${setting}" in
      GH_TEST_FETCH_HEAD_ABSENT=true) fetch_head_existed=false ;;
      GH_STUB_ENTERPRISE=true) github_repository=ghe.example/acme/widget ;;
      GH_TEST_RESUME_MODE=*) resume_case=${setting#GH_TEST_RESUME_MODE=} ;;
      GH_TEST_AUDIT_INFER=true) audit_case=infer ;;
      GH_TEST_AUDIT_ALL_OPTIONS=true) audit_case=all-options ;;
      GH_TEST_AUDIT_BAD_FORMAT=true) audit_case=bad-format ;;
      GH_TEST_AUDIT_ZERO_LIMIT=true) audit_case=zero-limit ;;
      GH_TEST_AUDIT_HIGH_LIMIT=true) audit_case=high-limit ;;
      GH_TEST_AUDIT_ZERO_DAYS=true) audit_case=zero-days ;;
      GH_TEST_AUDIT_HIGH_DAYS=true) audit_case=high-days ;;
      GH_TEST_INBOX_INFER=true) inbox_case=infer ;;
      GH_TEST_INBOX_ALL_OPTIONS=true) inbox_case=all-options ;;
      GH_TEST_INBOX_BAD_FORMAT=true) inbox_case=bad-format ;;
      GH_TEST_INBOX_POSITIONAL=true) inbox_case=positional ;;
      GH_TEST_INBOX_AUDIT_OPTION=true) inbox_case=audit-option ;;
    esac
  done
  mkdir -p "${case_directory}"
  mkdir -p "${case_directory}/state"
  mkdir -p "${case_directory}/home"
  mkdir -p "${case_directory}/repository"
  mkdir -p "${case_directory}/non-git"
  mkdir -p "${case_directory}/tmp"
  : > "${case_directory}/calls.txt"
  if [[ "${fetch_head_existed}" == true ]]; then
    printf 'original fetch head\n' > "${case_directory}/FETCH_HEAD"
  fi
  local -a command_arguments
  local command_directory=${case_directory}
  case "${command}" in
    demo)
      command_directory=${case_directory}/non-git
      command_arguments=(demo --port 0 --no-open)
      ;;
    resume)
      case "${resume_case}" in
        current)
          command_directory=${case_directory}/repository
          command_arguments=(resume 17 --no-open)
          ;;
        current-outside)
          command_directory=${case_directory}/non-git
          command_arguments=(resume 17 --no-open)
          ;;
        repo-dir-only)
          command_arguments=(resume 17 --repo-dir "${case_directory}/repository" --no-open)
          ;;
        repository-only)
          command_directory=${case_directory}/non-git
          command_arguments=(resume 17 -R "${github_repository}" --no-open)
          ;;
        repository-only-inside)
          command_directory=${case_directory}/repository
          command_arguments=(resume 17 -R "${github_repository}" --no-open)
          ;;
        both)
          command_directory=${case_directory}/non-git
          command_arguments=(
            resume 17
            --repo-dir "${case_directory}/repository"
            -R "${github_repository}"
            --no-open
          )
          ;;
        invalid-repo-dir)
          command_directory=${case_directory}/non-git
          command_arguments=(
            resume 17
            --repo-dir "${case_directory}/missing-repository"
            -R "${github_repository}"
            --no-open
          )
          ;;
        *)
          printf 'unknown resume test mode: %s\n' "${resume_case}" >&2
          exit 1
          ;;
      esac
      ;;
    ownership-snapshot)
      command_arguments=(
        ownership-snapshot
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
        --repo-dir "${case_directory}/repository"
        -R "${github_repository}"
        --output "${case_directory}/ownership.json"
      )
      ;;
    audit)
      command_directory=${case_directory}/non-git
      case "${audit_case}" in
        default)
          command_arguments=(audit -R "${github_repository}")
          ;;
        infer)
          command_directory=${case_directory}/repository
          command_arguments=(audit)
          ;;
        all-options)
          command_arguments=(
            audit
            -R "${github_repository}"
            --limit 7
            --days 14
            --format json
            --output "${case_directory}/audit.json"
            --end-exclusive 2026-09-01T00:00:00Z
          )
          ;;
        bad-format)
          command_arguments=(audit -R "${github_repository}" --format yaml)
          ;;
        zero-limit)
          command_arguments=(audit -R "${github_repository}" --limit 0)
          ;;
        high-limit)
          command_arguments=(audit -R "${github_repository}" --limit 101)
          ;;
        zero-days)
          command_arguments=(audit -R "${github_repository}" --days 0)
          ;;
        high-days)
          command_arguments=(audit -R "${github_repository}" --days 366)
          ;;
      esac
      ;;
    inbox)
      command_directory=${case_directory}/non-git
      case "${inbox_case}" in
        default)
          command_arguments=(inbox -R "${github_repository}")
          ;;
        infer)
          command_directory=${case_directory}/repository
          command_arguments=(inbox)
          ;;
        all-options)
          command_arguments=(
            inbox
            -R "${github_repository}"
            --format json
            --output "${case_directory}/inbox.json"
          )
          ;;
        bad-format)
          command_arguments=(inbox -R "${github_repository}" --format yaml)
          ;;
        positional)
          command_arguments=(inbox -R "${github_repository}" 17)
          ;;
        audit-option)
          command_arguments=(inbox -R "${github_repository}" --limit 7)
          ;;
      esac
      ;;
    *)
      printf 'unknown test command: %s\n' "${command}" >&2
      exit 1
      ;;
  esac

  set +e
  CASE_OUTPUT="$(
    cd -- "${command_directory}"
    env \
        PATH="${stubs}:${PATH}" \
        HOME="${case_directory}/home" \
        TMPDIR="${case_directory}/tmp" \
        DISPLAY=:99 \
        WAYLAND_DISPLAY=wayland-stratadiff-test \
        XDG_RUNTIME_DIR="${case_directory}/runtime" \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/stratadiff-test-bus \
        XAUTHORITY="${case_directory}/Xauthority" \
        LANG=C.UTF-8 \
        LANGUAGE= \
        GH_TOKEN=must-not-reach-git \
        GITHUB_TOKEN=must-not-reach-viewer \
        CALLER_SECRET=must-not-reach-viewer \
        github_token=must-not-reach-git \
        git_authorization=must-not-reach-git \
        STRATADIFF_BIN=stratadiff \
        STRATADIFF_AUDIT_TOOL="${stubs}/audit_tool.py" \
        STRATADIFF_EXTENSION_TEST_LOG="${case_directory}/calls.txt" \
        GH_STUB_COMMAND="${command}" \
        GH_STUB_AUDIT_CWD="${command_directory}" \
        GH_STUB_STATE="${case_directory}/state" \
        GH_STUB_FETCH_HEAD="${case_directory}/FETCH_HEAD" \
        GH_STUB_REPOSITORY_DIRECTORY="${case_directory}/repository" \
        "$@" \
        bash "${extension_directory}/gh-stratadiff" "${command_arguments[@]}" 2>&1
  )"
  CASE_STATUS=$?
  set -e
  CASE_LOG="$(< "${case_directory}/calls.txt")"
  CASE_STATE=${case_directory}/state
  CASE_REPOSITORY=${case_directory}/repository
  CASE_ISOLATED_REPOSITORY="$(
    printf '%s\n' "${CASE_LOG}" |
      sed -n 's#^git init --bare --quiet \(.*/repository\.git\)$#\1#p'
  )"
  CASE_OUTPUT_PATH=${case_directory}/ownership.json
  CASE_AUDIT_OUTPUT_PATH=${case_directory}/audit.json
  CASE_INBOX_OUTPUT_PATH=${case_directory}/inbox.json
  CASE_VIEWER_ENVIRONMENT=
  if [[ -f "${case_directory}/home/review-environment.txt" ]]; then
    CASE_LOG+=$'\n'"$(< "${case_directory}/home/review-call.txt")"
    CASE_VIEWER_ENVIRONMENT="$(< "${case_directory}/home/review-environment.txt")"
  elif [[ -f "${case_directory}/home/demo-environment.txt" ]]; then
    CASE_LOG+=$'\n'"$(< "${case_directory}/home/demo-call.txt")"
    CASE_VIEWER_ENVIRONMENT="$(< "${case_directory}/home/demo-environment.txt")"
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
  if compgen -G "${case_directory}/repository/.git/objects/pack/*.keep" >/dev/null; then
    printf 'temporary pack keep file remained for case %s\n' "${name}" >&2
    exit 1
  fi
  if compgen -G "${case_directory}/tmp/gh-stratadiff-*" >/dev/null; then
    printf 'temporary directory remained for case %s\n' "${name}" >&2
    exit 1
  fi
}

run_extension() {
  local name=$1
  shift
  run_case "${name}" resume "$@"
}

run_demo() {
  local name=$1
  shift
  run_case "${name}" demo "$@"
}

run_ownership_snapshot() {
  local name=$1
  shift
  run_case "${name}" ownership-snapshot "$@"
}

run_audit() {
  local name=$1
  shift
  run_case "${name}" audit "$@"
}

run_inbox() {
  local name=$1
  shift
  run_case "${name}" inbox "$@"
}

assert_discovery_did_not_touch_git_state() {
  assert_not_contains "${CASE_LOG}" 'git '
  assert_not_contains "${CASE_LOG}" 'stratadiff '
  assert_not_contains "${CASE_LOG}" 'gh auth token'
  assert_not_contains "${CASE_LOG}" 'refs/stratadiff'
}

TOP_LEVEL_HELP="$(bash "${extension_directory}/gh-stratadiff" --help)"
assert_contains "${TOP_LEVEL_HELP}" 'inbox                      Find open PRs that need your review resumed'
assert_contains "${TOP_LEVEL_HELP}" 'demo                       Open a deterministic offline Review Resume scenario'
DEMO_HELP="$(bash "${extension_directory}/gh-stratadiff" demo --help)"
assert_contains "${DEMO_HELP}" 'Usage: gh stratadiff demo [options]'
assert_contains "${DEMO_HELP}" 'network-free Review Resume scenario'
set +e
DEMO_BAD_PORT_OUTPUT="$(bash "${extension_directory}/gh-stratadiff" demo --port 65536 2>&1)"
DEMO_BAD_PORT_STATUS=$?
DEMO_POSITIONAL_OUTPUT="$(bash "${extension_directory}/gh-stratadiff" demo unexpected 2>&1)"
DEMO_POSITIONAL_STATUS=$?
set -e
[[ "${DEMO_BAD_PORT_STATUS}" -ne 0 ]]
assert_contains "${DEMO_BAD_PORT_OUTPUT}" '--port must be an integer from 0 through 65535'
[[ "${DEMO_POSITIONAL_STATUS}" -ne 0 ]]
assert_contains "${DEMO_POSITIONAL_OUTPUT}" 'demo does not accept positional arguments'
INBOX_HELP="$(bash "${extension_directory}/gh-stratadiff" inbox --help)"
assert_contains "${INBOX_HELP}" 'Usage: gh stratadiff inbox [options]'
assert_contains "${INBOX_HELP}" '--format markdown|json'
assert_contains "${INBOX_HELP}" 'works outside a Git checkout'
RESUME_HELP="$(bash "${extension_directory}/gh-stratadiff" resume --help)"
assert_contains "${RESUME_HELP}" 'With -R and no --repo-dir'
assert_contains "${RESUME_HELP}" 'isolated temporary bare'

run_demo demo-default
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'stratadiff demo --port 0 --no-open'
assert_not_contains "${CASE_LOG}" 'gh '
assert_not_contains "${CASE_LOG}" 'git '
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'DISPLAY=:99'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'LANG=C.UTF-8'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_CONFIG_GLOBAL=/dev/null'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_NO_LAZY_FETCH=1'
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GH_TOKEN='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GITHUB_TOKEN='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'CALLER_SECRET='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'STRATADIFF_EXTENSION_TEST_LOG='
assert_not_contains "${CASE_VIEWER_ENVIRONMENT}" 'GH_STUB_STATE='

run_audit audit-default
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" '# Review Memory Audit'
assert_contains "${CASE_LOG}" 'gh repo view github.com/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'audit-tool audit --repository acme/widget --hostname github.com --limit 50 --days 90 --format markdown'
assert_discovery_did_not_touch_git_state

run_audit audit-enterprise-all-options GH_STUB_ENTERPRISE=true GH_TEST_AUDIT_ALL_OPTIONS=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh repo view ghe.example/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" "audit-tool audit --repository acme/widget --hostname ghe.example --limit 7 --days 14 --format json --output ${CASE_AUDIT_OUTPUT_PATH} --end-exclusive 2026-09-01T00:00:00Z"
[[ -f "${CASE_AUDIT_OUTPUT_PATH}" ]]
assert_contains "$(< "${CASE_AUDIT_OUTPUT_PATH}")" '"status":"affected"'
assert_discovery_did_not_touch_git_state

run_audit audit-infer GH_TEST_AUDIT_INFER=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh repo view --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'audit-tool audit --repository acme/widget --hostname github.com --limit 50 --days 90 --format markdown'
assert_discovery_did_not_touch_git_state

run_audit audit-backend-failure AUDIT_STUB_EXIT_STATUS=23
[[ "${CASE_STATUS}" -eq 23 ]]
assert_contains "${CASE_LOG}" 'audit-tool audit --repository acme/widget --hostname github.com'
assert_discovery_did_not_touch_git_state

run_audit audit-invalid-format GH_TEST_AUDIT_BAD_FORMAT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--format must be markdown or json'
assert_not_contains "${CASE_LOG}" 'audit-tool'

run_inbox inbox-default
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" '# StrataDiff Review Inbox'
assert_contains "${CASE_LOG}" 'gh repo view github.com/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'audit-tool inbox --repository acme/widget --hostname github.com --format markdown'
assert_discovery_did_not_touch_git_state

run_inbox inbox-enterprise-json GH_STUB_ENTERPRISE=true GH_TEST_INBOX_ALL_OPTIONS=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh repo view ghe.example/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" "audit-tool inbox --repository acme/widget --hostname ghe.example --format json --output ${CASE_INBOX_OUTPUT_PATH}"
[[ -f "${CASE_INBOX_OUTPUT_PATH}" ]]
assert_contains "$(< "${CASE_INBOX_OUTPUT_PATH}")" '"schema":"stratadiff-review-inbox-v1"'
assert_discovery_did_not_touch_git_state

run_inbox inbox-infer GH_TEST_INBOX_INFER=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh repo view --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'audit-tool inbox --repository acme/widget --hostname github.com --format markdown'
assert_discovery_did_not_touch_git_state

run_inbox inbox-backend-failure INBOX_STUB_EXIT_STATUS=29
[[ "${CASE_STATUS}" -eq 29 ]]
assert_contains "${CASE_OUTPUT}" 'inbox backend stdout'
assert_contains "${CASE_OUTPUT}" 'inbox backend stderr'
assert_contains "${CASE_LOG}" 'audit-tool inbox --repository acme/widget --hostname github.com --format markdown'
assert_discovery_did_not_touch_git_state

run_inbox inbox-invalid-format GH_TEST_INBOX_BAD_FORMAT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--format must be markdown or json'
assert_not_contains "${CASE_LOG}" 'audit-tool'
assert_discovery_did_not_touch_git_state

run_inbox inbox-positional-rejected GH_TEST_INBOX_POSITIONAL=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'inbox does not accept positional arguments'
assert_not_contains "${CASE_LOG}" 'audit-tool'
assert_discovery_did_not_touch_git_state

run_inbox inbox-audit-option-rejected GH_TEST_INBOX_AUDIT_OPTION=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'unknown option: --limit'
assert_not_contains "${CASE_LOG}" 'audit-tool'
assert_discovery_did_not_touch_git_state

run_audit audit-zero-limit GH_TEST_AUDIT_ZERO_LIMIT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--limit must be an integer from 1 through 100'
assert_not_contains "${CASE_LOG}" 'audit-tool'

run_audit audit-high-limit GH_TEST_AUDIT_HIGH_LIMIT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--limit must be an integer from 1 through 100'
assert_not_contains "${CASE_LOG}" 'audit-tool'

run_audit audit-zero-days GH_TEST_AUDIT_ZERO_DAYS=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--days must be an integer from 1 through 365'
assert_not_contains "${CASE_LOG}" 'audit-tool'

run_audit audit-high-days GH_TEST_AUDIT_HIGH_DAYS=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--days must be an integer from 1 through 365'
assert_not_contains "${CASE_LOG}" 'audit-tool'

run_extension current-repository-mode GH_TEST_RESUME_MODE=current
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'git -C . rev-parse --show-toplevel'
assert_contains "${CASE_LOG}" 'gh repo view --json nameWithOwner,url'
assert_not_contains "${CASE_LOG}" 'git init --bare --quiet'
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_REPOSITORY}"

run_extension explicit-repository-mode GH_TEST_RESUME_MODE=both
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" "git -C ${CASE_REPOSITORY} rev-parse --show-toplevel"
assert_contains "${CASE_LOG}" 'gh repo view github.com/acme/widget --json nameWithOwner,url'
assert_not_contains "${CASE_LOG}" 'git init --bare --quiet'
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_REPOSITORY}"

run_extension first-run-non-git \
  GH_TEST_RESUME_MODE=repository-only \
  GH_STUB_FETCH_PACK_KEEP_RECORD=true
[[ "${CASE_STATUS}" -eq 0 ]]
[[ -n "${CASE_ISOLATED_REPOSITORY}" ]]
[[ ! -e "${CASE_ISOLATED_REPOSITORY}" ]]
assert_contains "${CASE_LOG}" "git init --bare --quiet ${CASE_ISOLATED_REPOSITORY}"
assert_contains "${CASE_LOG}" 'gh repo view github.com/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/cccccccccccccccccccccccccccccccccccccccc'
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_ISOLATED_REPOSITORY}"
assert_not_contains "${CASE_LOG}" 'rev-parse --show-toplevel'
assert_not_contains "${CASE_LOG}" "git -C ${CASE_REPOSITORY}"
assert_not_contains "${CASE_LOG}" ' clone '
assert_not_contains "${CASE_LOG}" ' checkout '
assert_not_contains "${CASE_LOG}" ' switch '
assert_not_contains "${CASE_LOG}" ' reset '
assert_not_contains "${CASE_LOG}" ' pull '

run_extension first-run-ignores-current-checkout GH_TEST_RESUME_MODE=repository-only-inside
[[ "${CASE_STATUS}" -eq 0 ]]
[[ -n "${CASE_ISOLATED_REPOSITORY}" ]]
[[ ! -e "${CASE_ISOLATED_REPOSITORY}" ]]
assert_contains "${CASE_LOG}" "git init --bare --quiet ${CASE_ISOLATED_REPOSITORY}"
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_ISOLATED_REPOSITORY}"
assert_not_contains "${CASE_LOG}" 'rev-parse --show-toplevel'
assert_not_contains "${CASE_LOG}" "git -C ${CASE_REPOSITORY}"

run_extension first-run-failure-cleans-scratch \
  GH_TEST_RESUME_MODE=repository-only \
  GH_STUB_PROVIDER_MISSING_BASE=true
[[ "${CASE_STATUS}" -ne 0 ]]
[[ -n "${CASE_ISOLATED_REPOSITORY}" ]]
[[ ! -e "${CASE_ISOLATED_REPOSITORY}" ]]
assert_contains "${CASE_OUTPUT}" 'GitHub no longer exposes exact pull request base aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension current-outside-does-not-fallback \
  GH_TEST_RESUME_MODE=current-outside \
  GH_STUB_NOT_GIT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'current directory is not inside a Git repository; use -R HOST/OWNER/REPO to run without a checkout'
assert_not_contains "${CASE_LOG}" 'git init --bare --quiet'
assert_not_contains "${CASE_LOG}" 'gh repo view'
assert_not_contains "${CASE_LOG}" 'stratadiff '

run_extension invalid-explicit-repo-dir-does-not-fallback \
  GH_TEST_RESUME_MODE=invalid-repo-dir \
  GH_STUB_NOT_GIT=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" '--repo-dir is not inside a Git repository'
assert_not_contains "${CASE_LOG}" 'git init --bare --quiet'
assert_not_contains "${CASE_LOG}" 'gh repo view'
assert_not_contains "${CASE_LOG}" 'stratadiff '

run_extension repo-dir-inferred-mode
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Resuming @alice review of acme/widget#17 at exact checkpoint cccccccccccccccccccccccccccccccccccccccc.'
assert_contains "${CASE_LOG}" "git -C ${CASE_REPOSITORY} rev-parse --show-toplevel"
assert_contains "${CASE_LOG}" 'gh repo view --json nameWithOwner,url'
assert_not_contains "${CASE_LOG}" 'git init --bare --quiet'
assert_contains "${CASE_LOG}" 'gh api --paginate --slurp --hostname github.com repos/acme/widget/pulls/17/reviews?per_page=100'
assert_not_contains "${CASE_LOG}" '&page='
assert_not_contains "${CASE_LOG}" '--include'
assert_contains "${CASE_LOG}" 'stratadiff github-checkpoint'
assert_contains "${CASE_LOG}" '--reviewer alice'
assert_contains "${CASE_LOG}" '--gh-slurp-pages'
assert_not_contains "${CASE_LOG}" '--gh-included-response'
assert_contains "${CASE_LOG}" 'stratadiff github-commit-object'
assert_contains "${CASE_LOG}" "stratadiff review aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --repo ${CASE_REPOSITORY} --checkpoint cccccccccccccccccccccccccccccccccccccccc --workbench --port 0 --no-open"
assert_contains "${CASE_LOG}" 'gh pr view 17 --repo github.com/acme/widget'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'DISPLAY=:99'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'LANG=C.UTF-8'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_CONFIG_GLOBAL=/dev/null'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" 'GIT_NO_LAZY_FETCH=1'
assert_contains "${CASE_VIEWER_ENVIRONMENT}" $'\nLANGUAGE=\n'
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

run_extension multi-page-reviews GH_STUB_MULTI_PAGE_REVIEWS=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Resuming @alice review of acme/widget#17 at exact checkpoint cccccccccccccccccccccccccccccccccccccccc.'
assert_contains "${CASE_LOG}" 'gh api --paginate --slurp --hostname github.com repos/acme/widget/pulls/17/reviews?per_page=100'
assert_contains "${CASE_LOG}" 'stratadiff github-checkpoint'
assert_contains "${CASE_LOG}" '--gh-slurp-pages'
assert_contains "${CASE_LOG}" 'stratadiff review '

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

run_extension keep-fetch-pack-record GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_KEEP_RECORD=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Resuming @alice review of acme/widget#17 at exact checkpoint cccccccccccccccccccccccccccccccccccccccc.'
assert_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'

run_extension duplicate-fetch-pack-keep GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_DUPLICATE_KEEP=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension malformed-fetch-pack-keep GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_MALFORMED_KEEP=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension late-fetch-pack-keep GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_KEEP_AFTER_REF=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension duplicate-fetch-pack-ref GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_DUPLICATE_REF=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension oversized-fetch-pack-output GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_OVERSIZED=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import returned oversized output'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension trailing-blank-fetch-pack-output GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_TRAILING_BLANK=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'local import did not return the exact verified review checkpoint ref'
assert_not_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/resume/'
assert_not_contains "${CASE_LOG}" 'stratadiff review '

run_extension unterminated-fetch-pack-output GH_STUB_MISSING_CHECKPOINT=true GH_STUB_FETCH_PACK_NO_FINAL_NEWLINE=true
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

run_ownership_snapshot ownership-happy
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_OUTPUT}" 'Collecting exact-base ownership for github.com/acme/widget at aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.'
assert_contains "${CASE_LOG}" 'gh repo view github.com/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'stratadiff github-commit-object'
assert_contains "${CASE_LOG}" "stratadiff github-ownership-snapshot aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --repo ${CASE_REPOSITORY} --github-repository acme/widget --provider-url https://github.com --output ${CASE_OUTPUT_PATH}"
assert_not_contains "${CASE_LOG}" 'gh auth token'
[[ -f "${CASE_OUTPUT_PATH}" ]]
[[ "$(python3 -c 'import os, stat, sys; print(oct(stat.S_IMODE(os.stat(sys.argv[1]).st_mode)))' "${CASE_OUTPUT_PATH}")" == 0o600 ]]
assert_contains "$(< "${CASE_OUTPUT_PATH}")" '"base_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"'

run_ownership_snapshot ownership-missing-base GH_STUB_MISSING_BASE=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh api --hostname github.com repos/acme/widget/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'gh auth token --hostname github.com'
assert_contains "${CASE_LOG}" 'fetch --quiet --no-tags --no-recurse-submodules https://github.com/acme/widget.git aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:refs/stratadiff/provider/base-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/ownership-snapshot/'
assert_contains "${CASE_LOG}" 'update-ref --no-deref -d refs/stratadiff/ownership-snapshot/'
assert_not_contains "${CASE_LOG}" 'stub-github-token'
[[ -f "${CASE_OUTPUT_PATH}" ]]

run_ownership_snapshot ownership-enterprise GH_STUB_ENTERPRISE=true
[[ "${CASE_STATUS}" -eq 0 ]]
assert_contains "${CASE_LOG}" 'gh repo view ghe.example/acme/widget --json nameWithOwner,url'
assert_contains "${CASE_LOG}" 'gh api --hostname ghe.example repos/acme/widget/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
assert_contains "${CASE_LOG}" "stratadiff github-ownership-snapshot aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --repo ${CASE_REPOSITORY} --github-repository acme/widget --provider-url https://ghe.example --output ${CASE_OUTPUT_PATH}"
assert_not_contains "${CASE_LOG}" '--hostname github.com'

run_ownership_snapshot ownership-provider-missing GH_STUB_PROVIDER_MISSING_BASE=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_OUTPUT}" 'GitHub no longer exposes exact base commit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; the provider cannot materialize that commit'
assert_not_contains "${CASE_LOG}" 'stratadiff github-ownership-snapshot'
[[ ! -e "${CASE_OUTPUT_PATH}" ]]

run_ownership_snapshot ownership-core-failure GH_STUB_MISSING_BASE=true GH_STUB_OWNERSHIP_FAIL=true
[[ "${CASE_STATUS}" -ne 0 ]]
assert_contains "${CASE_LOG}" 'update-ref --no-deref refs/stratadiff/ownership-snapshot/'
assert_contains "${CASE_LOG}" 'update-ref --no-deref -d refs/stratadiff/ownership-snapshot/'
[[ ! -e "${CASE_OUTPUT_PATH}" ]]

printf 'gh-stratadiff tests passed\n'
