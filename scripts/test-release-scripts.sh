#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"
stratadiff_temporary_directory="$(mktemp -d "${TMPDIR:-/tmp}/stratadiff-release-tests-XXXXXX")"
trap 'rm -r -- "${stratadiff_temporary_directory}"' EXIT

if command -v sha256sum >/dev/null 2>&1; then
  stratadiff_lock_sha256="$(sha256sum "${stratadiff_repository_root}/Cargo.lock")"
  stratadiff_lock_sha256=${stratadiff_lock_sha256%% *}
else
  stratadiff_lock_sha256="$(shasum -a 256 "${stratadiff_repository_root}/Cargo.lock")"
  stratadiff_lock_sha256=${stratadiff_lock_sha256%% *}
fi

stratadiff_test_version="$(
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
IFS=. read -r stratadiff_test_major stratadiff_test_minor stratadiff_test_patch <<< "${stratadiff_test_version}"
stratadiff_test_tag=v${stratadiff_test_version}
stratadiff_mismatched_tag=v${stratadiff_test_major}.${stratadiff_test_minor}.$((stratadiff_test_patch + 1))

stratadiff_test_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
stratadiff_stub=${stratadiff_script_directory}/tests/fixtures/stratadiff-release-stub
export STRATADIFF_TEST_REPOSITORY_ROOT=${stratadiff_repository_root}
export STRATADIFF_TEST_LOCK_SHA256=${stratadiff_lock_sha256}
export STRATADIFF_TEST_COMMIT=${stratadiff_test_commit}
export STRATADIFF_TEST_VERSION=${stratadiff_test_version}
export PATH=${stratadiff_script_directory}/tests/stubs:${PATH}

stratadiff_dist=${stratadiff_temporary_directory}/dist
for stratadiff_target in \
  x86_64-unknown-linux-musl \
  aarch64-unknown-linux-musl \
  x86_64-apple-darwin \
  aarch64-apple-darwin
do
  export STRATADIFF_TEST_TARGET=${stratadiff_target}
  "${stratadiff_script_directory}/package-release-binary.sh" \
    "${stratadiff_target}" \
    "${stratadiff_stub}" \
    "${stratadiff_dist}" \
    "${stratadiff_test_tag}" \
    "${stratadiff_test_commit}" >/dev/null
done

for stratadiff_binary in "${stratadiff_dist}"/stratadiff-*; do
  if [[ "${stratadiff_binary}" == *.sha256 || "${stratadiff_binary}" == *.intoto.jsonl ]]; then
    continue
  fi
  printf '{"test-only":true}\n' > "${stratadiff_binary}.intoto.jsonl"
done
"${stratadiff_script_directory}/verify-release-assets.sh" "${stratadiff_dist}"

export STRATADIFF_TEST_TARGET=x86_64-unknown-linux-musl
export STRATADIFF_TEST_DYNAMIC=true
if "${stratadiff_script_directory}/package-release-binary.sh" \
  x86_64-unknown-linux-musl \
  "${stratadiff_stub}" \
  "${stratadiff_temporary_directory}/dynamic" \
  "${stratadiff_test_tag}" \
  "${stratadiff_test_commit}" >/dev/null 2>&1
then
  echo "release packager accepted a dynamically linked Linux binary" >&2
  exit 1
fi
unset STRATADIFF_TEST_DYNAMIC

cp -R "${stratadiff_dist}" "${stratadiff_temporary_directory}/tampered"
printf 'tampered\n' >> "${stratadiff_temporary_directory}/tampered/stratadiff-linux-x86_64"
if "${stratadiff_script_directory}/verify-release-assets.sh" \
  "${stratadiff_temporary_directory}/tampered" >/dev/null 2>&1
then
  echo "release verifier accepted a tampered binary" >&2
  exit 1
fi

cp -R "${stratadiff_dist}" "${stratadiff_temporary_directory}/extra"
: > "${stratadiff_temporary_directory}/extra/unexpected"
if "${stratadiff_script_directory}/verify-release-assets.sh" \
  "${stratadiff_temporary_directory}/extra" >/dev/null 2>&1
then
  echo "release verifier accepted an unexpected asset" >&2
  exit 1
fi

