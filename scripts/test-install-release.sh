#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_temporary_directory="$(
  cd -- "$(mktemp -d "${TMPDIR:-/tmp}/stratadiff-install-tests-XXXXXX")" && pwd -P
)"
trap 'rm -r -- "${stratadiff_temporary_directory}"' EXIT

export STRATADIFF_INSTALL_TEST_LOG=${stratadiff_temporary_directory}/gh.log
export STRATADIFF_INSTALL_TEST_STATE=${stratadiff_temporary_directory}/tag-state
export STRATADIFF_INSTALL_TEST_BINARY=${stratadiff_script_directory}/tests/fixtures/stratadiff-release-stub
export STRATADIFF_TEST_VERSION=0.3.0
export PATH=${stratadiff_script_directory}/tests/install-stubs:${PATH}

installer=${stratadiff_script_directory}/install-release.sh
release_tag=v0.3.0
release_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

fail() {
  echo "$*" >&2
  exit 1
}

reset_scenario() {
  export STRATADIFF_INSTALL_TEST_SCENARIO=$1
  export STRATADIFF_INSTALL_TEST_KERNEL=$2
  export STRATADIFF_INSTALL_TEST_MACHINE=$3
  : > "${STRATADIFF_INSTALL_TEST_LOG}"
  rm -f -- "${STRATADIFF_INSTALL_TEST_STATE}"
  STRATADIFF_TEST_VERSION=0.3.0
  export STRATADIFF_TEST_VERSION
}

assert_no_staged_file() {
  local install_directory=$1
  local staged
  for staged in "${install_directory}"/.stratadiff-install-*; do
    [[ -e "${staged}" || -L "${staged}" ]] || continue
    fail "installer left staged file behind: ${staged}"
  done
}

assert_old_binary() {
  local install_directory=$1
  [[ -f "${install_directory}/stratadiff" ]] || fail "old binary was removed"
  [[ "$(< "${install_directory}/stratadiff")" == old-binary ]] || \
    fail "old binary changed after failed installation"
  assert_no_staged_file "${install_directory}"
}

expect_failure_preserves_old_binary() {
  local label=$1
  local install_directory=$2
  mkdir -p -- "${install_directory}"
  printf 'old-binary\n' > "${install_directory}/stratadiff"
  chmod 0755 "${install_directory}/stratadiff"
  if "${installer}" "${release_tag}" "${install_directory}" >/dev/null 2>&1; then
    fail "installer accepted ${label}"
  fi
  assert_old_binary "${install_directory}"
}

run_platform_case() {
  local kernel=$1
  local machine=$2
  local expected_asset=$3
  local install_directory=${stratadiff_temporary_directory}/install-${kernel}-${machine}

  reset_scenario success "${kernel}" "${machine}"
  "${installer}" "${release_tag}" "${install_directory}" >/dev/null

  [[ -x "${install_directory}/stratadiff" ]] || \
    fail "${kernel} ${machine} did not install an executable"
  [[ "$("${install_directory}/stratadiff" --version)" == 'stratadiff 0.3.0' ]] || \
    fail "${kernel} ${machine} installed the wrong version"
  grep -Fx "ARG=${expected_asset}" "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null || \
    fail "${kernel} ${machine} selected the wrong asset"
  grep -Fx 'ARG=github.com/gcomfident-crypto/stratadiff' \
    "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null || fail "release repository was not fixed"
  grep -Fx 'ARG=gcomfident-crypto/stratadiff/.github/workflows/release.yml' \
    "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null || fail "signer workflow was not fixed"
  grep -Fx "ARG=${release_commit}" "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null || \
    fail "source digest was not enforced"
  grep -Fx 'ARG=--deny-self-hosted-runners' "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null || \
    fail "self-hosted runners were not denied"
  [[ "$(< "${STRATADIFF_INSTALL_TEST_STATE}")" == 2 ]] || \
    fail "release tag was not resolved before and after download"
  [[ "$(grep -Fc "ARG=repos/gcomfident-crypto/stratadiff/git/tags/cccccccccccccccccccccccccccccccccccccccc" "${STRATADIFF_INSTALL_TEST_LOG}")" == 2 ]] || \
    fail "annotated release tag was not fully dereferenced twice"
  assert_no_staged_file "${install_directory}"
}

run_platform_case Linux x86_64 stratadiff-linux-x86_64
run_platform_case Linux aarch64 stratadiff-linux-aarch64
run_platform_case Darwin x86_64 stratadiff-macos-x86_64
run_platform_case Darwin arm64 stratadiff-macos-arm64

reset_scenario success Linux x86_64
default_home=${stratadiff_temporary_directory}/default-home
mkdir -p -- "${default_home}"
HOME="${default_home}" "${installer}" "${release_tag}" >/dev/null
[[ -x "${default_home}/.local/bin/stratadiff" ]] || \
  fail "default install directory did not receive an executable"

