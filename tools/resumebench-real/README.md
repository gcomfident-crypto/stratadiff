# ResumeBench-Real tooling

`resumebench_real.py` owns five explicit operations for the real-history benchmark:

- `verify-provenance`: bind every frozen patch set and review event to the public Gerrit API;
- `materialize`: fetch pinned public Gerrit refs into a local non-shallow thin repository;
- `generate`: build independent raw-Git oracle files;
- `verify`: recompute and compare every checked-in oracle without writing;
- `evaluate`: compare the `stratadiff review` CLI output with those frozen oracles.

The oracle is deliberately Python standard-library code and Git plumbing. It must not import the
StrataDiff package, call the StrataDiff binary, or deserialize production Rust types while
generating expectations. Only `evaluate` launches the system under test.
The evaluator accepts only a clean release build with complete `stratadiff build-info` provenance
as a complete benchmark run.

See the [dataset card](../../benchmarks/resumebench-real-v0/README.md) for commands, provenance,
scope, and claim limits.
