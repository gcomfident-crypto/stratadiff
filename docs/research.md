# Structured code differencing survey

Survey date: 2026-09-04. This document distinguishes mapping accuracy, edit-script size,
human-readable presentation, and replayability; they are different objectives.

## Executive finding

No snapshot-only differencer can guarantee the developer's true edit history. Two identical blocks
can be exchanged, independently deleted and recreated, or left untouched while producing the same
before and after snapshots. More generally, semantic program equivalence is undecidable, and tree
edit distance with unrestricted moves has hard optimization cases.

The strongest practical direction is therefore evidence-bearing and abstention-aware:

- make the byte transformation exactly replayable;
- verify every equality predicate independently;
- separate structural equivalence from historical identity;
- preserve ties rather than select an arbitrary AST mapping;
- add language semantic evidence incrementally.

## Leading systems

| System | Primary objective and approach | Strong point | Important limitation | License |
|---|---|---|---|---|
| [RefactoringMiner / ASTDiff](https://github.com/tsantalis/RefactoringMiner) | Refactoring-aware, multi-stage AST matching using declarations, references, detected refactorings, and local GumTree matching | Current public accuracy leader on the Java DiffBenchmark; supports cross-file and multi-mapping cases | Published accuracy evidence is primarily Java; still not perfect, especially around repeated statements and difficult nested changes | MIT |
| [GumTree 4](https://github.com/GumTreeDiff/gumtree) | Hash-based top-down anchors followed by bottom-up recovery and edit-script generation | Fast, broadly reused baseline; GumTree Simple greatly improves speed and script size | Greedy one-to-one mapping, duplicate-code ambiguity, weak semantic roles, and short-script bias | LGPL-3.0 |
| [GumTree-Spoon](https://github.com/SpoonLabs/gumtree-spoon-ast-diff) | GumTree over Spoon's higher-level Java model | Rich Java model and convenient analysis API | Java-only and inherits matcher ambiguity; its documentation explicitly notes that an oracle may not be unique | Apache-2.0 |
| [Difftastic](https://github.com/Wilfred/difftastic) | Finds a minimum-cost path over syntax-tree positions using Dijkstra | Excellent human-facing structural display and broad Tree-sitter language coverage | Optimizes its display cost, not developer intent; no applicable patch or consumable mapping certificate | MIT |
| [SemanticDiff](https://semanticdiff.com/docs/what-is-semanticdiff/) | Proprietary language-specific parsing and invariance rules | Strong review UI and useful suppression of non-semantic syntax | Closed engine, no public accuracy or speed benchmark, text fallback on parse failure | Proprietary |
| [diffsitter](https://github.com/afnanenayet/diffsitter) | Extracts Tree-sitter leaves, removes whitespace, and applies Myers | Small, understandable syntax-aware token diff | Tree hierarchy does not drive matching; project describes itself as not production-ready | MIT |
| [Graphtage](https://github.com/trailofbits/graphtage) | Typed IR, ordered edit distance, and bipartite matching for unordered structures | Principled optimal-cost diff for JSON/XML/YAML/TOML and related data | Its guarantee is relative to its cost model; unordered matching can be expensive and it is not a source-code semantic differ | LGPL-3.0 |
| [Tree-sitter](https://github.com/tree-sitter/tree-sitter) | Incremental concrete syntax parsing with changed-range support | Fast, lossless-enough frontend with a large grammar ecosystem | A parser, not a node matcher; changed ranges do not establish move, rename, or identity | MIT |

Historical baselines include
[ChangeDistiller](https://doi.org/10.1109/TSE.2007.70731),
[MTDIFF](https://doi.org/10.1145/2970276.2970315), and
[IJM](https://doi.org/10.1109/ICSME.2018.00036). They introduced useful change taxonomies,
move-oriented recovery, and Java-name-aware partitioning, but are no longer the best maintained
general foundation.

## Best public accuracy evidence

The 2025 RefactoringMiner AST-diff study and
[DiffBenchmark](https://github.com/pouryafard75/DiffBenchmark) provide the strongest directly
annotated comparison found in this survey. DiffBenchmark contains 800 Defects4J commits and 188
refactoring commits. Reported fine-grained overall precision/recall are:

| Matcher | Precision | Recall |
|---|---:|---:|
| RefactoringMiner 3.0 ASTDiff | 99.7% | 99.3% |
| GumTree Simple | 95.2% | 90.0% |

RefactoringMiner reports a perfect-diff rate of 82.9% overall and 70.2% on the refactoring subset,
which is excellent but materially below 100%. On Defects4J, its reported median time is 22.5 ms,
compared with 8.75 ms for GumTree Simple. These figures show the practical accuracy/latency tradeoff
and also why a system should expose abstention instead of hiding the remaining errors.

Sources:

- Alikhanifard and Tsantalis, “Refactoring-aware Abstract Syntax Tree Differencing,” TOSEM,
  [DOI 10.1145/3696002](https://doi.org/10.1145/3696002),
  [open manuscript](https://users.encs.concordia.ca/~nikolaos/publications/TOSEM_2024.pdf).
- Frick et al., “GumTree Simple: A Fine-grained, Accurate and Scalable Source Code Differencing
  Approach,” ICSE 2024,
  [DOI 10.1145/3597503.3639148](https://doi.org/10.1145/3597503.3639148),
  [open manuscript](https://hal.science/hal-04855170v1/file/GumTree_simple__fine_grained__accurate_and_scalable_source_differencing.pdf).
- Fan et al., “A Differential Testing Approach for Evaluating Abstract Syntax Tree Mapping
  Algorithms,” ICSE 2021,
  [DOI 10.1109/ICSE43902.2021.00108](https://doi.org/10.1109/ICSE43902.2021.00108),
  [preprint](https://arxiv.org/abs/2103.00141).

GumTree Simple reports a 50–281x matching-stage speedup over the older `opt-1000` configuration in
its evaluated settings and substantially shorter scripts. That makes it the speed baseline, but
shorter edit scripts are not evidence that every selected mapping reflects the true change.

## Measured StrataDiff result

The provenance-complete v3 run on DiffBenchmark's fixed 285-case intra-file literature subset is
described in the [benchmark notes](benchmarks.md#diffbenchmark-literature-subset-result), with the
complete case-level data in the
[official evaluation report](../benchmarks/diffbenchmark-literature-evaluation-v3.json). Of the 285
selected cases, 283 were evaluated, independently verified, and replayed byte for byte. One
digest-pinned malformed oracle and one digest-pinned malformed source were classified separately;
there were no unexpected case errors, and `benchmarkComplete` is `true`.

Within this run's fixed scorable adapter universe, program-element relations achieved 99.993%
micro precision, 93.600% micro recall, and 96.691% micro F1. Fine mappings achieved 99.948% micro
precision, 92.559% micro recall, and 96.112% micro F1. The adapter made 15,499 of 15,680 raw
program-element relations and 143,454 of 145,435 raw fine-mapping relations scorable. It excluded
the remaining 181 and 1,981 oracle relations, respectively, and recorded predictions outside that
universe as unscored rather than correct or incorrect. Multi-relation recall remained limited:
0/22 for program elements and 22/2,256 for fine mappings, with no ambiguity candidate covering a
scorable gold relation.

These measurements characterize this adapter and protocol on the fixed literature subset. They are
not directly comparable with published full-corpus results or results produced under a different
protocol, including the RefactoringMiner and GumTree figures above. Differences in corpus coverage,
node taxonomy, adapter exclusions, relation categories, and scoring denominators preclude a valid
head-to-head ranking or improvement claim from these numbers alone.

## Design lessons adopted by StrataDiff

1. **Use Tree-sitter as a frontend, not an oracle.** Parser success and grammar identity are part of
   the report. Syntax errors fail explicitly in conservative mode.
2. **Steal GumTree Simple's fast path, not its forced commitment.** Unique identical Merkle subtrees
   and local child alignment are cheap anchors; duplicate buckets remain ambiguous.
3. **Adopt RefactoringMiner's hierarchy.** Future adapters will solve repository, file, declaration,
   statement, and expression levels separately, then feed verified refactoring evidence downward.
4. **Use Graphtage-style exact local optimization selectively.** Small ambiguity components can use
   ordered dynamic programming or min-cost matching; large repeated regions stay symbolic.
5. **Treat Difftastic and SemanticDiff as presentation references.** A useful terminal/UI view can
   suppress trivia, but the machine report must retain raw bytes and uncertainty.
6. **Support one-to-many relations explicitly.** Extract, inline, copy, split, and merge cannot be
   represented honestly by a universal one-to-one node map.
7. **Keep language semantics pluggable.** Lossless CST is universal; bindings, overload resolution,
   macro expansion, and typed equivalence require compiler-grade language adapters.

## What “100%” means here

StrataDiff aims for 100% replay success and 100% precision of claims marked as verified. It does not
claim 100% correspondence recall. A low-coverage result with explicit ambiguity is preferable to a
plausible-looking false move. Accuracy reports will therefore publish coverage and abstention beside
precision and recall.

Tree-sitter positions in the report use zero-based rows and UTF-8 byte columns. Consumers needing
Unicode code-point or UTF-16 editor coordinates must convert them explicitly.
