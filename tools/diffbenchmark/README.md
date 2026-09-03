# DiffBenchmark adapter workspace

The benchmark dataset is fetched on demand and is not vendored into StrataDiff.

```console
tools/diffbenchmark/fetch.sh /absolute/path/to/DiffBenchmark
```

The fetch script pins commit `870592abd559d0bd822a27eb5c8ea45aee47015b`. Only `GOD.json` data is
ground truth. Directories named RMD, GTG, GTS, IJM, or MTD contain outputs from evaluated tools and
must not be treated as oracle labels.

The evaluator must convert JDT UTF-16 code-unit offsets to Tree-sitter UTF-8 byte offsets from the
original source and map both parsers into a shared node-role taxonomy. Raw span equality across the
two coordinate systems is invalid.
