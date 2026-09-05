#!/usr/bin/env bash
set -euo pipefail

stratadiff_release_binary="${1:-target/release/stratadiff}"
if [[ ! -f "${stratadiff_release_binary}" ]]; then
  echo "release binary does not exist: ${stratadiff_release_binary}" >&2
  exit 1
fi

if LC_ALL=C strings "${stratadiff_release_binary}" | grep -E '/home/[^/]+/|/Users/[^/]+/'; then
  echo "release binary contains a local build path" >&2
  exit 1
fi