cp -R "${stratadiff_dist}" "${stratadiff_temporary_directory}/extra-directory"
mkdir "${stratadiff_temporary_directory}/extra-directory/unexpected"
if "${stratadiff_script_directory}/verify-release-assets.sh" \
  "${stratadiff_temporary_directory}/extra-directory" >/dev/null 2>&1
then
  echo "release verifier accepted an unexpected directory" >&2
  exit 1
fi

cp -R "${stratadiff_dist}" "${stratadiff_temporary_directory}/symlink"
rm "${stratadiff_temporary_directory}/symlink/stratadiff-linux-x86_64.intoto.jsonl"
ln -s ../dist/stratadiff-linux-x86_64.intoto.jsonl \
  "${stratadiff_temporary_directory}/symlink/stratadiff-linux-x86_64.intoto.jsonl"
if "${stratadiff_script_directory}/verify-release-assets.sh" \
  "${stratadiff_temporary_directory}/symlink" >/dev/null 2>&1
then
  echo "release verifier accepted a symlinked asset" >&2
  exit 1
fi

if "${stratadiff_script_directory}/package-release-binary.sh" \
  x86_64-unknown-linux-musl \
  "${stratadiff_stub}" \
  "${stratadiff_temporary_directory}/bad-tag" \
  latest \
  "${stratadiff_test_commit}" >/dev/null 2>&1
then
  echo "release packager accepted a non-version tag" >&2
  exit 1
fi

stratadiff_tag_repository=${stratadiff_temporary_directory}/tag-repository
git clone --quiet --no-local "${stratadiff_repository_root}" "${stratadiff_tag_repository}"
cp "${stratadiff_script_directory}/check-release-tag.sh" \
  "${stratadiff_tag_repository}/scripts/check-release-tag.sh"
chmod 0755 "${stratadiff_tag_repository}/scripts/check-release-tag.sh"
git -C "${stratadiff_tag_repository}" add scripts/check-release-tag.sh
if ! git -C "${stratadiff_tag_repository}" diff --cached --quiet; then
  git -C "${stratadiff_tag_repository}" \
    -c user.name='StrataDiff release test' \
    -c user.email='release-test@invalid.example' \
    commit --quiet -m 'test release tag contract'
fi
stratadiff_tag_commit="$(git -C "${stratadiff_tag_repository}" rev-parse HEAD)"
git -C "${stratadiff_tag_repository}" tag -d "${stratadiff_test_tag}" >/dev/null 2>&1 || true
git -C "${stratadiff_tag_repository}" tag "${stratadiff_test_tag}"
stratadiff_tag_version="$(
  "${stratadiff_tag_repository}/scripts/check-release-tag.sh" \
    "${stratadiff_test_tag}" \
    "${stratadiff_tag_commit}"
)"
[[ "${stratadiff_tag_version}" == "${stratadiff_test_version}" ]] || {
  echo "release tag verifier returned an unexpected version" >&2
  exit 1
}
if "${stratadiff_tag_repository}/scripts/check-release-tag.sh" \
  "${stratadiff_test_tag}" \
  bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >/dev/null 2>&1
then
  echo "release tag verifier accepted the wrong commit" >&2
  exit 1
fi
git -C "${stratadiff_tag_repository}" tag -f "${stratadiff_mismatched_tag}" >/dev/null
if "${stratadiff_tag_repository}/scripts/check-release-tag.sh" \
  "${stratadiff_mismatched_tag}" \
  "${stratadiff_tag_commit}" >/dev/null 2>&1
then
  echo "release tag verifier accepted a manifest version mismatch" >&2
  exit 1
fi

stratadiff_remote_commit="$(
  "${stratadiff_script_directory}/resolve-release-tag.sh" acme/stratadiff v0.3.0
)"
[[ "${stratadiff_remote_commit}" == "${stratadiff_test_commit}" ]] || {
  echo "remote tag resolver returned an unexpected commit" >&2
  exit 1
}
if "${stratadiff_script_directory}/resolve-release-tag.sh" \
  acme/stratadiff v0.3.1 >/dev/null 2>&1
then
  echo "remote tag resolver accepted a non-commit tag target" >&2
  exit 1
fi

printf 'release packaging self-test passed\n'
