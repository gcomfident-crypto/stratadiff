# Pull Request Candidate v2

Pull Request Candidate v2 is a deterministic, network-free benchmark for one question:

> Which exact Git object is GitHub currently evaluating for the required checks of this pull
> request: the PR head, a test-merge commit, or a merge-group commit?

The corpus freezes ten controlled cases and two read-only provenance captures made on 2026-09-08.
Verification never calls GitHub. The captures preserve only target identity and queue topology; they
do not preserve source, diffs, logs, credentials, or mutable API responses.

## Why v2 supersedes v1

V2 supersedes the candidate-selection semantics in
[`pull-request-candidate-v1`](../pull-request-candidate-v1/) without modifying that frozen bundle.
V1 treated complete but empty Check Runs and legacy statuses on the test-merge commit as
inconclusive. GitHub's official required-check troubleshooting rule instead says that GitHub first
looks for required checks on the test-merge commit and, when that commit has no status, uses the
head commit's status. V2 therefore selects `pr_head` for the unchanged
`github-docs-empty-test-merge` observation. This one oracle change is the intentional semantic delta;
the other nine outcomes and all ten observation payloads remain equivalent to v1.

V1 remains an immutable historical regression bundle and is still verified in CI. V2 is the current
product conformance contract.

## Selection contract

The independent v2 oracle implements these rules:

1. Reject a capture when the target identity changes between the first and final read. This includes
   the PR head or base, test-merge SHA, queue membership, queue-entry identity, state, position,
   base commit, or head commit.
2. When the PR is in a merge queue, select the current GraphQL queue entry's `headCommit`. A missing
   entry is inconclusive. Never substitute the PR head, REST `merge_commit_sha`, or an older queue
   candidate.
3. Outside a merge queue, inspect both Check Runs and legacy commit statuses on the test-merge SHA.
   If either surface contains a signal, select the test-merge commit.
4. If both test-merge signal surfaces were read completely and contain no signal, select the PR head.
   This is GitHub's documented fallback when the test-merge commit has no status.
5. A missing test-merge SHA or an incomplete signal surface remains inconclusive. Absence is
   actionable only after both signal surfaces were collected completely.
6. `isMergeQueueEnabled` alone does not select a merge-group candidate. The PR must currently be in
   the queue and have a stable queue entry.

The selected SHA is only the input to required-check diagnosis. It is not proof that reviews,
conflicts, mergeability, compliance, or code safety are clear.

## Coverage

| Case | Controlled fact | Expected outcome |
|---|---|---|
| `test-merge-check-signal` | test merge has a Check Run | select test merge |
| `test-merge-status-signal` | test merge has only a legacy status | select test merge |
| `github-docs-empty-test-merge` | live-derived test merge has no signal; queue is enabled but PR is not queued | select PR head |
| `test-merge-null` | GitHub has not supplied a test-merge SHA | inconclusive |
| `queue-enabled-not-enqueued` | queue enabled is true but queue membership is false | use normal test-merge rule |
| `clickhouse-queued-entry` | live-derived GraphQL queue entry exists | select entry head commit |
| `queued-entry-missing` | PR claims queue membership without an entry | inconclusive |
| `queue-entry-drift` | queue entry changes between boundary reads | retry |
| `old-queue-green-new-missing` | an obsolete queue SHA is green and the current SHA has no signal | select current SHA |
| `test-merge-api-gap` | one test-merge signal surface is unavailable | inconclusive |

## Provenance controls

`github/docs#45788` preserves the independently observed PR head, base, test-merge SHA, queue flags,
and absent test-merge `statusCheckRollup`. `ClickHouse/ClickHouse` preserves the first five entries
returned by `repository.mergeQueue.entries(first:5)`, including the linked base/head commit chain.
The live records are provenance controls, not prevalence estimates and not promises that those
mutable GitHub objects still have the same values.

The ClickHouse capture demonstrates an important empirical invariant: each queue entry's synthetic
head differed from its PR head, and entries 2–5 used the preceding entry's synthetic head as their
base. The benchmark still treats `MergeQueueEntry.headCommit` as an observed GraphQL contract rather
than claiming a stronger undocumented guarantee.

## Bundle layout

- `cases.json` contains closed-shape observations and the two provenance captures.
- `oracle.json` contains only expected candidate-selection outcomes.
- `manifest.json` binds cases and oracle by SHA-256, declares the v1 supersession, and freezes
  coverage gates and claim limits.
- `verify.py` validates every field, derives the v2 oracle independently, enforces the declared
  supersession metadata and provenance controls, verifies checksums, and runs mutation tests.
- `SHA256SUMS` covers every other bundle asset. It is integrity metadata, not a signature.

The repository-level Rust conformance test separately proves that all ten observation payloads are
equal to v1 and that only the named oracle outcome differs.

## Offline verification

```text
python3 -B benchmarks/pull-request-candidate-v2/verify.py verify
python3 -B benchmarks/pull-request-candidate-v2/verify.py self-test
(cd benchmarks/pull-request-candidate-v2 && sha256sum -c SHA256SUMS)
```

For inspection:

```text
python3 -B benchmarks/pull-request-candidate-v2/verify.py derive-oracle
python3 -B benchmarks/pull-request-candidate-v2/verify.py summary
```

Passing this bundle demonstrates agreement with the frozen candidate-selection contract. It does
not demonstrate live collector conformance, GitHub-wide failure frequency, production accuracy,
developer adoption, willingness to pay, or product-market fit.

## Primary references

- [GitHub required-check target troubleshooting](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks#conflicts-between-head-commit-and-test-merge-commit)
- [GitHub REST: Get a pull request](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)
- [GitHub merge queues](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue)
- [GitHub GraphQL: MergeQueueEntry](https://docs.github.com/en/graphql/reference/objects#mergequeueentry)
- [GitHub Actions `merge_group`](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#merge_group)
