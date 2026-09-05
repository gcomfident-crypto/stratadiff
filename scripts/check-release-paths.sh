#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"
stratadiff_release_binary="${1:-target/release/stratadiff}"
if [[ ! -f "${stratadiff_release_binary}" ]]; then
  echo "release binary does not exist: ${stratadiff_release_binary}" >&2
  exit 1
fi

if LC_ALL=C strings "${stratadiff_release_binary}" | grep -E '/home/[^/]+/|/Users/[^/]+/'; then
  echo "release binary contains a local build path" >&2
  exit 1
fi

if ! "${stratadiff_release_binary}" licenses | cmp - "${stratadiff_repository_root}/THIRD_PARTY_NOTICES.txt"; then
  echo "release binary does not contain the current third-party notices" >&2
  exit 1
fi
