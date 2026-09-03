# StrataDiff

**A structural diff that knows when it does not know.**

StrataDiff is a fast, proof-carrying source-code differ. It separates three questions that
traditional AST differencers often mix together:

1. What byte transformation turns the old file into the new file?
2. Which structural predicates can be checked directly?
3. Which node correspondences are forced by the declared model, merely suggested, or ambiguous?

The first question is answered losslessly. The second is independently rechecked by
`stratadiff verify`. The third never silently turns a heuristic score into a historical fact.

> **Project status:** research alpha. The replay certificate, conservative syntax anchors, bounded
> all-optima child alignment, duplicate ambiguity handling, deterministic JSON report, and seven
> grammar adapters work now. A provenance-complete DiffBenchmark literature-subset evaluation is
> published below. Binding-aware rename proofs remain on the roadmap.

## Why another code diff?

Line diff is exact but structurally coarse. GumTree-style matching is useful but must choose a
single mapping even when multiple histories explain the same two snapshots. That creates false
moves, false updates, and unstable output around repeated code.

StrataDiff makes uncertainty part of the data model:

- `predicate` says what is observable: `byte_equal`, `syntax_equal`, or `shape_equal`.
- `correspondence` says how a pair was selected. The current engine emits `model_forced` pairs;
  `suggested` remains reserved in the v1 data model for future explicitly evidenced rules.
- `ambiguities` preserve repeated or symmetric candidates as sets instead of guessing a pair.
- `patch` and `certificate` rebuild and hash-check the target byte for byte.

Two snapshots cannot reveal whether identical blocks were swapped, deleted and pasted, or left
untouched. No snapshot-only algorithm can be 100% correct about that hidden history. StrataDiff's
100% target is therefore precise: every certified predicate must be true, and applying a valid
report must reproduce the target bytes exactly. It abstains where identity is not observable.

## Evaluated result

The checked-in evaluation selected all 285 cases in DiffBenchmark's pinned Java literature subset.
It evaluated, independently verified, and byte-for-byte replayed all 283 well-formed cases. The two
remaining inputs are digest-pinned upstream data defects: one malformed oracle and one malformed
Java source. There were no unexpected case errors.

| Fixed scorable adapter universe | Precision | Recall | F1 | Oracle coverage |
|---|---:|---:|---:|---:|
| Program elements | 99.993% | 93.600% | 96.691% | 98.846% |
| Fine mappings | 99.948% | 92.559% | 96.112% | 98.638% |

These correspondence scores apply only inside the declared scoring universe; they are not a claim
of 100% historical identity accuracy. In particular, ambiguity-covered gold relations were 0 in
this run, multi-relation recall remains weak, and predictions outside the scoring universe are
reported but not counted as true or false positives. This subset and protocol are not directly
comparable with published full-corpus GumTree or RefactoringMiner figures.

See the [complete results and limitations](docs/benchmarks.md), the
[raw evaluation report](benchmarks/diffbenchmark-literature-evaluation-v3.json), and the
[artifact checksums](benchmarks/SHA256SUMS).

## Quick start

Rust 1.90 or newer is required.

```console
cargo build --release
target/release/stratadiff diff examples/demo/before.py examples/demo/after.py \
  --output change.axd
target/release/stratadiff verify change.axd \
  examples/demo/before.py examples/demo/after.py
target/release/stratadiff apply change.axd examples/demo/before.py \
  --output rebuilt.py
cmp rebuilt.py examples/demo/after.py
```

Run the complete local CI gate with `scripts/ci.sh`.

Print the full machine-readable result with `--json`:

```console
stratadiff diff old.ts new.ts --json
```

Supported grammars in the current build are Python, JavaScript, TypeScript, TSX, Rust, Java, and JSON.
Unknown extensions and parser error nodes fail explicitly; StrataDiff does not disguise a text
fallback as a successful structural analysis.

## Current algorithm

The alpha implements the first useful slice of Proof-Carrying Structural Diff (PCSD):

1. Parse both byte arrays into concrete syntax trees with Tree-sitter.
2. Compute domain-separated byte, syntax, and shape Merkle fingerprints bottom-up.
3. Verify hash hits recursively, so correctness does not depend on collision resistance alone.
4. Map globally unique identical subtrees and unique identical children under mapped parents.
5. Split unmatched direct children at non-crossing exact anchors, partition each region into
   bounded compatibility-graph components, and align at most 64 active children per side in each
   order-interaction component. Map a singleton candidate-group pair only when it is present in every
   maximum-cardinality ordered alignment.
6. Preserve ties, the full symmetry class of observationally identical duplicates, and oversized
   interaction components as symbolic ambiguity groups.
7. Derive insertions, deletions, exact equivalent relocations, child-order changes, and
   model-forced shape updates without conflating their evidence levels.
8. Build an exact patch using line-level Patience anchors, bounded byte-level Myers refinement, and
   a linear replacement path for large unmatched regions.
9. Replay the patch immediately and emit a BLAKE3 certificate only if the output is byte-identical.

Typical matching and hashing are linear in syntax-tree size. Ordered dynamic programming is
restricted to independent interaction components of at most 64 active nodes per side; larger
components remain symbolic and never allocate a quadratic candidate matrix. Candidate compatibility
scanning is capped at 16,384 pairs per verified shape class. Expensive byte refinement is capped at
64 KiB per unmatched region.

Source rows and columns follow Tree-sitter: zero-based rows and UTF-8 byte columns.

See [DESIGN.md](DESIGN.md) for invariants, [docs/research.md](docs/research.md) for the tool and paper
survey that motivated the design, and [docs/benchmarks.md](docs/benchmarks.md) for reproducible
evaluation results and the local performance baseline.

The JSON serialization and structural constraints are published as
[schema/report-v1.schema.json](schema/report-v1.schema.json). Semantic validity is stricter than
the schema alone and is established by `stratadiff verify`, which independently reparses the
snapshots and re-derives the report's claims.

## Report excerpt

```json
{
  "relations": [
    {
      "predicate": "syntax_equal",
      "correspondence": "model_forced",
      "evidence": ["globally_unique_identical_syntax_subtree", "recursive_syntax_equality_check"]
    },
    {
      "predicate": "shape_equal",
      "correspondence": "model_forced",
      "evidence": [
        "bounded_ordered_child_alignment_v1",
        "pair_present_in_every_optimal_alignment",
        "recursive_shape_equality_check",
        "not_a_historical_identity_claim"
      ]
    }
  ],
  "ambiguities": [
    {
      "reason": "repeated shape-equivalent children are not treated as identities even when source order selects one optimal alignment"
    }
  ],
  "certificate": {
    "patch_verified": true
  }
}
```

## Near-term roadmap

- Binding-aware alpha equivalence and no-capture rename certificates.
- An independent, minimal verifier crate with correspondence-rule certificates.
- Repository mode with conservative file pairing and parallel parsing.
- Java and C/C++ semantic adapters using compiler-grade front ends.
- IDE provenance mode, which can observe actual node lineage instead of inferring history.
- Accuracy and abstention benchmarks against GumTree, RefactoringMiner, and curated adversarial
  edits.

## License

MIT.
