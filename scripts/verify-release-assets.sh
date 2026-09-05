#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 && $# -ne 5 ]]; then
  echo "usage: scripts/verify-release-assets.sh DIRECTORY [REPOSITORY SOURCE_REF SOURCE_DIGEST SIGNER_WORKFLOW]" >&2
  exit 2
fi

stratadiff_asset_directory=$1
if [[ ! -d "${stratadiff_asset_directory}" ]]; then
  echo "release asset directory does not exist: ${stratadiff_asset_directory}" >&2
  exit 1
fi

stratadiff_asset_names=(
  stratadiff-linux-aarch64
  stratadiff-linux-x86_64
  stratadiff-macos-arm64
  stratadiff-macos-x86_64
)
stratadiff_expected_files=()
for stratadiff_asset_name in "${stratadiff_asset_names[@]}"; do
  stratadiff_expected_files+=(
    "${stratadiff_asset_name}"
    "${stratadiff_asset_name}.intoto.jsonl"
    "${stratadiff_asset_name}.sha256"
  )
done

stratadiff_expected_inventory="$(printf '%s\n' "${stratadiff_expected_files[@]}" | LC_ALL=C sort)"
stratadiff_actual_inventory="$(
  find "${stratadiff_asset_directory}" -mindepth 1 -maxdepth 1 -exec basename {} \; |
    LC_ALL=C sort
)"
if [[ "${stratadiff_actual_inventory}" != "${stratadiff_expected_inventory}" ]]; then
  echo "release asset inventory is incomplete or contains unexpected files" >&2
  diff -u \
    <(printf '%s\n' "${stratadiff_expected_inventory}") \
    <(printf '%s\n' "${stratadiff_actual_inventory}") >&2 || true
  exit 1
fi

for stratadiff_asset_name in "${stratadiff_asset_names[@]}"; do
  stratadiff_asset_path=${stratadiff_asset_directory}/${stratadiff_asset_name}
  stratadiff_checksum_path=${stratadiff_asset_path}.sha256
  stratadiff_bundle_path=${stratadiff_asset_path}.intoto.jsonl
  [[ -f "${stratadiff_asset_path}" && ! -L "${stratadiff_asset_path}" && \
     -s "${stratadiff_asset_path}" ]] || {
    echo "release binary is not a nonempty regular file: ${stratadiff_asset_name}" >&2
    exit 1
  }
  [[ -f "${stratadiff_checksum_path}" && ! -L "${stratadiff_checksum_path}" && \
     -s "${stratadiff_checksum_path}" ]] || {
    echo "release checksum is not a nonempty regular file: ${stratadiff_asset_name}.sha256" >&2
    exit 1
  }
  [[ -f "${stratadiff_bundle_path}" && ! -L "${stratadiff_bundle_path}" && \
     -s "${stratadiff_bundle_path}" ]] || {
    echo "release attestation bundle is not a nonempty regular file: ${stratadiff_asset_name}.intoto.jsonl" >&2
    exit 1
  }

  stratadiff_checksum_line="$(< "${stratadiff_checksum_path}")"
  if [[ ! "${stratadiff_checksum_line}" =~ ^[0-9a-f]{64}[[:space:]][[:space:]]${stratadiff_asset_name}$ ]]; then
    echo "invalid checksum record: ${stratadiff_asset_name}.sha256" >&2
    exit 1
  fi
  stratadiff_expected_sha256=${stratadiff_checksum_line%% *}
  if command -v sha256sum >/dev/null 2>&1; then
    stratadiff_actual_sha256="$(sha256sum "${stratadiff_asset_path}")"
    stratadiff_actual_sha256=${stratadiff_actual_sha256%% *}
  else
    stratadiff_actual_sha256="$(shasum -a 256 "${stratadiff_asset_path}")"
    stratadiff_actual_sha256=${stratadiff_actual_sha256%% *}
  fi
  if [[ "${stratadiff_actual_sha256}" != "${stratadiff_expected_sha256}" ]]; then
    echo "checksum mismatch: ${stratadiff_asset_name}" >&2
    exit 1
  fi

  if [[ $# -eq 5 ]]; then
    command -v gh >/dev/null 2>&1 || {
      echo "gh is required for attestation verification" >&2
      exit 1
    }
    gh attestation verify "${stratadiff_asset_path}" \
      --bundle "${stratadiff_bundle_path}" \
      --repo "$2" \
      --source-ref "$3" \
      --source-digest "$4" \
      --signer-workflow "$5" \
      --deny-self-hosted-runners >/dev/null
  fi
done
