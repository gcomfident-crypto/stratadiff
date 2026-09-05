#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"

command -v cargo-about >/dev/null 2>&1 || {
  echo "cargo-about 0.9.2 is required; install it with: cargo install cargo-about --version 0.9.2 --locked --features cli" >&2
  exit 1
}
[[ "$(cargo-about --version)" == "cargo-about 0.9.2" ]] || {
  echo "cargo-about 0.9.2 is required to regenerate notices" >&2
  exit 1
}

cd -- "${stratadiff_repository_root}"
exec cargo about generate --locked --fail \
  --output-file THIRD_PARTY_NOTICES.txt \
  about.hbs
