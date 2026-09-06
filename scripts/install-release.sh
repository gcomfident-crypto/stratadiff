#!/usr/bin/env bash
set -euo pipefail

readonly stratadiff_github_host=github.com
readonly stratadiff_repository=gcomfident-crypto/stratadiff
readonly stratadiff_signer_workflow=gcomfident-crypto/stratadiff/.github/workflows/release.yml

die() {
  echo "$*" >&2
  exit 1
}

usage() {
  echo "usage: scripts/install-release.sh vMAJOR.MINOR.PATCH [INSTALL_DIRECTORY]" >&2
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required to install StrataDiff"
}

resolve_release_tag() {
  local release_tag=$1
  local object object_type object_sha

  object="$(
    gh api \
      --hostname "${stratadiff_github_host}" \
      "repos/${stratadiff_repository}/git/ref/tags/${release_tag}" \
      --jq '.object.type + "\t" + .object.sha'
  )"

  for _ in 1 2 3 4 5 6 7 8; do
    object_type=${object%%$'\t'*}
    object_sha=${object#*$'\t'}
    if [[ "${object_type}" == "${object}" || \
          ! "${object_sha}" =~ ^[0-9a-f]{40}$ ]]; then
      die "GitHub returned an invalid object for tag ${release_tag}"
    fi
    case "${object_type}" in
      commit)
        printf '%s\n' "${object_sha}"
        return 0
        ;;
      tag)
        object="$(
          gh api \
            --hostname "${stratadiff_github_host}" \
            "repos/${stratadiff_repository}/git/tags/${object_sha}" \
            --jq '.object.type + "\t" + .object.sha'
        )"
        ;;
      *)
        die "tag ${release_tag} points to unsupported object type ${object_type}"
        ;;
    esac
  done

  die "tag ${release_tag} exceeds the eight-object dereference limit"
}

validate_install_directory() {
  local requested=$1
  local component current canonical
  local -a components

  [[ -n "${requested}" && "${requested}" == /* ]] || \
    die "install directory must be an absolute path"
  [[ "${requested}" != / ]] || die "refusing to install directly under /"
  [[ "${requested}" != */ && "${requested}" != *//* ]] || \
    die "install directory must be a normalized absolute path"
  [[ "/${requested#/}/" != *'/./'* && "/${requested#/}/" != *'/../'* ]] || \
    die "install directory must not contain . or .. path components"
  [[ "${requested}" != *$'\n'* && "${requested}" != *$'\r'* ]] || \
    die "install directory must be a single line"

  IFS=/ read -r -a components <<< "${requested#/}"
  current=
  for component in "${components[@]}"; do
    [[ -n "${component}" ]] || die "install directory contains an empty path component"
    current=${current}/${component}
    if [[ -L "${current}" ]]; then
      die "install directory traverses symbolic link: ${current}"
    fi
    if [[ -e "${current}" && ! -d "${current}" ]]; then
      die "install directory component is not a directory: ${current}"
    fi
  done

  mkdir -p -- "${requested}"
  canonical="$(cd -- "${requested}" && pwd -P)"
  [[ "${canonical}" == "${requested}" ]] || \
    die "install directory does not resolve to itself: ${requested}"
  printf '%s\n' "${canonical}"
}

[[ $# -eq 1 || $# -eq 2 ]] || usage

stratadiff_release_tag=$1
if [[ ! "${stratadiff_release_tag}" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  die "release tag must be a stable vMAJOR.MINOR.PATCH tag: ${stratadiff_release_tag}"
fi
stratadiff_release_version=${stratadiff_release_tag#v}

if [[ $# -eq 2 ]]; then
  stratadiff_requested_install_directory=$2
else
  [[ -n "${HOME:-}" ]] || die "HOME is required when INSTALL_DIRECTORY is omitted"
  stratadiff_requested_install_directory=${HOME}/.local/bin
fi

require_command gh
require_command uname
require_command mkdir
require_command mktemp
require_command chmod
require_command install
require_command mv
require_command rm
require_command sort

stratadiff_kernel="$(uname -s)"
stratadiff_machine="$(uname -m)"
case "${stratadiff_kernel}:${stratadiff_machine}" in
  Linux:x86_64)
    stratadiff_asset_name=stratadiff-linux-x86_64
    ;;
  Linux:aarch64|Linux:arm64)
    stratadiff_asset_name=stratadiff-linux-aarch64
    ;;
  Darwin:x86_64)
    stratadiff_asset_name=stratadiff-macos-x86_64
    ;;
  Darwin:arm64|Darwin:aarch64)
    stratadiff_asset_name=stratadiff-macos-arm64
    ;;
  *)
    die "unsupported release platform: ${stratadiff_kernel} ${stratadiff_machine}"
    ;;
esac

stratadiff_install_directory="$(
  validate_install_directory "${stratadiff_requested_install_directory}"
)"
stratadiff_destination=${stratadiff_install_directory}/stratadiff
if [[ -L "${stratadiff_destination}" || \
      (-e "${stratadiff_destination}" && ! -f "${stratadiff_destination}") ]]; then
  die "refusing to replace unsafe install target: ${stratadiff_destination}"
fi

stratadiff_temporary_directory="$(
  mktemp -d "${TMPDIR:-/tmp}/stratadiff-install-XXXXXX"
)"
stratadiff_staged_path=
cleanup() {
  if [[ -n "${stratadiff_staged_path}" ]]; then
    rm -f -- "${stratadiff_staged_path}"
  fi
  rm -r -- "${stratadiff_temporary_directory}"
}
trap cleanup EXIT

stratadiff_source_digest="$(resolve_release_tag "${stratadiff_release_tag}")"

gh release download "${stratadiff_release_tag}" \
  --repo "${stratadiff_github_host}/${stratadiff_repository}" \
  --dir "${stratadiff_temporary_directory}" \
  --pattern "${stratadiff_asset_name}" \
  --pattern "${stratadiff_asset_name}.sha256" \
  --pattern "${stratadiff_asset_name}.intoto.jsonl"

stratadiff_asset_path=${stratadiff_temporary_directory}/${stratadiff_asset_name}
stratadiff_checksum_path=${stratadiff_asset_path}.sha256
stratadiff_bundle_path=${stratadiff_asset_path}.intoto.jsonl
stratadiff_expected_inventory="$(
  printf '%s\n' \
    "${stratadiff_asset_name}" \
    "${stratadiff_asset_name}.intoto.jsonl" \
    "${stratadiff_asset_name}.sha256" | LC_ALL=C sort
)"
stratadiff_actual_inventory="$(
  (
    shopt -s dotglob nullglob
    for stratadiff_downloaded_entry in "${stratadiff_temporary_directory}"/*; do
      printf '%s\n' "${stratadiff_downloaded_entry##*/}"
    done
  ) | LC_ALL=C sort
)"
[[ "${stratadiff_actual_inventory}" == "${stratadiff_expected_inventory}" ]] || \
  die "downloaded release asset inventory is incomplete or contains unexpected files"

