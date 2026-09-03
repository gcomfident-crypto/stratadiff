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
4. Use non-crossing exact direct-child mappings as barriers. A shape-only pair cannot become forced
   by crossing one of these stronger anchors.
5. In each remaining region, use `(field, kind, shape_hash)` only as an index, partition hash
   buckets by recursive shape equality, filter pairs against existing exact mappings, and join the
   resulting bipartite candidate graph into order-interaction components. A boundary is valid only
   when complete connected candidate groups form the same prefix on both sides, which proves that
   choices in adjacent components cannot cross or share an endpoint.
6. For at most 64 active children per side in each component, compute a maximum-cardinality ordered
   alignment and emit a pair as `model_forced` only when forbidding it lowers the component optimum.
7. Require a forced compatibility-connected candidate group to have one endpoint per side. This
   symmetry guard keeps unconstrained observationally identical duplicates ambiguous even when
   source order yields one optimal diagonal alignment. Exact descendant anchors may split a
   repeated shape class into singleton candidate groups, making the containing root pair forced by
   the declared model rather than by source order.
8. Preserve alignment ties and components above the cap as `AmbiguityGroup` values. A shape class
   requiring more than 16,384 compatibility checks is kept symbolic before pair enumeration;
   otherwise connected candidate groups are derived exactly. No oversized component constructs a
   quadratic DP matrix, and unrelated bounded components in the same anchor region are resolved.

## Change events

Events are derived after matching and cannot influence certified equality:

- `formatting_only`: root syntax is equal while bytes differ;
- `equivalent_relocation`: an exact subtree is under a different mapped parent;
- `child_order_changed`: mapped direct children have a different relative order;
- `model_forced_update`: a shape-equal pair occurs in every optimal ordered alignment but has
  different syntax;
- `suggested_update`: reserved for a future explicitly evidenced suggestion rule;
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

This keeps the common case compact and caps pathological refinement costs. The byte-patch primitive
works for BOMs, NULs, invalid UTF-8, mixed line endings, and files with no final newline. The
structural `diff` command still requires both snapshots to parse successfully under the selected
grammar.

## Verification boundary

`stratadiff verify` currently performs a fresh parse and checks:

- both input sizes and BLAKE3 digests;
- patch bounds, non-overlap, replay bytes, and target digest;
- relation IDs and all serialized node metadata;
- one-to-one relation cardinality;
- every declared byte, syntax, and shape predicate;
- exact-anchor uniqueness and descendant membership;
- stable-core interaction partitioning, all-optima membership, duplicate symmetry closure, and
  64-node component abstention;
- ambiguity groups, derived changes, and summary counters.

The verifier independently re-derives these rules without calling the producer matcher. A later
milestone will move it into a dependency-minimal crate and replace recomputation with compact proof
objects where practical.

## Complexity

For N syntax nodes and E bytes, parsing, hashing, indexes, and component bookkeeping use O(N + E)
expected space. Candidate compatibility scanning is capped at 16,384 pairs per verified shape
class. A bounded component with dimensions A and B uses O(A × B) memory and O(U × A × B) time,
where A and B are at most 64 and U is at most 64 unique possible-optimal pairs checked for
forcedness. Components above the cap stay symbolic; independent bounded components in a larger
region do not inherit that abstention. Line anchoring uses the Patience implementation from
`similar`; byte-level Myers is limited to 64 KiB per unmatched region.
