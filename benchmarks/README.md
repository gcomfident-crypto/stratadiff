# Benchmark artifacts

[`diffbenchmark-literature-manifest-v3.json`](diffbenchmark-literature-manifest-v3.json) is the
canonical source manifest for the 285-case DiffBenchmark literature subset pinned at revision
`870592abd559d0bd822a27eb5c8ea45aee47015b`. It records repository revisions, paths, and content
digests without vendoring third-party source files.

Canonical manifest BLAKE3:

```text
0012eecb59360ef45e9ccc2ecaa9c11ca1387bfa6c391238d0301a84ee44d9d3
```

`stratadiff-evaluate` requires this digest before a run can report `benchmarkComplete: true`.

## Official literature result

- [DiffBenchmark literature evaluation v6](diffbenchmark-literature-evaluation-v6.json)
- [Prior v5 evaluation](diffbenchmark-literature-evaluation-v5.json)
- [Prior v4 evaluation](diffbenchmark-literature-evaluation-v4.json)
- [Prior v3 evaluation](diffbenchmark-literature-evaluation-v3.json)
- [SHA-256 checksums](SHA256SUMS)

The v6 artifact is the latest first-party StrataDiff result for the complete pinned literature
subset. Its `benchmarkComplete: true` status records the expected 283 evaluated cases, one known
malformed oracle, one known malformed source, and zero unexpected errors, together with complete
engine provenance, a verified JDT cache, the canonical manifest, the full 285-case selection, and
the standalone verifier's deterministic work usage. It is an evaluation artifact for this fixed
corpus and engine, not a cross-tool comparison.

Interpret the result within these limits:

- the adapter projects the edge union of explicit `possible_pairs`, not one jointly selectable
  mapping, while symbolic abstentions contribute no pair candidates; this run produced no scorable
  projected ambiguity candidate, so it does not establish ambiguity coverage and must not be read
  as evidence that the engine emitted no ambiguity groups;
- GumTree and RefactoringMiner comparison runs are not included;
- Defects4J and cross-file evaluation remain future stages; and
- relation scores apply only to the fixed parser-adapter universe. Excluded oracle relations and
  out-of-universe predictions are reported separately, while latency and report-size quantiles are
  corpus-wide rather than stratified by input size.
