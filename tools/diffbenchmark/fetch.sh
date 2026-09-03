#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 TARGET_DIRECTORY" >&2
  exit 2
fi

target_directory=$1
benchmark_revision=870592abd559d0bd822a27eb5c8ea45aee47015b

if [[ -e $target_directory ]]; then
  echo "target already exists: $target_directory" >&2
  exit 1
fi

git clone --filter=blob:none --sparse \
  https://github.com/pouryafard75/DiffBenchmark.git \
  "$target_directory"
git -C "$target_directory" checkout "$benchmark_revision"
git -C "$target_directory" sparse-checkout set \
  hrd-oracle/adb-paper/literature-exp \
  info.csv \
  csv-outputs/adb-paper

echo "DiffBenchmark is pinned at $benchmark_revision in $target_directory"
