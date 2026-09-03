# DiffBenchmark adapter workspace

The benchmark dataset is fetched on demand and is not vendored into StrataDiff.

```console
tools/diffbenchmark/fetch.sh /absolute/path/to/DiffBenchmark
```

The fetch script pins commit `870592abd559d0bd822a27eb5c8ea45aee47015b` and checks out only the
285 literature `GOD.json` files plus the exact metadata inputs. A clean checkout is about 43 MB;
tool-output directories are intentionally excluded. Only `GOD.json` data is ground truth.
Directories named RMD, GTG, GTS, IJM, or MTD contain outputs from evaluated tools and must not be
treated as oracle labels.

Audit the pinned checkout before attempting an evaluation:

```console
cargo run --release --bin stratadiff-benchmark -- /absolute/path/to/DiffBenchmark
```

The audit is strict by default. At the pinned revision, the literature corpus contains 285 oracle
files; 284 are valid JSON. The Hive `TestInputOutputFormat` oracle contains raw newlines inside JSON
strings and therefore makes the command exit unsuccessfully after printing the complete audit.
`--allow-invalid` changes only the exit status, never the report or the input bytes. A future
normalization mode must be explicit, deterministic, digest-guarded, and recorded in its manifest.

The evaluator must convert JDT UTF-16 code-unit offsets to Tree-sitter UTF-8 byte offsets from the
original source and map both parsers into a shared node-role taxonomy. Raw span equality across the
two coordinate systems is invalid. Stage-one scoring covers intra-file edges only; cross-file
oracle edges remain reported but unscored until repository mode exists.
