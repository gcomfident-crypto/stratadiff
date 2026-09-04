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
5. **Determinism:** identical bytes, artifact labels, parser identity and version, and options
   produce identical report output.

`model_forced` means forced by the documented matching model. It is not a claim about the author's
unobserved editing history. A future provenance mode can add `observed_identity` when an editor or
LSP event log actually records that history.

## Snapshot model

Native Tree-sitter syntax nodes record:

- snapshot-local preorder ID;
- kind, parent field, byte span, and source position;
- ordered children and subtree size;
- `byte_hash`: exact bytes in the node span, including internal trivia;
- `syntax_hash`: kind and child syntax, with inter-node trivia omitted;
- `shape_hash`: syntax with grammar kinds containing `identifier`, `string`, `integer`, `float`,
  `number`, or `comment` normalized into coarse classes; other leaf kinds retain their grammar
  identity.

Hashes are domain-separated BLAKE3 indexes. A hash match is only a candidate: the engine recursively
checks kind, arity, leaves, and children before emitting an equality predicate.

Whitespace between syntax nodes is intentionally absent from the CST equality view but remains in
the original byte tape and lossless patch. Consequently, formatting-only changes are visible and
replayable without pretending they changed program structure.

Universal mode is a separate byte-defined representation: `universal_file` contains line nodes,
which contain runs of ASCII word bytes, whitespace, LF, ASCII punctuation, or opaque bytes. Its
leaves retain exact bytes even in the shape hash. It provides conservative structure and exact
replay for arbitrary byte content within declared resource limits, not language syntax or
semantics.

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
8. Encode ties between singleton candidate groups once per interaction component as an exact
   ordered constraint: the explicit possible-pair support, the residual number of matches to
   select, one-to-one endpoints, and strict child order together describe every valid optimum
   without enumerating exponentially many bundles. Repeated symmetry, components above the
   64-node cap, and shape classes requiring more than 16,384 compatibility checks instead emit
   `symbolic_abstention` with `pair_claims: none`. Their endpoint sets define only the abstention
   scope. No consumer may interpret them as a Cartesian product.

## Change events

Events are derived after matching and cannot influence certified equality:

- `formatting_only`: root syntax is equal while bytes differ;
- `equivalent_relocation`: under the declared mapping model, an exact pair's before parent has a
  mapped counterpart different from the pair's actual after parent;
- `child_order_changed`: mapped direct children have a different relative order;
- `model_forced_update`: a shape-equal pair occurs in every optimal ordered alignment but has
  different syntax;
- `suggested_update`: reserved for a future explicitly evidenced suggestion rule;
- `insert` and `delete`: maximal unmatched, non-ambiguous subtrees.

The wording is deliberate. `equivalent_relocation` describes the snapshots; it does not assert that
the user executed a move command. The current exact-anchor compatibility rule is intentionally
conservative and does not promise relocation recall; move-plus-edit cases may remain insert/delete
or explicit ambiguity until a dedicated evidence phase can prove more.

## Patch and certificate

The lossless layer is independent from parser semantics. Its current producer strategy identifier is
`bounded-patience-lines+bounded-byte-refinement-v2`:

1. Patience diff finds stable complete-line anchors when the two inputs contain at most 65,536
   lines in total. Inputs above that budget bypass line-index materialization.
2. Changed regions whose two trimmed sides total at most 64 KiB use Myers over bytes.
3. Larger equal-length regions are scanned linearly into aligned unequal-byte runs, capped at 4,096
   edits per region. When that cap is reached, the remaining tail becomes one replacement.
4. Larger unequal-length regions become a single replacement after trimming their common prefix
   and suffix. Patch output is capped at 65,536 edits and falls back to one whole-file replacement
   if later regions would exceed that cap.
5. Each edit addresses a non-overlapping before-byte interval and carries replacement bytes as
   base64.
6. Report construction replays all edits and refuses to issue a certificate unless the resulting
   bytes and BLAKE3 digest match the target.

