# Review Ledger v1 offline runner

`runner.py` materializes deterministic webhook bodies and isolated Git repositories from the
symbolic fixtures in `benchmarks/review-ledger-v1/manifest.json`. Its oracle is implemented with the
Python standard library and does not import StrataDiff's Rust reducers. Product observations come
only from the public `stratadiff` CLI.

Build the binary, run the unit tests for the independent oracle, and execute the benchmark:

```console
cargo build --bin stratadiff
python3 -m unittest tools/review-ledger-v1/test_runner.py
python3 tools/review-ledger-v1/runner.py
```

The runner writes `target/review-ledger-v1/result.json`, including the manifest SHA-256 and the
binary's `build-info`, prints one `PASS`, `FAIL`, or `SKIP` line per manifest case, and exits nonzero
when a case or control fails. All 20 manifest cases currently run; `SKIP` remains available for a
selected control whose prerequisite case was not run, and `--strict-skips` makes that nonzero too.
`--case CASE_ID` selects cases, `--list` shows support, and `--keep-workdir PATH` retains generated
webhook, ledger, Git, ownership, and passport fixtures for inspection.

The separate `passport-tamper-detected` control first verifies an unchanged signed passport, changes
its body without resigning it, and requires verification to fail on the body digest before offline
recomputation. It is a runner control rather than a manifest case and is reported separately.

The result is a controlled conformance artifact, not a safety or performance claim. A passing case
means both that the independent oracle agrees with the checked-in expectation and that the CLI
observation agrees with that oracle for the materialized fixture.
