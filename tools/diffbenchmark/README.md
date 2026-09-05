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
scripts/build-release.sh --bin stratadiff-benchmark
target/release/stratadiff-benchmark /absolute/path/to/DiffBenchmark
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

Materialize the exact before/after revisions without placing third-party sources in this repository:

```console
scripts/build-release.sh --bin stratadiff-materialize
target/release/stratadiff-materialize \
  /absolute/path/to/DiffBenchmark \
  /absolute/path/to/materialized \
  --repository-map tools/diffbenchmark/repository-mirrors.json \
  --source-backend git \
  --git-cache /absolute/path/to/git-cache \
  --git-transport https
```

The v3 manifest uses neutral `.source` filenames, hashes every byte sequence, and checkpoints each
completed case atomically. A complete validated manifest can restore missing source files directly
from the Git object cache without querying repository metadata again. Its canonical copy is
[`benchmarks/diffbenchmark-literature-manifest-v3.json`](../../benchmarks/diffbenchmark-literature-manifest-v3.json).

Bootstrap the pinned JDT dependencies and run the complete evaluator without `--limit`:

```console
JAVA_HOME=/absolute/path/to/jdk-17 \
  tools/diffbenchmark/jdt/bootstrap.sh /absolute/path/to/jdt-cache

scripts/build-release.sh --bin stratadiff-evaluate
target/release/stratadiff-evaluate \
  /absolute/path/to/DiffBenchmark \
  /absolute/path/to/materialized \
  --jdt-cache /absolute/path/to/jdt-cache \
  --java-executable /absolute/path/to/jdk-17/bin/java \
  --require-complete \
  --output /absolute/path/to/evaluation.json
```

The pinned inputs contain one malformed Hive oracle and one syntactically malformed Alluxio source;
both are identified by path, revision, and content digest. A complete run therefore evaluates and
independently verifies 283 cases, records those two exclusions separately, and has zero unexpected
case errors. `benchmarkComplete` is a provenance and execution-completeness gate, not an accuracy
threshold.