This keeps the common case compact and caps pathological refinement costs. The byte-patch primitive
works for BOMs, NULs, invalid UTF-8, mixed line endings, and files with no final newline. Native
structural modes require both snapshots to parse successfully under the selected grammar;
Universal mode accepts arbitrary byte content within the declared source, syntax-node, and work
limits but makes no language-level claim.

The producer emits at most 65,536 edits. The default verifier accepts at most 250,000 so it can
validate conforming reports from other producers; it checks the strategy identifier, edit bounds,
canonical Base64, and exact replay, but does not rerun this producer strategy or certify that a
patch is minimal or canonical.

The large-region aligned scan does not infer insert/delete resynchronization. An insertion followed
by a deletion with no net length change can therefore appear as a larger exact replacement between
the two synchronization points. This affects presentation granularity, never replay correctness.

## Verification boundary

`stratadiff verify` currently rebuilds the selected parser representation and checks:

- both input sizes and BLAKE3 digests;
- patch bounds, non-overlap, replay bytes, and target digest;
- relation IDs and all serialized node metadata;
- one-to-one relation cardinality;
- every declared byte, syntax, and shape predicate;
- exact-anchor uniqueness and descendant membership;
- stable-core interaction partitioning, all-optima support and residual cardinality, duplicate
  symmetry closure, and 64-node component abstention;
- exact ambiguity constraints, no-pair symbolic scopes, derived changes, and summary counters.

The verifier independently re-derives these rules without calling the producer matcher. It lives in
the separately consumable `stratadiff-verifier` crate, whose dependency graph excludes the producer
matcher, `similar`, CLI parsing, CSV tooling, and temporary-file support. Compact proof objects may
replace more of the current recomputation in later versions.

### Untrusted-input boundary

The bytes entry points enforce the boundary in layers:

1. Reject raw reports and source snapshots beyond their byte caps. A streaming JSON pass counts
   relations, ambiguities, endpoints, possible pairs, changes, edits, and evidence items before the
   typed report can allocate their vectors.
2. Preflight the typed report with checked arithmetic, canonical RFC 4648 Base64 validation, patch
   range checks, decoded-replacement limits, and replay-output limits.
3. Rebuild both parser representations with combined node and per-tree depth limits. Native
   Tree-sitter modes also enforce per-tree progress-callback limits; Universal is bounded by source,
   node, depth, and verification-work limits. Invalid-tree diagnosis stays inside the applicable
   bounds.
4. Charge the independent structural/model verification passes for relation scans, recursive
   equality, exact-anchor traversal, candidate construction, component partitioning, sorting,
   dynamic-programming cells, and per-candidate forcedness. A charge is checked before the
   corresponding expensive loop or allocation.
5. `apply` decodes replacement bytes once, reuses that replay for certificate and structural
   verification, and opens the destination only after every check succeeds.

The compatibility functions `verify_report` and `apply_patch` use `VerificationLimits::default()`.
The bytes APIs should be preferred for untrusted JSON because a caller that constructs a
`DiffReport` first has already paid its deserialization cost. Limits make adversarial work finite
and diagnosable; they are not an OS sandbox, wall-clock deadline, or absolute process-memory cap.
Tree-sitter and the selected grammar remain trusted, and its node limit applies to Rust-side syntax
materialization after Tree-sitter has built its internal tree. Verification work units are a
deterministic conservative accounting model rather than elapsed time or machine instructions.

## Complexity

For N syntax nodes and E bytes, parsing, hashing, indexes, and component bookkeeping use O(N + E)
expected space. Candidate compatibility scanning is capped at 16,384 pairs per verified shape
class. The ordered-DP portion for a bounded component with dimensions A and B uses O(A × B) memory
and O((U + 1) × A × B) scalar DP work, where A and B are at most 64 and U is at most 64 unique
possible-optimal pairs checked for forcedness. Recursive equality and compatibility work is charged
separately by visited subtree nodes. Components above the cap stay symbolic; independent bounded
components in a larger region do not inherit that abstention. Line anchoring uses the Patience
implementation from `similar` only within the 65,536-line combined budget; byte-level Myers is
limited to 64 KiB across both trimmed sides of an unmatched region, and larger equal-length
refinement is linear.
