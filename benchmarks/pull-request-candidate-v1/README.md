# Pull Request Candidate v1

Pull Request Candidate v1 is a deterministic, network-free benchmark for one question:

> Which exact Git object is GitHub currently evaluating for the required checks of this pull
> request: the PR head, a test-merge commit, or a merge-group commit?

The corpus freezes ten controlled cases and two read-only provenance captures made on 2026-09-08.
Verification never calls GitHub. The captures preserve only target identity and queue topology; they
do not preserve source, diffs, logs, credentials, or mutable API responses.

## Selection contract

The independent oracle implements these rules:

1. Reject a capture when the target identity changes between the first and final read. This includes
   the PR head or base, test-merge SHA, queue membership, queue-entry identity, state, position,
   base commit, or head commit.
2. When the PR is in a merge queue, select the current GraphQL queue entry's `headCommit`. A missing
   entry is inconclusive. Never substitute the PR head, REST `merge_commit_sha`, or an older queue
   candidate.
3. Outside a merge queue, inspect both Check Runs and legacy commit statuses on the test-merge SHA.
   If either surface contains a signal, select the test-merge commit.
4. A missing test-merge SHA, an incomplete signal surface, or complete but empty test-merge signals
   is inconclusive. The corpus does not infer that the PR head is active from absence alone.
5. `isMergeQueueEnabled` alone does not select a merge-group candidate. The PR must currently be in
   the queue and have a stable queue entry.

The selected SHA is only the input to required-check diagnosis. It is not proof that reviews,
conflicts, mergeability, compliance, or code safety are clear.

## Coverage

| Case | Controlled fact | Expected outcome |
|---|---|---|
| `test-merge-check-signal` | test merge has a Check Run | select test merge |
| `test-merge-status-signal` | test merge has only a legacy status | select test merge |
| `github-docs-empty-test-merge` | live-derived test merge has no rollup; queue is enabled but PR is not queued | inconclusive |
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
- `manifest.json` binds cases and oracle by SHA-256 and freezes coverage gates and claim limits.
- `verify.py` validates every field, derives the oracle independently, checks provenance controls,
  verifies checksums, and runs mutation tests.
- `SHA256SUMS` covers every other bundle asset. It is integrity metadata, not a signature.

## Offline verification

```text
python3 -B benchmarks/pull-request-candidate-v1/verify.py verify
python3 -B benchmarks/pull-request-candidate-v1/verify.py self-test
(cd benchmarks/pull-request-candidate-v1 && sha256sum -c SHA256SUMS)
```

For inspection:

```text
python3 -B benchmarks/pull-request-candidate-v1/verify.py derive-oracle
python3 -B benchmarks/pull-request-candidate-v1/verify.py summary
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
