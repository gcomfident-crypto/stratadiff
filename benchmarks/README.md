# Benchmark artifacts

## Rebase-aware real review oracle

[`resumebench-real-v1/`](resumebench-real-v1/) freezes the previously rejected Gerrit base-drift
history as a four-snapshot oracle for the current policy. Its independent verifier checks 4 exact
identity carries, 1 non-interacting four-way byte replay, 2 needs-review files named by Gerrit's
submission record, and 2 retired checkpoint changes. The checked-in clean-release evaluation has
zero disagreement with that 5/2 partition. This is one selected correctness case, not evidence of
prevalence, reviewer-time savings, defect recall, or semantic safety.

## Real review-history diagnostic

[`resumebench-real-v0/`](resumebench-real-v0/) pins five public Gerrit review histories: four
same-base checkpoint transitions and one rebase/base-drift case that the v0 policy must reject. Its
independent Python/Git oracle records the complete checkpoint, head, and checkpoint-to-head
identity sets plus blob SHA-256 evidence. The checked-in evaluation passes all five cases with zero
false carries and zero false invalidations under that historical policy.

The evaluation predates non-interacting four-way byte replay. The same base-drift case now has a
separate v1 oracle and evaluation; v0 remains unchanged as historical policy evidence. Neither
frozen result estimates reviewer-time savings or defect recall; see the
[`ResumeBench-Real v0` dataset card](resumebench-real-v0/README.md).

## Exact-review-resume safety seed

[`resumebench-seed-v1.json`](resumebench-seed-v1.json) describes controlled three-snapshot Git
histories for the original checkpoint policy: rewritten history, changed bytes, changed paths and
modes, new and retired changes, exact deletions, and parser-unsupported content. The gate requires
every current change to be accounted for exactly once and covers complete Git change-identity
matching. It does not yet serve as the oracle for base-drift replay. See the
[`ResumeBench` dataset card](resumebench/README.md) and run `cargo test --test resumebench`.

This seed tests a factual invalidation rule. It does not show that the checkpoint was actually
reviewed, that unchanged files are semantically safe, or that developers save time in practice.

## Review-residue safety seed

[`reviewbench-seed-v1.json`](reviewbench-seed-v1.json) is the first checked-in product-facing
classification corpus. It pairs whitespace-only controls with behavior-sensitive mutations across
supported languages and includes adversarial Python debug-f-string, Rust/C stringification, and
HTML rendering cases. Evidence class and attention priority are tested separately; the gate is
deliberately asymmetric, and every behavior-sensitive change must retain `review_first` priority.
Passing does not establish production recall or reviewer-time savings. See the
[`reviewbench` dataset card](reviewbench/README.md) and run `cargo test --test reviewbench`.

The planned real-review track uses the CC-BY-4.0 CodeReviewer/FSE'22 refinement corpus and a
counterbalanced human study. That work is specified in
[`docs/product-strategy.md`](../docs/product-strategy.md); it has not yet been run, so no product
value claim is attached to this seed result.

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
