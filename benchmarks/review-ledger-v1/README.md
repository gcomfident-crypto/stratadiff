# Review Ledger v1

Review Ledger v1 is a controlled, declarative oracle for Issue
[`#14`](https://github.com/gcomfident-crypto/stratadiff/issues/14). It freezes the expected behavior of
the GitHub review ledger and reviewer × CODEOWNERS coverage gate independently of the product
implementation.

The dataset is intentionally independent of the current implementation. [`manifest.json`](manifest.json)
contains symbolic webhook events, ownership snapshots, Git snapshots, and machine-readable expected
states. Its separate `implementation_snapshot` reports authoring-time coverage but is not an input
to any oracle.

## Contract

The suite has four boundaries:

1. Authenticate the exact raw webhook body before decoding it. Treat `X-GitHub-Delivery` as a
   deduplication identity, not as signed content.
2. Keep receipts, dismissals, and synchronize transitions as immutable audit facts. Reduce them by
   provider semantics and authoritative current PR state, never by arrival order.
3. Resolve CODEOWNERS from the exact protected-base commit. Owners on one winning line are OR
   alternatives; independently matched owner domains remain separate requirements.
4. Carry review coverage only by complete Git change identity or the conservative four-snapshot
   replay proof. Missing objects, invalid policy, ambiguous state, and incomplete ownership all keep
   the required check red.

The dismissal cases deliberately separate audit history from active state. A dismissal tombstone is
keyed by stable review ID even when GitHub supplies null commit metadata. A delayed submission is
still stored after its dismissal and supplies the immutable commit evidence, but it cannot reactivate
that review. If a reviewer's latest completed review is dismissed, an older approval is not reused;
only a distinct later review can reactivate coverage.

The exact base and head passed to `review-coverage` are caller-provided authoritative observations;
the ledger does not authenticate their provider freshness. When synchronize facts exist, every
deduplicated edge must belong to one unbranched history ending at that supplied head. Missing history,
branches, cycles, and disconnected edges fail closed. This favors evidence integrity over liveness
when delivery history is incomplete.

The duplicate case also distinguishes provider identity from local observation time. GitHub reuses
the original `X-GitHub-Delivery` on redelivery, while a receiver naturally observes a later local
`received_at`. Same ID plus same event and raw-body digest is therefore a duplicate and retains the
first observation. Same ID plus different content is an atomic conflict.

Every non-comment delivery contains a receiver-signed, domain-separated digest of its one derived
fact. Submitted `commented` reviews are the only deliveries allowed to sign a null fact digest. A
separate receiver-signed snapshot binds the complete canonical ledger body, its revision, and the
counts of every delivery and fact collection. This detects adding, removing, changing, or moving a
fact, including deleting a delivery together with its matching fact.

The signed snapshot authenticates consistency, not freshness. Replaying an entire older valid
ledger and its older signed snapshot cannot be detected from the offline file alone; deployments
need receiver-side latest-revision/root storage or another trusted external checkpoint for rollback
protection. This limitation is outside the benchmark's claims.

The ledger schema remains named v1 because this worktree is an unpublished implementation of the
initial contract. The fact commitment and signed snapshot are part of that first published shape;
no released v1 artifact is being rewritten.

## Cases

| Area | Cases | Expected boundary |
|---|---|---|
| Webhook integrity | verified submit, delayed duplicate redelivery, conflicting body, tampered signature | Verify first; deduplicate without repeated effects; reject conflicts atomically |
| Review state | dismiss-before-submit, latest dismissed without fallback, distinct new review | Append-only audit with monotonic active state |
| PR state | out-of-order synchronize | Preserve transitions; reconcile against authoritative exact head/base |
| Ownership | two domains, owner OR, invalid CODEOWNERS | Invalidate only affected domains; all required domains gate together; policy errors fail closed |
| Teams | missing, secret, read-only, pending-only | Produce typed blockers; never guess or silently drop an owner |
| Provenance | exact-base CODEOWNERS, orphan review commit | Bind source and review to exact Git objects; never substitute a moving ref |
| Carry | exact restack, four-way base drift, genuine author edit | Carry only proven bytes; leave later author work as owner residue |

`invalid-selected-codeowners-source-fails-closed` is intentionally stricter than GitHub's native
behavior. GitHub documents that invalid lines can be skipped; Issue #14 requires this product to
fail closed, avoid partial policy, and avoid falling back to a lower-precedence CODEOWNERS file.

## Current implementation snapshot

The manifest's `implementation_snapshot` is frozen authoring-time metadata, not live status and not
an oracle input. Its `partial` entries deliberately remain historical rather than being rewritten
to match later implementation work.

The public offline runner now exercises every one of the 20 manifest cases end to end through the
CLI, including delayed redelivery, owner alternatives, typed ownership blockers, selective domain
invalidation, and four-way replay. It also runs a separate signed-Passport tamper control. A release
claim requires a clean `--strict-skips` run; source presence or the historical snapshot alone is not
evidence that the suite passes.

## Offline reproduction

The independent [`runner.py`](../../tools/review-ledger-v1/runner.py) now materializes deterministic
raw webhook bytes, exact HMAC vectors, ownership snapshots, and isolated Git histories. Its oracle
uses only the Python standard library and manifest fixtures; it does not import the production Rust
reducers. It invokes the public StrataDiff CLI as the system under test and emits a separate result
artifact with explicit `PASS`, `FAIL`, and `SKIP` outcomes.

```console
cargo build --bin stratadiff
python3 -m unittest tools/review-ledger-v1/test_runner.py
python3 tools/review-ledger-v1/runner.py
```

The default result is `target/review-ledger-v1/result.json`. All 20 manifest cases execute against
the public CLI. The head-state case also requires stale-head and disconnected-history requests to be
rejected, so retaining transitions without using the reducer cannot pass. The runner performs a
separate passport-tamper control: an unchanged passport must verify, while a body modified without
resigning must fail before offline recomputation.

See [`tools/review-ledger-v1/README.md`](../../tools/review-ledger-v1/README.md) for selection,
strict-skip, and retained-fixture options.

## Sources and claim boundary

All provider claims are linked to GitHub's official webhook, REST, CODEOWNERS, teams, checks, and
branch-protection documentation in `manifest.json`; they were verified on
`2026-09-05T12:56:40+08:00`. Review carry and the stricter fail-closed rules are explicitly labeled
as StrataDiff policy rather than GitHub behavior.

Passing this suite would establish conformance to the declared state, ownership, provenance, and
byte-carry rules for these controlled cases. It would not establish semantic safety, approval
restoration, reviewer-time savings, defect recall, or that a pull request is safe to merge.

The benchmark metadata is MIT licensed under the repository's [`LICENSE`](../../LICENSE).
