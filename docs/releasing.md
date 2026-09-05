# Release procedure

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

Build distributable binaries through the path-remapping wrapper and check the result:

```console
scripts/build-release.sh --workspace
scripts/check-release-paths.sh target/release/stratadiff
```

The repository does not currently publish binary archives. Before adding them, generate and ship
third-party notices for the Rust dependency graph in addition to the embedded Workbench notices.
