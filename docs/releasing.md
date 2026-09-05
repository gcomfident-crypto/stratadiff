# Release procedure

StrataDiff has a fail-closed GitHub binary release workflow, but adding the workflow does not mean
that a release already exists. A maintainer must run the repository gate and push an exact stable
version tag before users can download a binary.

Run the complete repository gate from a clean checkout before publishing:

```console
scripts/ci.sh
```

The gate packages and verifies all three crates together. Cargo's temporary registry makes the
unpublished workspace dependencies available while it verifies the package tarballs, so this is
stronger than `cargo package --list` or `cargo package --no-verify`.

For the first crates.io release, publish the dependency graph in order and wait for each exact
version to become visible before continuing:

```console
cargo publish --package stratadiff-core --locked
cargo info stratadiff-core@0.3.0

cargo publish --package stratadiff-verifier --locked
cargo info stratadiff-verifier@0.3.0

cargo publish --package stratadiff --locked
```

Do not use `cargo publish --workspace` for the first release. The root and verifier manifests use
exact dependency versions, and a newly published dependency may not be immediately visible through
the crates.io index. A later release may advance only after the preceding `cargo info` command
succeeds.

## Binary release gate

Build a local distributable binary through the path-remapping wrapper and check the result:

```console
scripts/generate-third-party-notices.sh
git diff --exit-code -- THIRD_PARTY_NOTICES.txt
scripts/build-release.sh --workspace
scripts/check-release-paths.sh target/release/stratadiff
```

The notice scripts require exactly `cargo-about 0.9.2`. `THIRD_PARTY_NOTICES.txt` covers the locked
Rust build graph for the four supported release targets and is embedded in the executable; verify it
at runtime with `stratadiff licenses`. The Workbench's JavaScript notices remain embedded separately.

The tag must be an exact stable `vMAJOR.MINOR.PATCH` matching the root Cargo package version. From a
clean checkout of the intended commit:

```console
scripts/ci.sh
git tag v0.3.0
git push origin v0.3.0
```

Use the actual manifest version instead of copying the example blindly. The tag push starts
`.github/workflows/release.yml`. It creates a draft and produces this closed asset set:

| Runtime | Rust target | Release binary |
| --- | --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-musl` | `stratadiff-linux-x86_64` |
| Linux ARM64 | `aarch64-unknown-linux-musl` | `stratadiff-linux-aarch64` |
| macOS x86-64 | `x86_64-apple-darwin` | `stratadiff-macos-x86_64` |
| macOS ARM64 | `aarch64-apple-darwin` | `stratadiff-macos-arm64` |

Each binary has a same-name `.sha256` record and `.intoto.jsonl` GitHub build-provenance bundle.
Linux artifacts are musl-linked so they do not inherit the GitHub runner's glibc floor.

Every matrix job builds one native binary, checks its embedded version, Git commit, clean-tree bit,
Cargo.lock digest, release profile, Rust 1.90.0 toolchain, local-path removal, and embedded notices,
then attests and uploads those exact local bytes. There is no later rebuild and no artifact relay
between the build and upload steps. The final job downloads the complete draft, rejects missing or
unexpected files, checks every digest, and verifies every bundle against all of the following before
making the release public:

- this repository;
- `.github/workflows/release.yml` as the signer workflow;
- the exact tag ref;
- the exact release commit recorded before any build started;
- a GitHub-hosted rather than self-hosted runner.

Immediately before publication, the workflow dereferences the remote tag again and requires it to
resolve to that same release commit. Checkout credentials are not persisted, and permissions are
scoped per job; `GH_TOKEN` is exposed only to the individual release API steps.

If any build, checksum, inventory, or signature check fails, the release remains a draft. A rerun may
replace only the twelve expected assets. Any unrelated filesystem entry represented in the download,
including a directory or symbolic link in local verification, deliberately blocks publication;
inspect an unexpected remote asset and remove it explicitly with `gh release delete-asset TAG ASSET`
before rerunning. A previously published release is never overwritten by this workflow.

## Install and verify a released binary

Select the asset for the current kernel and CPU, then download the binary, checksum, and provenance
bundle. For example, on Linux x86-64:

```console
tag=v0.3.0
asset=stratadiff-linux-x86_64
source_digest="$(gh api "repos/gcomfident-crypto/stratadiff/commits/$tag" --jq .sha)"
gh release download "$tag" -R gcomfident-crypto/stratadiff \
  -p "$asset" -p "$asset.sha256" -p "$asset.intoto.jsonl"
sha256sum -c "$asset.sha256"
gh attestation verify "$asset" \
  --bundle "$asset.intoto.jsonl" \
  --repo gcomfident-crypto/stratadiff \
  --source-ref "refs/tags/$tag" \
  --source-digest "$source_digest" \
  --signer-workflow gcomfident-crypto/stratadiff/.github/workflows/release.yml \
  --deny-self-hosted-runners
mkdir -p "$HOME/.local/bin"
install -m 0755 "$asset" "$HOME/.local/bin/stratadiff"
stratadiff build-info
```

Use `shasum -a 256 -c "$asset.sha256"` on macOS. The macOS binaries currently have neither an
Apple Developer ID signature nor notarization, so Gatekeeper behavior is still a documented
distribution limitation. A checksum proves byte integrity; the provenance bundle additionally
binds those bytes to this repository's release workflow.

## GitHub CLI extension boundary

The binary release above installs the native `stratadiff` command. It does **not** make this
repository remotely installable as `gh stratadiff`.

GitHub's extension contract requires a dedicated repository whose name begins with `gh-`; its root
executable must match that repository name, or its release must contain precompiled assets named
with the `gh-<name>-<os>-<arch>` convention. See GitHub's
[extension authoring documentation](https://docs.github.com/en/github-cli/github-cli/creating-github-cli-extensions)
and [`gh extension install` reference](https://cli.github.com/manual/gh_extension_install).
This repository is named `stratadiff`, while its extension launcher lives below
`extensions/gh-stratadiff`. Therefore a documented command such as
`gh extension install gcomfident-crypto/stratadiff` would be false and is intentionally not offered.

Remote one-command extension installation requires a separate `gh-stratadiff` repository (or a
repository rename and corresponding product migration), release assets such as
`gh-stratadiff-linux-amd64` and `gh-stratadiff-darwin-arm64`, and an independently tested update
path. Until that distribution decision is made, local extension installation from
`extensions/gh-stratadiff` remains the truthful path.

Before the first public release, enable immutable releases and a tag ruleset that prevents updates
or deletion of `v*` tags. The workflow rechecks the remote tag in the same shell step that publishes
the draft and refuses to modify an already published release; the repository controls close the
remaining service-side gap and protect the tag and assets afterward.
