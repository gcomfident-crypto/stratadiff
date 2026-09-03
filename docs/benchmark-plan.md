# Benchmark and acceptance plan

## Corpora

1. **DiffBenchmark:** the primary Java mapping oracle. The first executable stage pins the 285-file
   literature subset at commit `870592abd559d0bd822a27eb5c8ea45aee47015b`; Defects4J and
   cross-file evaluation are later, separately reported stages.
2. **ICSE 2021 differential-testing corpus:** 263,165 Java file revisions for disagreement mining
   and metamorphic tests.
3. **GumTree Simple evaluation inputs:** Defects4J, BugsInPy, and sampled GitHub revisions for
   latency and edit-size comparisons.
4. **StrataDiff adversarial corpus:** duplicate siblings, reorderings, move-plus-edit, wrapper
   insertion, split/merge/extract/inline, shadowing, overloads, syntax errors, CRLF/LF, BOM,
   non-normalized Unicode, invalid UTF-8, deep trees, and generated files.

## Current status

- [x] The pinned 285-case DiffBenchmark literature intra-file stage is complete. The
  [official v5 report](../benchmarks/diffbenchmark-literature-evaluation-v5.json) records 283
  evaluated cases, one known malformed oracle, one known malformed source, zero unexpected errors,
  and `benchmarkComplete: true`.
- [ ] Ambiguity/abstention evaluation remains incomplete. The flattened ambiguity list is only the
  edge union of explicit `possible_pairs`, not one jointly selectable mapping, and
  `symbolic_abstention` scopes contribute no pair candidates. This run produced no scorable
  projected candidate, so zero measured coverage is not evidence that the engine emitted no
  ambiguity or that ambiguity handling is ineffective.
- [ ] Cross-tool comparison against GumTree and RefactoringMiner remains incomplete.
- [ ] Defects4J and cross-file evaluation remain later, separately reported stages.

The completed literature stage does not mark the broader benchmark and acceptance plan complete.

## Required metrics

- node-mapping precision, recall, and F1 by granularity;
- perfect-diff rate;
- certified precision and certified coverage;
- ambiguity/abstention rate;
- one-to-many and cross-file precision/recall;
- replay success, which must remain 100%;
- deterministic report rate, which must remain 100%;
- p50/p95 latency, peak RSS, and report size by input size.

DiffBenchmark results are split into program elements versus fine mappings and singleton versus
multi-mapping components. Report micro and per-case macro precision/recall/F1, perfect-case rate,
parser-taxonomy coverage, abstention, ambiguity-candidate coverage and expansion, and multi-group
overclaim. Predictions outside the fixed parser-adapter universe are unscored and counted. Do not
report true-negative accuracy because the non-edge universe is not well-defined.

Edit-script length is reported but never used as a proxy for mapping correctness.

## Acceptance gates

- No relation marked `byte_equal` or `syntax_equal` may fail independent verification.
- Mutation of an input hash, patch range, relation endpoint, predicate, ambiguity, change, parser
  manifest, or summary must make verification fail.
- Duplicate-code tests may reduce coverage but may not manufacture a one-to-one identity.
- Parser errors and unsupported grammars must produce a clear diagnostic.
- A generated file with 10,000 duplicate nodes must not construct a quadratic candidate graph.
- Deep inputs must stop at an explicit resource limit rather than overflow the process stack.
- Every release records comparison versions, grammar versions, hardware, corpus commit, and complete
  command lines.
