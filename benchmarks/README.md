# Benchmark artifacts

`diffbenchmark-literature-manifest-v3.json` is the canonical source manifest for the 285-case
DiffBenchmark literature subset pinned at revision
`870592abd559d0bd822a27eb5c8ea45aee47015b`. It records repository revisions, paths, and content
digests without vendoring third-party source files.

Canonical manifest BLAKE3:

```text
0012eecb59360ef45e9ccc2ecaa9c11ca1387bfa6c391238d0301a84ee44d9d3
```

`stratadiff-evaluate` requires this digest before a run can report `benchmarkComplete: true`.
