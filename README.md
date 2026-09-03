# StrataDiff

**A structural diff that knows when it does not know.**

StrataDiff is a fast, proof-carrying source-code differ. It separates three questions that
traditional AST differencers often mix together:

1. What byte transformation turns the old file into the new file?
2. Which structural predicates can be checked directly?
3. Which node correspondences are forced by the declared model, merely suggested, or ambiguous?

The first question is answered losslessly. The second is independently rechecked by
`stratadiff verify`. The third never silently turns a heuristic score into a historical fact.

> **Project status:** early alpha. The replay certificate, conservative syntax anchors,
> duplicate ambiguity handling, deterministic JSON report, and six grammar adapters work now.
> Binding-aware rename proofs and the all-optima stable-core solver are on the roadmap.

## Why another code diff?

Line diff is exact but structurally coarse. GumTree-style matching is useful but must choose a
single mapping even when multiple histories explain the same two snapshots. That creates false
moves, false updates, and unstable output around repeated code.

StrataDiff makes uncertainty part of the data model:

- `predicate` says what is observable: `byte_equal`, `syntax_equal`, or `shape_equal`.
- `correspondence` says how a pair was selected: `model_forced` or `suggested`.
- `ambiguities` preserve repeated or symmetric candidates as sets instead of guessing a pair.
- `patch` and `certificate` rebuild and hash-check the target byte for byte.

Two snapshots cannot reveal whether identical blocks were swapped, deleted and pasted, or left
untouched. No snapshot-only algorithm can be 100% correct about that hidden history. StrataDiff's
100% target is therefore precise: every certified predicate must be true, and applying a valid
report must reproduce the target bytes exactly. It abstains where identity is not observable.

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
5. Label unique shape-only pairs as suggestions, never as facts.
6. Preserve repeated shape-equivalent children as symbolic ambiguity groups.
7. Derive insertions, deletions, equivalent relocations, child-order changes, and suggested updates.
8. Build an exact patch using line-level Patience anchors, bounded byte-level Myers refinement, and
   a linear replacement path for large unmatched regions.
9. Replay the patch immediately and emit a BLAKE3 certificate only if the output is byte-identical.

Typical matching and hashing are linear in syntax-tree size. Candidate buckets avoid the
quadratic cross-product caused by repeated nodes. Expensive byte refinement is capped at 64 KiB
per unmatched region.

Source rows and columns follow Tree-sitter: zero-based rows and UTF-8 byte columns.

See [DESIGN.md](DESIGN.md) for invariants, [docs/research.md](docs/research.md) for the tool and paper
survey that motivated the design, and [docs/benchmarks.md](docs/benchmarks.md) for the first local
performance baseline.

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
      "correspondence": "suggested",
      "evidence": ["unique_shape_under_mapped_parent", "not_an_identity_claim"]
    }
  ],
  "ambiguities": [
    {
      "reason": "multiple shape-equivalent children admit more than one correspondence"
    }
  ],
  "certificate": {
    "patch_verified": true
  }
}
```

## Near-term roadmap

- All-optima ordered alignment: emit only pairs present in every optimal alignment.
- Binding-aware alpha equivalence and no-capture rename certificates.
- An independent, minimal verifier crate with correspondence-rule certificates.
- Repository mode with conservative file pairing and parallel parsing.
- Java and C/C++ semantic adapters using compiler-grade front ends.
- IDE provenance mode, which can observe actual node lineage instead of inferring history.
- Accuracy and abstention benchmarks against GumTree, RefactoringMiner, and curated adversarial
  edits.

## License

MIT.
