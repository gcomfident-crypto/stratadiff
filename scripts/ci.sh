#!/usr/bin/env bash
set -euo pipefail

npm --prefix web ci
npm --prefix web test
npm --prefix web run build
if [[ -n "$(git status --porcelain --untracked-files=all -- web/dist)" ]]; then
  git status --short --untracked-files=all -- web/dist
  echo "web/dist is not the committed production build" >&2
  exit 1
fi
cargo package --list --locked --allow-dirty | grep -Fx 'web/dist/index.html'
python3 tools/resumebench-real/resumebench_real.py self-test

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --release --locked
