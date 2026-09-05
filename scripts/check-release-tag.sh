#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/check-release-tag.sh TAG EXPECTED_COMMIT" >&2
  exit 2
fi

stratadiff_release_tag=$1
stratadiff_expected_commit=$2

if [[ ! "${stratadiff_release_tag}" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "release tag must be a stable vMAJOR.MINOR.PATCH tag: ${stratadiff_release_tag}" >&2
  exit 1
fi
if [[ ! "${stratadiff_expected_commit}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "expected release commit must be a full lowercase SHA-1" >&2
  exit 1
fi

cd -- "${stratadiff_repository_root}"

stratadiff_resolved_commit="$(
  git rev-parse --verify "refs/tags/${stratadiff_release_tag}^{commit}"
)"
if [[ "${stratadiff_resolved_commit}" != "${stratadiff_expected_commit}" ]]; then
  echo "release tag ${stratadiff_release_tag} resolves to ${stratadiff_resolved_commit}, expected ${stratadiff_expected_commit}" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain=v1 --untracked-files=all)" ]]; then
  echo "release checkout is dirty" >&2
  git status --short --untracked-files=all >&2
  exit 1
fi

stratadiff_manifest_version="$(
  cargo metadata --locked --no-deps --format-version 1 |
    python3 -c '
import json
import pathlib
import sys

metadata = json.load(sys.stdin)
repository_manifest = pathlib.Path(sys.argv[1]).resolve()
versions = [
    package["version"]
    for package in metadata["packages"]
    if pathlib.Path(package["manifest_path"]).resolve() == repository_manifest
]
if len(versions) != 1:
    raise SystemExit("could not identify exactly one root stratadiff package")
print(versions[0])
' "${stratadiff_repository_root}/Cargo.toml"
)"
stratadiff_tag_version=${stratadiff_release_tag#v}
if [[ "${stratadiff_manifest_version}" != "${stratadiff_tag_version}" ]]; then
  echo "release tag version ${stratadiff_tag_version} does not match Cargo version ${stratadiff_manifest_version}" >&2
  exit 1
fi

printf '%s\n' "${stratadiff_manifest_version}"
