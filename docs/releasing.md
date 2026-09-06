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
cargo info stratadiff-core@0.4.0

cargo publish --package stratadiff-verifier --locked
cargo info stratadiff-verifier@0.4.0

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
clean checkout of the intended commit, use Python 3 and an authenticated GitHub identity with
repository Administration write access to verify the service-side policy immediately before
creating the tag.
GitHub omits ruleset bypass actors from this API response unless the caller can write the ruleset,
so a read-only administration token is insufficient for this fail-closed check. Missing, null, or
nonempty `bypass_actors` all fail closed:

```console
scripts/check-release-repository-policy.sh gcomfident-crypto/stratadiff
scripts/ci.sh
git tag -a v0.4.0 -m "StrataDiff v0.4.0"
git push origin v0.4.0
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

If any prepublication build, checksum, inventory, or signature check fails, the release remains a
draft. A rerun may replace only the twelve expected assets. Any unrelated filesystem entry
represented in the download, including a directory or symbolic link in local verification,
deliberately blocks publication; inspect an unexpected remote asset and remove it explicitly with
`gh release delete-asset TAG ASSET` before rerunning. After publication, the workflow requires the
release API to report a stable immutable release before marking it latest; otherwise it tries to
remove the invalid release while preserving the protected tag, then fails. The installer repeats
the stable and immutable release check before downloading assets. A previously published release is
never overwritten by this workflow.

## Install and verify a released binary

The normal user path downloads the installer from the same immutable version tag, then lets it
select and verify the platform asset:

```bash
(
  set -e
  installer="$(mktemp)"
  trap 'rm -f "$installer"' EXIT
  gh api --hostname github.com -H 'Accept: application/vnd.github.raw+json' \
    'repos/gcomfident-crypto/stratadiff/contents/scripts/install-release.sh?ref=v0.4.0' \
    > "$installer"
  test -s "$installer"
  bash "$installer" v0.4.0
)
```

After that block succeeds:

```console
export PATH="$HOME/.local/bin:$PATH"
stratadiff build-info
stratadiff resume https://github.com/OWNER/REPOSITORY/pull/123
```

Use the actual immutable release tag instead of copying the example blindly. The bootstrap step
trusts GitHub's authenticated contents response for that protected tag and downloads it completely
before execution; the binary verification does not retroactively attest the installer itself. The
installer accepts only stable semantic-version tags, fixes the release repository and signer
workflow, fully dereferences the tag before download and again before installation, verifies the
checksum and bundled GitHub attestation, checks the binary's reported version, and installs through
a same-directory atomic rename. An existing binary is left untouched on every validation failure.
`scripts/test-install-release.sh` exercises the four platform mappings and principal failure modes;
the release-contract CI job runs it under both Ubuntu and macOS system tooling.

Select the asset for the current kernel and CPU, then download the binary, checksum, and provenance
bundle. For example, on Linux x86-64:

```console
tag=v0.4.0
asset=stratadiff-linux-x86_64
source_digest="$(gh api --hostname github.com \
  "repos/gcomfident-crypto/stratadiff/commits/$tag" --jq .sha)"
gh release download "$tag" -R github.com/gcomfident-crypto/stratadiff \
  -p "$asset" -p "$asset.sha256" -p "$asset.intoto.jsonl"
sha256sum -c "$asset.sha256"
gh attestation verify "$asset" \
  --hostname github.com \
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
or deletion of `v*` tags. `scripts/check-release-repository-policy.sh` verifies both controls with a
repository Administration write credential immediately before tag creation; GitHub requires this
to expose the complete bypass-actor list, and its default Actions token cannot call the
immutable-release settings endpoint. The workflow rechecks the remote tag in the same shell step
that publishes the draft, verifies the resulting release's public `isImmutable` state, and refuses
to modify an already published release. The repository controls then protect the tag and assets
afterward.
