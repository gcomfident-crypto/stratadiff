#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"

if [[ $# -ne 5 ]]; then
  echo "usage: scripts/package-release-binary.sh TARGET BINARY OUTPUT_DIR TAG COMMIT" >&2
  exit 2
fi

stratadiff_target=$1
stratadiff_source_binary=$2
stratadiff_output_directory=$3
stratadiff_release_tag=$4
stratadiff_release_commit=$5

case "${stratadiff_target}" in
  x86_64-unknown-linux-musl)
    stratadiff_asset_name=stratadiff-linux-x86_64
    ;;
  aarch64-unknown-linux-musl)
    stratadiff_asset_name=stratadiff-linux-aarch64
    ;;
  x86_64-apple-darwin)
    stratadiff_asset_name=stratadiff-macos-x86_64
    ;;
  aarch64-apple-darwin)
    stratadiff_asset_name=stratadiff-macos-arm64
    ;;
  *)
    echo "unsupported release target: ${stratadiff_target}" >&2
    exit 1
    ;;
esac

if [[ ! "${stratadiff_release_tag}" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "release tag must be a stable vMAJOR.MINOR.PATCH tag: ${stratadiff_release_tag}" >&2
  exit 1
fi
if [[ ! "${stratadiff_release_commit}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "release commit must be a full lowercase SHA-1" >&2
  exit 1
fi
if [[ ! -x "${stratadiff_source_binary}" ]]; then
  echo "release binary is missing or not executable: ${stratadiff_source_binary}" >&2
  exit 1
fi

case "${stratadiff_target}" in
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl)
    command -v readelf >/dev/null 2>&1 || {
      echo "readelf is required to validate a Linux release binary" >&2
      exit 1
    }
    stratadiff_program_headers="$(LC_ALL=C readelf -l "${stratadiff_source_binary}")"
    if [[ "${stratadiff_program_headers}" == *'Requesting program interpreter'* ]]; then
      echo "Linux release binary has a dynamic program interpreter; expected a static musl binary" >&2
      exit 1
    fi
    stratadiff_dynamic_section="$(LC_ALL=C readelf -d "${stratadiff_source_binary}" 2>/dev/null)"
    if [[ "${stratadiff_dynamic_section}" == *'(NEEDED)'* ]]; then
      echo "Linux release binary has a shared-library dependency; expected a static musl binary" >&2
      exit 1
    fi
    stratadiff_elf_machine="$(
      LC_ALL=C readelf -h "${stratadiff_source_binary}" |
        awk -F: '$1 ~ /^[[:space:]]*Machine$/ { sub(/^[[:space:]]+/, "", $2); print $2 }'
    )"
    case "${stratadiff_target}:${stratadiff_elf_machine}" in
      x86_64-unknown-linux-musl:Advanced\ Micro\ Devices\ X86-64) ;;
      aarch64-unknown-linux-musl:AArch64) ;;
      *)
        echo "unexpected ELF machine for ${stratadiff_target}: ${stratadiff_elf_machine}" >&2
        exit 1
        ;;
    esac
    ;;
  x86_64-apple-darwin|aarch64-apple-darwin)
    command -v lipo >/dev/null 2>&1 || {
      echo "lipo is required to validate a macOS release binary" >&2
      exit 1
    }
    stratadiff_macho_architecture="$(lipo -archs "${stratadiff_source_binary}")"
    case "${stratadiff_target}:${stratadiff_macho_architecture}" in
      x86_64-apple-darwin:x86_64) ;;
      aarch64-apple-darwin:arm64) ;;
      *)
        echo "unexpected Mach-O architecture for ${stratadiff_target}: ${stratadiff_macho_architecture}" >&2
        exit 1
        ;;
    esac
    ;;
esac

