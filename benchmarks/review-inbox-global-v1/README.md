# Review Inbox Global v1

Review Inbox Global v1 is a deterministic, network-free target-semantic corpus for a
cross-repository review-resume inbox. It specifies one narrow question that an implementation must
answer without silently losing evidence:

> For this exact human reviewer, which previously completed review checkpoints now require
> attention, and when is the available metadata insufficient to say?

The corpus contains 60 controlled synthetic cases shaped by observed GitHub and API failure modes.
The cases model GitHub review and
pagination behavior, but contain no source code, diff, PR text, review text, commit message, email,
or credential. They are reproducible regression vectors, not a claim that the scenarios occur at
the same rate in production.

The checked-in commands validate the corpus and its implementation-independent Python reference
evaluator. A Rust conformance adapter also feeds all 60 materialized observations through the
shared target-policy decision core and compares every result with the frozen oracle in CI. The
live Inbox uses an additional executable-Resume policy and has separate CLI tests; this corpus does
not exercise the GitHub collector or the complete `stratadiff inbox` command path.

## Target contract

The frozen oracle requires the following behavior:

- A checkpoint belongs to one immutable human reviewer identity. Matching a mutable login alone is
  insufficient.
- Only `APPROVED` and `CHANGES_REQUESTED` are completed checkpoints. `COMMENTED`, `PENDING`, and
  `DISMISSED` never replace one.
- The latest completed checkpoint is selected by `(submitted_at, database_id)`, so a later comment
  cannot erase an earlier formal review.
- A changed head, changed effective base, or active re-review request can make a completed
  checkpoint actionable. The trigger remains explicit as `head_changed`, `base_drift`, or
  `review_re_requested`, including combinations.
- A missing head, an unprovable stable-head base comparison, or an all-review count beyond the
  resumable bound is `insufficient_evidence`, never clean.
- A truncated outer search is always `partial`, including when every captured row is up to date or
  no row was captured.
- Provider host, repository node, PR node, reviewer node, review node, checkpoint/head/base OIDs,
  and active request state participate in transition identity. Identical observations are stable;
  cross-host or cross-object aliases are distinct.
- Incomplete pagination, duplicate nodes, identity changes, repository confusion, permission loss,
  non-atomic revalidation changes, and malformed checkpoints fail closed with stable error codes.

This is a target semantic contract. The `0.5.0` collector records the current base and active
review-request state, but GitHub does not expose the historical base at review time. The live path
therefore uses a stricter executable-Resume policy: with a stable head and no checkpoint base it
reports insufficient evidence instead of emitting the corpus's base-only or re-request-only target
actions. The benchmark must not weaken those future-facing cases merely to match that limitation.

## Bundle layout

- `cases.json` contains one fully specified base observation and 60 minimal patches. A patch makes
  each changed fact auditable instead of repeating a large mock API response.
- `oracle.json` freezes the expected status, per-category counts, selected checkpoint, trigger,
  unobservable reason, or fail-closed error for every case.
- `manifest.json` binds both assets by SHA-256, freezes acceptance gates and required coverage, and
  records the exact aggregate expected from independent derivation.
- `verify.py` materializes the patches, validates closed schemas and privacy constraints, derives
  every outcome independently of the Rust product code, checks transition-identity relations, and
  compares the result with the oracle.
- `SHA256SUMS` binds the human-readable and executable bundle. It detects accidental/local
  modification; it is not a publisher signature.

The patch language is deliberately small:

- `replace` changes one existing JSON Pointer target;
- `append` adds one explicit value to an existing array;
- `append_copy` duplicates an existing value into an existing array.

Unknown paths, unsupported operations, and duplicate JSON keys are rejected.

## Coverage

The 60 cases exercise 55 coverage tags. The most important groups are:

| Group | Representative cases | Expected behavior |
|---|---|---|
| Formal checkpoints | `approved-stale`, `changes-requested-stale`, `same-time-database-id-tiebreak` | reviewer-specific latest checkpoint |
| Non-checkpoints | `commented-only`, `pending-only`, `dismissed-rerequested` | no eligible checkpoint |
| Later review activity | `approval-followed-by-comment`, `newer-approval-wins`, `dismissed-then-approved` | preserve or supersede correctly |
| Rewrites | `force-push`, `pure-rebase`, `restack` | actionable changed-head transition |
| Base/request signals | `base-drift-only`, `head-and-base-drift`, `rerequest-only`, `rerequest-and-head` | explicit independent triggers |
| Partial collection | `partial-actionable`, `partial-clean`, `partial-zero-captured` | always `partial`, never false-clean |
| Review pagination | `multipage-complete`, `incomplete-review-pagination`, `stalled-review-cursor`, `review-count-mismatch` | complete or fail closed |
| Actor identity | `mixed-case-login`, `reviewer-node-collision`, `reviewer-bot-collision`, `revalidation-viewer-change` | immutable human binding |
| Host/repository identity | `provider-url-mismatch`, `ghes-canonical`, `other-repository-same-number`, requested-repository mismatch cases | no cross-scope aliasing |
| Duplicate identity | `duplicate-review-node`, `duplicate-review-database-id`, `duplicate-pull-request-node` | reject ambiguous evidence |
| Unobservable evidence | `missing-head`, missing-base cases, `total-review-limit-exceeded` | retain as insufficient evidence |
| Permission/revalidation | `search-forbidden`, `repository-not-found`, revalidation failure/change cases | explicit error, never omitted |

The frozen global-status aggregate is:

| Result | Cases |
|---|---:|
| Actionable | 19 |
| Up to date | 6 |
| No eligible reviews | 3 |
| Insufficient evidence | 4 |
| Partial | 3 |
| Fail-closed error | 25 |
| Total | 60 |

The three partial scans retain their captured candidate decisions instead of erasing them. Across
all successful cases the candidate-level oracle contains 20 actionable, 7 up-to-date, 3
no-eligible-review, and 4 unobservable decisions.

## Offline verification

Run the complete bundle and Rust conformance checks:

```text
python3 -B benchmarks/review-inbox-global-v1/verify.py verify
python3 -B benchmarks/review-inbox-global-v1/verify.py self-test
(cd benchmarks/review-inbox-global-v1 && sha256sum -c SHA256SUMS)
cargo test --test review_inbox_global_v1 --locked
```

To inspect fully expanded observations or independently generated expectations:

```text
python3 -B benchmarks/review-inbox-global-v1/verify.py materialize
python3 -B benchmarks/review-inbox-global-v1/verify.py derive-oracle
python3 -B benchmarks/review-inbox-global-v1/verify.py summary
```

`materialize` writes canonical JSON to stdout. The Rust conformance adapter consumes that output
without granting the reference evaluator network access or repository write access.

## Independent and tamper checks

The oracle contains outcomes, not executable rules. The verifier derives outcomes from materialized
evidence and then compares them with that separately frozen oracle. It also recomputes manifest
summary gates and checks all cross-case transition relations.

The self-test invokes the real validation path and rejects nine mutations: a forged oracle result,
a changed observation with a rebound asset checksum, an omitted case, a forbidden payload field, a
stale manifest checksum, a forged transition relation, an unknown patch path, a duplicate JSON key,
and an unmet required-coverage tag.

## Claim boundary

Passing the complete bundle demonstrates that the frozen fixtures, oracle, reference evaluator,
manifest, and shared Rust target-policy core agree across all 60 cases. It does not demonstrate
end-to-end conformance by the live GitHub collector or complete Rust CLI. It also does not prove
that a checkpoint was reviewed carefully, that two source histories are semantically equivalent,
that the Resume residue is correct, that defects are recalled, or that developers save time, nor
does it estimate production prevalence or product-market fit.

The rewrite labels are controlled provenance: metadata alone cannot distinguish a force-push from
a rebase or restack. Source-level residue correctness belongs in Review Continuity/Review Delta
benchmarks, while time saved and issue recall belong in the preregistered reviewer study.