[[ -f "${stratadiff_asset_path}" && ! -L "${stratadiff_asset_path}" && \
   -s "${stratadiff_asset_path}" ]] || \
  die "downloaded release binary is not a nonempty regular file"
[[ -f "${stratadiff_checksum_path}" && ! -L "${stratadiff_checksum_path}" && \
   -s "${stratadiff_checksum_path}" ]] || \
  die "downloaded release checksum is not a nonempty regular file"
[[ -f "${stratadiff_bundle_path}" && ! -L "${stratadiff_bundle_path}" && \
   -s "${stratadiff_bundle_path}" ]] || \
  die "downloaded release attestation bundle is not a nonempty regular file"

stratadiff_checksum_line="$(< "${stratadiff_checksum_path}")"
if [[ ! "${stratadiff_checksum_line}" =~ ^[0-9a-f]{64}[[:space:]][[:space:]]${stratadiff_asset_name}$ ]]; then
  die "downloaded release checksum record is invalid"
fi
stratadiff_expected_sha256=${stratadiff_checksum_line%% *}
if command -v sha256sum >/dev/null 2>&1; then
  stratadiff_actual_sha256="$(sha256sum "${stratadiff_asset_path}")"
  stratadiff_actual_sha256=${stratadiff_actual_sha256%% *}
elif command -v shasum >/dev/null 2>&1; then
  stratadiff_actual_sha256="$(shasum -a 256 "${stratadiff_asset_path}")"
  stratadiff_actual_sha256=${stratadiff_actual_sha256%% *}
else
  die "sha256sum or shasum is required to verify the release checksum"
fi
[[ "${stratadiff_actual_sha256}" == "${stratadiff_expected_sha256}" ]] || \
  die "release checksum mismatch: ${stratadiff_asset_name}"

gh attestation verify "${stratadiff_asset_path}" \
  --hostname "${stratadiff_github_host}" \
  --bundle "${stratadiff_bundle_path}" \
  --repo "${stratadiff_repository}" \
  --source-ref "refs/tags/${stratadiff_release_tag}" \
  --source-digest "${stratadiff_source_digest}" \
  --signer-workflow "${stratadiff_signer_workflow}" \
  --deny-self-hosted-runners >/dev/null

chmod 0755 "${stratadiff_asset_path}"
stratadiff_reported_version="$("${stratadiff_asset_path}" --version)"
[[ "${stratadiff_reported_version}" == "stratadiff ${stratadiff_release_version}" ]] || \
  die "release binary reports ${stratadiff_reported_version}, expected stratadiff ${stratadiff_release_version}"

stratadiff_final_source_digest="$(resolve_release_tag "${stratadiff_release_tag}")"
[[ "${stratadiff_final_source_digest}" == "${stratadiff_source_digest}" ]] || \
  die "release tag moved from ${stratadiff_source_digest} to ${stratadiff_final_source_digest}; refusing installation"

stratadiff_staged_path="$(
  mktemp "${stratadiff_install_directory}/.stratadiff-install-XXXXXX"
)"
install -m 0755 "${stratadiff_asset_path}" "${stratadiff_staged_path}"
if [[ -L "${stratadiff_destination}" || \
      (-e "${stratadiff_destination}" && ! -f "${stratadiff_destination}") ]]; then
  die "install target became unsafe: ${stratadiff_destination}"
fi
mv -f -- "${stratadiff_staged_path}" "${stratadiff_destination}"
stratadiff_staged_path=

printf 'Installed verified StrataDiff %s to %s\n' \
  "${stratadiff_release_version}" "${stratadiff_destination}"
