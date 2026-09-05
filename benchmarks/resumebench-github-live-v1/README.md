# ResumeBench-GitHub-Live v1

ResumeBench-GitHub-Live v1 records five public GitHub pull-request histories in which a human
submitted an `APPROVED` or `CHANGES_REQUESTED` review and the PR branch was force-pushed afterward.
The cases exercise StrataDiff's review-resume policy against real, no-longer-advertised review
commits. The capture date is **2026-09-05**.

This is a purposefully selected diagnostic set, not a random or representative sample. It can test
whether the implementation reproduces its strict file-carry policy on these histories. It cannot
estimate how often force-pushes occur, how much reviewer time the product saves, or how accurately
its residue matches human attention priorities.

## Claim boundary

There is **no human-priority ground truth** in v1. `needs_review_now` means only that StrataDiff did
not establish one of its accepted carry proofs. It does not mean that a human judged the file
important, risky, or semantically changed. Conversely, a carried file is backed by the policy's Git
identity or non-interacting four-way byte-replay evidence; it is not a semantic-safety guarantee and
does not restore or grant a GitHub approval.

The checked-in counts are reproduced by the frozen independent per-file oracles and separately
matched by the pinned clean release build named in [`manifest.json`](manifest.json). They are policy
conformance ground truth, not human-priority ground truth. Do not generalize percentages from these
five deliberately selected cases.

## Five-snapshot model

Each case preserves five Git roles:

```text
Q = requested base revision supplied to StrataDiff
A = merge-base(Q, B), the checkpoint merge base
B = commit attached to the selected historical human review
C = merge-base(Q, D), the current PR merge base
D = captured final PR head

reviewed checkpoint delta = A..B
current PR delta          = C..D
```

`Q` and `C` are intentionally distinct roles. They happen to be equal in four cases; Ruff #28311
demonstrates why a verifier must still derive and check `C`. Multi-commit PRs also mean a verifier
must not assume that `A` is the parent of `B` or that `C` is the parent of `D`.

The selected review is historical and immutable in the manifest. Beets #6990 and Ruff #28311 later
received another eligible review from the same reviewer at `D`; asking GitHub for the reviewer's
latest review today would therefore select `D`, not the checkpoint under test.

## Observed diagnostic results

| Case | Current | Exact carry | Four-way carry | Needs review | Retired | Naive `B..D` paths |
|---|---:|---:|---:|---:|---:|---:|
| PostHog/posthog #95077 | 9 | 0 | 0 | 9 | 52 | 1,658 |
| opensearch-project/agent-health #481 | 26 | 21 | 4 | 1 | 2 | 84 |
| jellyfin/jellyfin-kodi #1178 | 3 | 0 | 1 | 2 | 2 | 75 |
| beetbox/beets #6990 | 5 | 2 | 1 | 2 | 2 | 10 |
| astral-sh/ruff #28311 | 4 | 0 | 0 | 4 | 9 | 11 |
| **Total** | **47** | **23** | **6** | **18** | **67** | **1,838** |

The policy established carry evidence for 29 of the 47 current PR files and left 18 in residue.
A naive path set from the obsolete review commit directly to the final head, `B..D`, contained 1,838
paths: 1,815 were not current `C..D` PR paths, while 24 current PR paths were absent from that naive
set. “Extra” and “missing” here describe path-set disagreement with `C..D`, not a human relevance
label.

## Online and offline boundary

Online provenance verification should use exact, bounded GitHub endpoints:

- fetch the exact PR and exact review database ID recorded in the manifest;
- require the review's commit to equal `B`, and verify the review state, account type, and timestamp;
- verify each recorded `HeadRefForcePushedEvent` by GraphQL node ID, timestamp, and before/after OIDs;
- verify commit objects through the GitHub commit API and fetch Q, B, and D only by full object ID;
- derive A and C from the fetched graph, with no fallback to a branch tip, current PR head, mirror, or
  different commit.

The CLI's `verify-provenance` command implements the API checks above. It deliberately does not
compare the mutable PR `updated_at` field. A later change to a live PR base/head snapshot or reviewer
association is reported as external provenance drift, not as a StrataDiff engine mismatch. The
scheduled workflow is named **ResumeBench GitHub Live Canary**. Negative capture-time observations
such as “B was not an advertised ref tip” and “this was the latest eligible review at capture” are
retained assertions; they cannot be reconstructed later because this bundle intentionally omits raw
API responses and an advertised-ref snapshot.

GitHub does not promise permanent retention of orphaned commits. A provider 404 or exact-SHA fetch
failure is external upstream unavailability, not an engine mismatch; the CLI exits nonzero and must
never substitute another SHA. Current review lists can also gain later reviews, so they
must not replace the manifest's exact historical review ID.

Offline evaluation begins only after the required graph, trees, and blobs have been materialized.
That blob closure includes both review ranges and the A/B/C/D snapshots of every checkpoint path,
so retired-change fallback rows remain displayable without a promisor remote. Verification removes
all remotes, sets `GIT_NO_LAZY_FETCH=1`, recomputes both merge bases and both raw Git change sets,
validates the review-delta source closure and any four-way replay witnesses, and compares every file
state and carry basis against a frozen oracle. CI should run bundle/schema checks, tamper tests, and
offline evaluation; live GitHub checks belong in a non-blocking canary because API availability and
object retention are external state.

The bundled CLI implements that split. Only `verify-provenance` and `materialize` use the network:

```console
python3 tools/resumebench-github-live/resumebench_github_live.py self-test
GITHUB_TOKEN=... python3 tools/resumebench-github-live/resumebench_github_live.py verify-provenance \
  --manifest benchmarks/resumebench-github-live-v1/manifest.json \
  --github-token-env GITHUB_TOKEN
python3 tools/resumebench-github-live/resumebench_github_live.py materialize \
  --manifest benchmarks/resumebench-github-live-v1/manifest.json \
  --output /tmp/resumebench-github-live-v1
```

The output contains five independent bare repositories and no remotes. All following commands run
with lazy fetching disabled:

```console
python3 tools/resumebench-github-live/resumebench_github_live.py verify \
  --materialization /tmp/resumebench-github-live-v1

python3 tools/resumebench-github-live/resumebench_github_live.py evaluate \
  --materialization /tmp/resumebench-github-live-v1 \
  --stratadiff target/release/stratadiff \
  --output /tmp/resumebench-github-live-evaluation.json

python3 tools/resumebench-github-live/resumebench_github_live.py verify-bundle
```

Maintainers use `freeze` instead of `evaluate` to regenerate the five oracles, the canonical
`evaluation-v1.0.0.json`, and `SHA256SUMS`. `evaluate` and `freeze` reject a dirty or non-release
StrataDiff build. A token for higher GitHub API limits can be passed without entering command-line
history by naming its environment variable, for example `--github-token-env GITHUB_TOKEN`.

No source text, patch text, review body, email address, avatar, or raw API response is checked in.
The manifest retains only public identifiers and timestamps needed to reproduce provenance, plus
license-file hashes observed through GitHub. Because the source repositories have different license
terms, this dataset does not publish a combined Git bundle.
