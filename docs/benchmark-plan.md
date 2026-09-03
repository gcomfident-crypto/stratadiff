# Benchmark and acceptance plan

## Corpora

1. **DiffBenchmark:** the primary Java mapping oracle, including 800 Defects4J commits and 188
   refactoring commits.
2. **ICSE 2021 differential-testing corpus:** 263,165 Java file revisions for disagreement mining
   and metamorphic tests.
3. **GumTree Simple evaluation inputs:** Defects4J, BugsInPy, and sampled GitHub revisions for
   latency and edit-size comparisons.
4. **StrataDiff adversarial corpus:** duplicate siblings, reorderings, move-plus-edit, wrapper
   insertion, split/merge/extract/inline, shadowing, overloads, syntax errors, CRLF/LF, BOM,
   non-normalized Unicode, invalid UTF-8, deep trees, and generated files.

## Required metrics

- node-mapping precision, recall, and F1 by granularity;
- perfect-diff rate;
- certified precision and certified coverage;
- ambiguity/abstention rate;
- one-to-many and cross-file precision/recall;
- replay success, which must remain 100%;
- deterministic report rate, which must remain 100%;
- p50/p95 latency, peak RSS, and report size by input size.

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