reset_scenario tamper Linux x86_64
tamper_directory=${stratadiff_temporary_directory}/tamper
expect_failure_preserves_old_binary tampered-binary "${tamper_directory}"
if grep -Fx 'ARG=attestation' "${STRATADIFF_INSTALL_TEST_LOG}" >/dev/null; then
  fail "installer attempted attestation after checksum failure"
fi

reset_scenario missing-asset Linux x86_64
expect_failure_preserves_old_binary missing-asset \
  "${stratadiff_temporary_directory}/missing-asset"

reset_scenario extra-asset Linux x86_64
expect_failure_preserves_old_binary extra-asset \
  "${stratadiff_temporary_directory}/extra-asset"

reset_scenario attestation-failure Linux x86_64
expect_failure_preserves_old_binary attestation-failure \
  "${stratadiff_temporary_directory}/attestation-failure"

reset_scenario tag-drift Linux x86_64
expect_failure_preserves_old_binary tag-drift "${stratadiff_temporary_directory}/tag-drift"

reset_scenario success Linux x86_64
STRATADIFF_TEST_VERSION=9.9.9
export STRATADIFF_TEST_VERSION
expect_failure_preserves_old_binary version-mismatch \
  "${stratadiff_temporary_directory}/version-mismatch"

reset_scenario success FreeBSD x86_64
unsupported_directory=${stratadiff_temporary_directory}/unsupported
if "${installer}" "${release_tag}" "${unsupported_directory}" >/dev/null 2>&1; then
  fail "installer accepted an unsupported platform"
fi
[[ ! -e "${unsupported_directory}" ]] || fail "unsupported platform created an install directory"
[[ ! -s "${STRATADIFF_INSTALL_TEST_LOG}" ]] || \
  fail "unsupported platform contacted GitHub"

reset_scenario success Linux x86_64
if "${installer}" latest "${stratadiff_temporary_directory}/invalid-tag" >/dev/null 2>&1; then
  fail "installer accepted a non-version release tag"
fi
[[ ! -s "${STRATADIFF_INSTALL_TEST_LOG}" ]] || fail "invalid tag contacted GitHub"

reset_scenario success Linux x86_64
if "${installer}" "${release_tag}" relative/path >/dev/null 2>&1; then
  fail "installer accepted a relative install directory"
fi
if "${installer}" "${release_tag}" / >/dev/null 2>&1; then
  fail "installer accepted the filesystem root"
fi
unsafe_real=${stratadiff_temporary_directory}/unsafe-real
unsafe_link=${stratadiff_temporary_directory}/unsafe-link
mkdir -p -- "${unsafe_real}"
ln -s "${unsafe_real}" "${unsafe_link}"
if "${installer}" "${release_tag}" "${unsafe_link}" >/dev/null 2>&1; then
  fail "installer accepted a symlinked install directory"
fi
[[ ! -s "${STRATADIFF_INSTALL_TEST_LOG}" ]] || fail "unsafe directory contacted GitHub"

reset_scenario success Linux x86_64
unsafe_target_directory=${stratadiff_temporary_directory}/unsafe-target
sentinel=${stratadiff_temporary_directory}/sentinel
mkdir -p -- "${unsafe_target_directory}"
printf 'sentinel\n' > "${sentinel}"
ln -s "${sentinel}" "${unsafe_target_directory}/stratadiff"
if "${installer}" "${release_tag}" "${unsafe_target_directory}" >/dev/null 2>&1; then
  fail "installer accepted a symlinked install target"
fi
[[ "$(< "${sentinel}")" == sentinel ]] || fail "unsafe target changed its referent"
[[ ! -s "${STRATADIFF_INSTALL_TEST_LOG}" ]] || fail "unsafe target contacted GitHub"

reset_scenario success Linux x86_64
directory_target=${stratadiff_temporary_directory}/directory-target
mkdir -p -- "${directory_target}/stratadiff"
if "${installer}" "${release_tag}" "${directory_target}" >/dev/null 2>&1; then
  fail "installer accepted a directory as its install target"
fi
[[ -d "${directory_target}/stratadiff" ]] || fail "unsafe target directory changed"
[[ ! -s "${STRATADIFF_INSTALL_TEST_LOG}" ]] || \
  fail "unsafe target directory contacted GitHub"

reset_scenario success Linux x86_64
replace_directory=${stratadiff_temporary_directory}/replace-existing
mkdir -p -- "${replace_directory}"
printf 'old-binary\n' > "${replace_directory}/stratadiff"
chmod 0755 "${replace_directory}/stratadiff"
"${installer}" "${release_tag}" "${replace_directory}" >/dev/null
[[ "$("${replace_directory}/stratadiff" --version)" == 'stratadiff 0.3.0' ]] || \
  fail "installer did not replace an existing regular binary"
assert_no_staged_file "${replace_directory}"

printf 'release installer self-test passed\n'
