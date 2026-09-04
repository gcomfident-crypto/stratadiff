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
> all-optima child alignment, duplicate ambiguity handling, deterministic JSON report, independent
> resource-bounded verifier crate, and seven grammar adapters work now. A provenance-complete
> DiffBenchmark literature-subset evaluation is published below. Binding-aware rename proofs remain
> on the roadmap.

## Why another code diff?

Line diff is exact but structurally coarse. GumTree-style matching is useful but must choose a
single mapping even when multiple histories explain the same two snapshots. That creates false
moves, false updates, and unstable output around repeated code.

StrataDiff makes uncertainty part of the data model:

- `predicate` says what is observable: `byte_equal`, `syntax_equal`, or `shape_equal`.
- `correspondence` says how a pair was selected. The current engine emits `model_forced` pairs;
  `suggested` remains reserved in the v2 data model for future explicitly evidenced rules.
- `ambiguities` encode coupled ordered choices as explicit pair constraints. Repeated or oversized
  regions carry `pair_claims: none`, so endpoint sets can never be mistaken for a Cartesian product.
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

The v5 adapter flattens only explicit `possible_pairs` into an edge-union coverage view. That union
is not a jointly selectable mapping, and symbolic abstentions contribute no pair candidates.

See the [complete results and limitations](docs/benchmarks.md), the
[raw evaluation report](benchmarks/diffbenchmark-literature-evaluation-v5.json), and the
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

`diff --output` and `diff --json` emit compact JSON so reports produced within the default 64 MiB
report boundary can be consumed by `verify` and `apply` without a formatting-size mismatch.

Print the full machine-readable result with `--json`:

```console
stratadiff diff old.ts new.ts --json
```

Supported grammars in the current build are Python, JavaScript, TypeScript, TSX, Rust, Java, and JSON.
Unknown extensions and parser error nodes fail explicitly; StrataDiff does not disguise a text
fallback as a successful structural analysis.

## Resource-bounded verification

The `stratadiff-verifier` crate has no dependency on the producer matcher, `similar`, CLI parsing,
CSV tooling, or temporary-file support. For untrusted input, use `verify_report_bytes` or
`verify_and_replay_report_bytes`; these scan collection lengths before constructing the typed
report. The older `verify_report` and `apply_patch` entry points remain source-compatible and use
the defaults below. Typed callers that need different bounds can use `verify_report_with_limits`
and `replay_patch_with_limits`.

| Limit | Default | Scope |
|---|---:|---|
| Raw or compact-serialized report | 64 MiB | One report |
| Source or replayed output | 16 MiB | Each byte array |
| Relations | 250,000 | Total |
| Ambiguity groups | 50,000 | Total |
| Ambiguity endpoints | 500,000 | Both sides combined |
| Exact ambiguity pairs | 1,000,000 | Total |
| Structural changes | 250,000 | Total |
| Patch edits | 250,000 | Total |
| Decoded replacement bytes | 32 MiB | All edits combined |
| Syntax nodes | 1,000,000 | Both fresh parses combined |
| Syntax depth | 512 | Each parse |
| Tree-sitter progress callbacks | 4,000,000 | Each parse |
| Verification work | 128 Mi units | Deterministic semantic-work budget |

The CLI uses these defaults and does not currently expose limit flags. It reads files through a
`limit + 1` bounded reader, validates canonical Base64 and checked size arithmetic, and never writes
an `apply` output until replay and the full structural verification have succeeded. Each relation
may carry at most four evidence items, matching the largest evidence recipe in report v2.

These controls bound attacker-selected input, collection growth, parser progress, recursive
comparison, candidate scanning, sorting, and alignment DP. They are not a process sandbox, a wall
clock deadline, or an allocator-level limit; the selected Tree-sitter grammars and the local runtime
remain part of the trusted computing base. Work units are conservative deterministic charges, not
CPU instructions or milliseconds. Callers that deserialize an untrusted report themselves before
using the typed API give up the decode-time collection protection.

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
6. Encode singleton-group ties as exact coupled ordered constraints. Preserve duplicate symmetry
   and oversized interaction components as symbolic abstentions that make no pair claims.
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
[schema/report-v2.schema.json](schema/report-v2.schema.json). The historical
[v1 schema](schema/report-v1.schema.json) remains available for inspection, but v1 ambiguity sets
cannot be losslessly upgraded without rerunning the original snapshots. Semantic validity is
stricter than the schema alone and is established by `stratadiff verify`, which independently
reparses the snapshots and re-derives the report's claims.

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
      "constraint": {
        "kind": "symbolic_abstention",
        "cause": "duplicate_symmetry",
        "pair_claims": "none"
      },
      "reason": "repeated shape-equivalent children are intentionally unresolved; endpoint sets make no pair claims"
    }
  ],
  "certificate": {
    "patch_verified": true
  }
}
```

## Near-term roadmap

- Binding-aware alpha equivalence and no-capture rename certificates.
- More compact correspondence proof objects for cheaper independent verification.
- Repository mode with conservative file pairing and parallel parsing.
- Java and C/C++ semantic adapters using compiler-grade front ends.
- IDE provenance mode, which can observe actual node lineage instead of inferring history.
- Accuracy and abstention benchmarks against GumTree, RefactoringMiner, and curated adversarial
  edits.

## License

MIT.
