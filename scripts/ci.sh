#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --locked
