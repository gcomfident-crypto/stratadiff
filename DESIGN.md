# StrataDiff design

## Accuracy contract

StrataDiff does not define accuracy as “always return a complete AST mapping.” A total mapping is
wrong on some symmetric inputs because the generating history is absent from two snapshots.
Instead, the engine has five invariants:

1. **Replay completeness:** applying the patch to the exact before bytes yields the exact after
   bytes.
2. **Predicate soundness:** every `byte_equal`, `syntax_equal`, or `shape_equal` relation can be
   checked from the snapshots and parser manifest.
3. **Explicit epistemic status:** observable predicates, model correspondence, and derived change
   events are separate fields.
4. **Conservative ambiguity:** repeated candidates remain a set unless the declared model forces a
   pair.
5. **Determinism:** identical bytes, grammar versions, and options produce identical structural
   output.

`model_forced` means forced by the documented matching model. It is not a claim about the author's
unobserved editing history. A future provenance mode can add `observed_identity` when an editor or
LSP event log actually records that history.

## Snapshot model

Each syntax node records:

- snapshot-local preorder ID;
- kind, parent field, byte span, and source position;
- ordered children and subtree size;
- `byte_hash`: exact bytes in the node span, including internal trivia;
- `syntax_hash`: kind and child syntax, with inter-node trivia omitted;
- `shape_hash`: syntax with identifier and literal values normalized.

Hashes are domain-separated BLAKE3 indexes. A hash match is only a candidate: the engine recursively
checks kind, arity, leaves, and children before emitting an equality predicate.

Whitespace between syntax nodes is intentionally absent from the CST equality view but remains in
the original byte tape and lossless patch. Consequently, formatting-only changes are visible and
replayable without pretending they changed program structure.

## Matching strata

The current alpha applies these rules in order:

1. Pair roots because the caller supplied the file pair.
2. Select globally unique, recursively verified identical syntax subtrees of at least three nodes.
3. In each mapped parent pair, select a unique unmatched child with the same field, kind, and
   verified syntax.
4. In the remaining local children, emit a suggestion only when field, kind, and recursively
   verified shape are unique on both sides.
5. Preserve non-unique local shape buckets as `AmbiguityGroup` values.

The planned stable-core solver will construct all optimal ordered alignments for each anchor-bounded
region and emit a pair as `model_forced` only when it occurs in every optimum. Oversized repetitive
regions will remain symbolic ambiguity buckets rather than consume quadratic memory.

## Change events

Events are derived after matching and cannot influence certified equality:

- `formatting_only`: root syntax is equal while bytes differ;
- `equivalent_relocation`: an exact subtree is under a different mapped parent;
- `child_order_changed`: mapped direct children have a different relative order;
- `suggested_update`: a unique local shape pair has different syntax;
- `insert` and `delete`: maximal unmatched, non-ambiguous subtrees.

The wording is deliberate. `equivalent_relocation` describes the snapshots; it does not assert that
the user executed a move command.

## Patch and certificate

The lossless layer is independent from parsing:

1. Patience diff finds stable complete-line anchors.
2. Changed regions of at most 64 KiB use Myers over bytes.
3. Larger unmatched regions become a single replacement after trimming their common prefix and
   suffix.
4. Each edit addresses a non-overlapping before-byte interval and carries replacement bytes as
   base64.
5. Report construction replays all edits and refuses to issue a certificate unless the resulting
   bytes and BLAKE3 digest match the target.

This keeps the common case compact and caps pathological refinement costs. It also works for BOMs,
NULs, invalid UTF-8, mixed line endings, and files with no final newline.

## Verification boundary

`stratadiff verify` currently performs a fresh parse and checks:

- both input sizes and BLAKE3 digests;
- patch bounds, non-overlap, replay bytes, and target digest;
- relation IDs and all serialized node metadata;
- one-to-one relation cardinality;
- every declared byte, syntax, and shape predicate.

The next verifier milestone separates this checker into a dependency-minimal crate and adds compact
proofs for `model_forced`, including anchor uniqueness and all-optima membership.

## Complexity

For N syntax nodes and E bytes, parsing, hashing, indexes, and fixed-size candidate buckets use
O(N + E) expected time and O(N) memory. Line anchoring uses the Patience implementation from
`similar`; byte-level Myers is limited to 64 KiB per unmatched region. The current matcher never
constructs the Cartesian product of duplicate buckets.
