#!/usr/bin/env bash
set -euo pipefail

npm --prefix web ci
npm --prefix web run notices:check
npm --prefix web test
npm --prefix web run build
cmp web/public/THIRD_PARTY_NOTICES.txt web/dist/THIRD_PARTY_NOTICES.txt
if [[ -n "$(git status --porcelain --untracked-files=all -- web/dist)" ]]; then
  git status --short --untracked-files=all -- web/dist
  echo "web/dist is not the committed production build" >&2
  exit 1
fi
cargo package --workspace --locked
cargo package --package stratadiff --list --locked | grep -Fx 'web/dist/index.html'
python3 tools/resumebench-real/resumebench_real.py self-test
python3 benchmarks/resumebench-real-v1/verify.py self-test
python3 tools/resumebench-github-live/resumebench_github_live.py self-test
python3 tools/resumebench-github-live/resumebench_github_live.py verify-bundle \
  --manifest benchmarks/resumebench-github-live-v1/manifest.json
python3 scripts/demo_review_coverage.py --self-test
python3 -B tools/reviewer-value-v1/reviewer_value_v1.py verify
bash extensions/gh-stratadiff/tests/resume_test.sh
python3 -B tools/reviewer-study-v1/reviewer_study_v1.py self-test

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked --no-fail-fast
scripts/build-release.sh --workspace
scripts/check-release-paths.sh target/release/stratadiff
python3 -B tools/review-ledger-v1/runner.py --strict-skips