stratadiff_release_version=${stratadiff_release_tag#v}
stratadiff_reported_version="$("${stratadiff_source_binary}" --version)"
if [[ "${stratadiff_reported_version}" != "stratadiff ${stratadiff_release_version}" ]]; then
  echo "release binary reports ${stratadiff_reported_version}, expected stratadiff ${stratadiff_release_version}" >&2
  exit 1
fi

mkdir -p "${stratadiff_output_directory}"
stratadiff_build_info_path="$(mktemp "${TMPDIR:-/tmp}/stratadiff-build-info-XXXXXX")"
trap 'rm -f -- "${stratadiff_build_info_path}"' EXIT
"${stratadiff_source_binary}" build-info > "${stratadiff_build_info_path}"

if command -v sha256sum >/dev/null 2>&1; then
  stratadiff_lock_sha256="$(sha256sum "${stratadiff_repository_root}/Cargo.lock")"
  stratadiff_lock_sha256=${stratadiff_lock_sha256%% *}
else
  stratadiff_lock_sha256="$(shasum -a 256 "${stratadiff_repository_root}/Cargo.lock")"
  stratadiff_lock_sha256=${stratadiff_lock_sha256%% *}
fi

python3 - \
  "${stratadiff_build_info_path}" \
  "${stratadiff_release_version}" \
  "${stratadiff_release_commit}" \
  "${stratadiff_lock_sha256}" <<'PY'
import json
import pathlib
import sys

build_info_path = pathlib.Path(sys.argv[1])
expected_version = sys.argv[2]
expected_commit = sys.argv[3]
expected_lock_sha256 = sys.argv[4]
build_info = json.loads(build_info_path.read_text(encoding="utf-8"))

expected = {
    "schema": "stratadiff-build-info-v1",
    "engine_version": expected_version,
    "git_revision": expected_commit,
    "git_dirty": False,
    "cargo_lock_sha256": expected_lock_sha256,
    "build_profile": "release",
}
for key, expected_value in expected.items():
    actual_value = build_info[key]
    if actual_value != expected_value:
        raise SystemExit(
            f"build-info {key} is {actual_value!r}, expected {expected_value!r}"
        )
rustc_version = build_info["rustc_version"]
if not isinstance(rustc_version, str) or not rustc_version.startswith("rustc 1.90.0 "):
    raise SystemExit(f"build-info rustc_version is not pinned Rust 1.90.0: {rustc_version!r}")
PY

rm -- "${stratadiff_build_info_path}"
trap - EXIT
"${stratadiff_script_directory}/check-release-paths.sh" "${stratadiff_source_binary}"

stratadiff_asset_path=${stratadiff_output_directory}/${stratadiff_asset_name}
stratadiff_checksum_path=${stratadiff_asset_path}.sha256
if [[ -e "${stratadiff_asset_path}" || -L "${stratadiff_asset_path}" || \
      -e "${stratadiff_checksum_path}" || -L "${stratadiff_checksum_path}" ]]; then
  echo "release output already exists for ${stratadiff_asset_name}" >&2
  exit 1
fi
install -m 0755 "${stratadiff_source_binary}" "${stratadiff_asset_path}"

if command -v sha256sum >/dev/null 2>&1; then
  stratadiff_asset_sha256="$(sha256sum "${stratadiff_asset_path}")"
  stratadiff_asset_sha256=${stratadiff_asset_sha256%% *}
else
  stratadiff_asset_sha256="$(shasum -a 256 "${stratadiff_asset_path}")"
  stratadiff_asset_sha256=${stratadiff_asset_sha256%% *}
fi
printf '%s  %s\n' "${stratadiff_asset_sha256}" "${stratadiff_asset_name}" > "${stratadiff_checksum_path}"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    printf 'asset_name=%s\n' "${stratadiff_asset_name}"
    printf 'binary=%s\n' "${stratadiff_asset_path}"
    printf 'checksum=%s\n' "${stratadiff_checksum_path}"
    printf 'sha256=%s\n' "${stratadiff_asset_sha256}"
  } >> "${GITHUB_OUTPUT}"
fi

printf '%s\n' "${stratadiff_asset_path}"
