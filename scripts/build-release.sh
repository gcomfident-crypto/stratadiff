#!/usr/bin/env bash
set -euo pipefail

stratadiff_script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stratadiff_repository_root="$(cd -- "${stratadiff_script_directory}/.." && pwd -P)"
stratadiff_encoded_rustflags="${CARGO_ENCODED_RUSTFLAGS:-}"
stratadiff_c_remap_flags=()

append_rustflag() {
  if [[ -n "${stratadiff_encoded_rustflags}" ]]; then
    stratadiff_encoded_rustflags+=$'\x1f'
  fi
  stratadiff_encoded_rustflags+="$1"
}

append_path_remap() {
  append_rustflag "--remap-path-prefix=$1=$2"
  stratadiff_c_remap_flags+=("-ffile-prefix-map=$1=$2")
}

if [[ -z "${stratadiff_encoded_rustflags}" && -n "${RUSTFLAGS:-}" ]]; then
  read -r -a stratadiff_existing_rustflags <<< "${RUSTFLAGS}"
  for stratadiff_rustflag in "${stratadiff_existing_rustflags[@]}"; do
    append_rustflag "${stratadiff_rustflag}"
  done
  unset RUSTFLAGS
fi

append_path_remap "${stratadiff_repository_root}" "/stratadiff/workspace"
if [[ -n "${HOME:-}" && -d "${HOME}" ]]; then
  stratadiff_build_home="$(cd -- "${HOME}" && pwd -P)"
  append_path_remap "${stratadiff_build_home}" "/stratadiff/build-home"
fi
if [[ -n "${CARGO_HOME:-}" && -d "${CARGO_HOME}" ]]; then
  stratadiff_cargo_home="$(cd -- "${CARGO_HOME}" && pwd -P)"
  append_path_remap "${stratadiff_cargo_home}" "/stratadiff/cargo-home"
fi
if [[ -n "${RUSTUP_HOME:-}" && -d "${RUSTUP_HOME}" ]]; then
  stratadiff_rustup_home="$(cd -- "${RUSTUP_HOME}" && pwd -P)"
  append_path_remap "${stratadiff_rustup_home}" "/stratadiff/rustup-home"
fi

export CARGO_ENCODED_RUSTFLAGS="${stratadiff_encoded_rustflags}"
stratadiff_cflags="${CFLAGS:-}"
stratadiff_cxxflags="${CXXFLAGS:-}"
for stratadiff_c_remap_flag in "${stratadiff_c_remap_flags[@]}"; do
  printf -v stratadiff_escaped_c_remap_flag '%q' "${stratadiff_c_remap_flag}"
  stratadiff_cflags+="${stratadiff_cflags:+ }${stratadiff_escaped_c_remap_flag}"
  stratadiff_cxxflags+="${stratadiff_cxxflags:+ }${stratadiff_escaped_c_remap_flag}"
done
export CFLAGS="${stratadiff_cflags}"
export CXXFLAGS="${stratadiff_cxxflags}"
cd -- "${stratadiff_repository_root}"

if [[ $# -eq 0 ]]; then
  set -- --bin stratadiff
fi
exec cargo build --locked --release "$@"
