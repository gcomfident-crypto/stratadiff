#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"
stratadiff_temporary_directory="$(mktemp -d "${TMPDIR:-/tmp}/stratadiff-notices-XXXXXX")"
trap 'rm -r -- "${stratadiff_temporary_directory}"' EXIT

command -v cargo-about >/dev/null 2>&1 || {
  echo "cargo-about 0.9.2 is required; install it with: cargo install cargo-about --version 0.9.2 --locked --features cli" >&2
  exit 1
}
[[ "$(cargo-about --version)" == "cargo-about 0.9.2" ]] || {
  echo "cargo-about 0.9.2 is required to verify notices" >&2
  exit 1
}

cd -- "${stratadiff_repository_root}"
cargo about generate --locked --fail \
  --output-file "${stratadiff_temporary_directory}/THIRD_PARTY_NOTICES.txt" \
  about.hbs
cmp THIRD_PARTY_NOTICES.txt "${stratadiff_temporary_directory}/THIRD_PARTY_NOTICES.txt"

if grep -Fq '<copyright holders>' THIRD_PARTY_NOTICES.txt; then
  echo "THIRD_PARTY_NOTICES.txt contains an unresolved copyright placeholder" >&2
  exit 1
fi
