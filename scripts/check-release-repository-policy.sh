#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly stratadiff_script_directory

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/check-release-repository-policy.sh OWNER/REPOSITORY" >&2
  exit 2
fi

readonly stratadiff_repository=$1
readonly stratadiff_ruleset_name='Protect immutable v* release tags'

if [[ ! "${stratadiff_repository}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "repository must use OWNER/REPOSITORY form" >&2
  exit 1
fi
command -v gh >/dev/null 2>&1 || {
  echo "gh is required to verify release repository policy" >&2
  exit 1
}
command -v python3 >/dev/null 2>&1 || {
  echo "python3 is required to verify release repository policy" >&2
  exit 1
}

stratadiff_immutable="$(
  gh api --hostname github.com \
    "repos/${stratadiff_repository}/immutable-releases" \
    --jq '.enabled'
)"
if [[ "${stratadiff_immutable}" != true ]]; then
  echo "immutable releases are not enabled for ${stratadiff_repository}" >&2
  exit 1
fi

stratadiff_ruleset_id="$(
  gh api --hostname github.com \
    "repos/${stratadiff_repository}/rulesets" \
    --jq ".[] | select(.name == \"${stratadiff_ruleset_name}\") | .id"
)"
if [[ ! "${stratadiff_ruleset_id}" =~ ^[1-9][0-9]*$ ]]; then
  echo "exactly one ${stratadiff_ruleset_name} ruleset is required" >&2
  exit 1
fi

if ! gh api --hostname github.com \
  "repos/${stratadiff_repository}/rulesets/${stratadiff_ruleset_id}" |
  python3 "${stratadiff_script_directory}/validate-release-ruleset.py"
then
  echo "release tag ruleset is missing or does not match the required fail-closed policy" >&2
  exit 1
fi

printf 'verified immutable release and protected v* tag policy for %s\n' \
  "${stratadiff_repository}"
