# Review Churn Census v1

Review Churn Census v1 is a **pilot-informed, prospective randomized panel** for one product-scope
question:

> In a fixed set of review-heavy GitHub repositories, how often does a completed reviewer-specific
> checkpoint fall behind the final PR head, and how much of that churn is or is not accompanied by
> an observed force-push event?

This is not an outcome-naive preregistration or an independent confirmatory study. Two convenience
probes and four semantic cases informed the metrics and thresholds before this plan was frozen.
The complete disclosure is machine-readable in [`sampling-plan.json`](sampling-plan.json) and
summarized below. The full v1 PR sample is still prospectively selected by a frozen hash rank; its
selection may nevertheless overlap previously inspected public PRs.

The census can screen which product hypothesis deserves a human experiment. It cannot demonstrate
reviewer time saved, issue recall, safe carry, willingness to pay, market size, or product-market
fit.

## Evidence seen before the freeze

The two probes must not be pooled: their repositories, selection procedures, units, and some actor
and checkpoint definitions differ. `PostHog/posthog` occurs in both, exact PR overlap was not frozen,
and the v1 randomized sample may select previously inspected PRs.

| Probe | Repositories and selection | Observations |
|---|---|---|
| Four-repository latest-25 | `PostHog/posthog`, `rust-lang/rust`, `microsoft/vscode`, `github/gh-stack`; latest-created 25 merged PRs per repository | External `APPROVED` or `CHANGES_REQUESTED`: 57/100. Later force-push: 8/57. Adding external `COMMENTED` created zero newly eligible units, but that probe did not measure a newer COMMENTED commit after an existing checkpoint and did not preserve its uplift denominator. |
| Four-repository newest-50 convenience sample | `astral-sh/ruff`, `zephyrproject-rtos/zephyr`, `PostHog/posthog`, `kubernetes/kubernetes`; newest 50 merged PRs per repository | `APPROVED` or `CHANGES_REQUESTED`: 136/200. Later force-push: 32/136. Selected review commit differed from final head: 40/136. This was not the frozen external peer PR × reviewer estimator. |

`home-assistant/core#181236`, `PostHog/posthog#95380`, `PostHog/posthog#95326`, and
`rust-lang/rust#162237` were separately inspected to understand author self-review, dismissed-state,
COMMENTED recency, final-head OIDs, and timeline semantics. They are semantic cases, not frequency
samples.

The screening thresholds were chosen after these observations: 10% for an observed post-review
force-push, 20% for broader drift, and 15% for a newer COMMENTED partial-attention candidate. The
first two sit below exploratory point estimates of 8/57 and 32/136, and 40/136 respectively. The
COMMENTED threshold follows a zero-uplift probe whose definition missed recency. These values are
therefore pilot-informed product choices, not statistical or market truths.

## Population and randomized selection

The estimand is limited to API-visible public PRs merged from 2026-06-03 00:00:00 UTC through, but
not including, 2026-09-01 00:00:00 UTC in ten named repositories. Repositories were purposefully
selected for target-segment relevance; they were not sampled from GitHub. Within each repository,
50 PRs are selected without replacement, for a target of 500.

The sampler enumerates the API-visible eligible frame. Search date ranges are recursively divided
into non-overlapping whole-day ranges until each exposes no more than GitHub's 1,000-result limit.
Every Search result is checked in place against repository identity, node identity, merged state,
and the exact half-open window. Each selected PR is then independently fetched during capture and
must match the frozen node ID, number, and merge time. Duplicate nodes, changing counts, incomplete
pagination, a selected-object mismatch, or a single day above the provider cap stop collection.
Non-selected Search results are not independently re-fetched, so an indexing defect could affect
the randomized frame; the result describes the provider-visible frame rather than an external
ground-truth inventory.

Candidate rank is:

```text
SHA-256(seed_bytes || 0x00 || casefold(nameWithOwner) || 0x00 || decimal(PR number))
```

The lowest 50 digests per repository win; PR number breaks a hypothetical digest tie. The frozen
rank cannot use reviews, actors, commits, force-pushes, dismissals, comments, diffs, or pilot
outcomes. A short repository contributes all candidates and remains a disclosed shortfall. No PR,
repository, or time range may be replaced after seeing outcomes.

This equal-quota panel describes only its repositories and 90-day window. It excludes private,
open, abandoned, and closed-unmerged PRs, other hosts, and review work outside formal GitHub review
objects. It must not be extrapolated to GitHub as a whole.

## Observable review contract

The only observed human-review unit is a submitted, non-`PENDING` GitHub `PullRequestReview`.
Standalone PR issue comments, commit comments, local or IDE review, chat, and unsubmitted attention
are invisible. Even `APPROVED` does not prove that every line was read.

A primary peer is a GitHub `User` whose PR-local actor key differs from the PR author's key. Author
self reviews, `Bot` actors, other actor types, and missing actors are reported separately. Login
suffix heuristics are forbidden.

A completed session is:

- a current `APPROVED` or `CHANGES_REQUESTED` review; or
- a current `DISMISSED` review only when its linked `ReviewDismissedEvent.previousReviewState` was
  `APPROVED` or `CHANGES_REQUESTED` and the event is not earlier than the review submission.

`COMMENTED` is formal partial-attention evidence, never a completed review, approval, CODEOWNER
coverage, or permission to suppress re-review. A dismissed review's current state alone is also
insufficient; the captured previous state is required.

The checkpoint unit is one PR × external peer reviewer. Completed sessions in that pair are ordered
by `submittedAt`, then positive `fullDatabaseId`; the last is the latest completed checkpoint.
Missing checkpoint or final-head OIDs remain missing. The analyzer never substitutes an earlier
review.

Force-push metrics inspect every completed session in a pair. A pair counts as post-force-push when
at least one event occurs strictly after any completed session. It counts as force-push re-review
only when the same reviewer has the sequence below. These two semantic timing metrics do not require
a review commit OID; OIDs are required for head drift and COMMENTED commit advancement.

```text
completed_before.submittedAt < force_event.createdAt < completed_after.submittedAt
```

Only final-head drift uses the latest completed checkpoint. “Without observed force-push” means no
qualifying event was present after that latest checkpoint in the completely captured timeline;
earlier force-pushes do not explain the final drift. It does not prove that the head changed through
an ordinary append-only push.

## Frozen metrics

Every metric publishes an integer numerator and denominator, basis-point point estimate, two-sided
95% Wilson interval, denominator-adequacy flag, and repository breakdown.

| Metric | Numerator / denominator |
|---|---|
| `formal_peer_reviewed_pr_rate` | Selected PRs with an external peer submitted review / all selected PRs |
| `completed_review_pr_rate` | Selected PRs with a semantic completed-review pair, regardless of OID visibility / all selected PRs |
| `checkpoint_oid_observability_rate` | Completed-review pairs with latest checkpoint and final-head OIDs / all semantic completed-review pairs |
| `checkpoint_pair_head_drift_rate` | Comparable latest checkpoints differing from final head / OID-comparable completed pairs |
| `completed_review_pair_post_force_push_rate` | Semantic completed-review pairs with a force-push after any completed session / all semantic completed-review pairs |
| `checkpoint_pair_drift_without_observed_force_push_rate` | Comparable drift pairs with no observed force-push after the latest checkpoint / OID-comparable completed pairs |
| `stranded_reviewer_pr_rate` | Fully pair-comparable completed PRs with at least one drifted reviewer / completed PRs whose every pair is comparable |
| `multi_round_completed_review_pr_rate` | Completed PRs whose peer completed sessions span at least two commit OIDs / completed-checkpoint PRs |
| `completed_review_dismissal_pr_rate` | Completed PRs where a pair's latest completed session is a qualifying dismissed review / completed-checkpoint PRs |
| `commented_only_pair_share` | Peer pairs with a non-null-commit COMMENTED session but no completed session / union of completed and COMMENTED-candidate pairs |
| `commented_newer_commit_candidate_pair_rate` | OID-visible completed pairs followed by same-reviewer COMMENTED on a different commit / OID-visible completed pairs |
| `completed_review_pair_force_push_rereview_rate` | Same-reviewer semantic completed re-review after a qualifying force-push / post-force-push semantic pairs |
| `bot_review_session_share` | Submitted Bot review sessions / submitted external-peer User plus Bot sessions |

The force-push re-review measure is diagnostic evidence about repeated review work; it has no
product pass threshold. COMMENTED measures are candidates for an explicit confirmation flow only.
Bot share cannot identify repeated, correct, or low-quality findings because no review text is
retained.

## Wilson decisions and gates

Point estimates are rounded to integer basis points, half away from zero. For denominator `n > 0`,
the evaluator computes a two-sided 95% Wilson score interval without continuity correction using
`z = 1.959963984540054`. The lower endpoint is floored and the upper endpoint is ceiled after
multiplication by 10,000. A zero denominator produces an undefined point and interval, never zero.

The panel gate requires all of:

- at least 400 sampled PRs;
- at least eight repositories reaching 50 PRs;
- at least 200 PRs with a completed peer checkpoint;
- zero capture failures.

Every metric reports whether its denominator is at least 100. Each product signal requires that
minimum. All-round continuity additionally requires at least 90% of semantic completed-review pairs
to expose both the latest checkpoint and final-head OIDs. The COMMENTED candidate metric uses its
own OID-visible denominator and does not require final-head observability. The force-push signal
requires complete timeline and timestamp capture.

After every applicable gate passes:

- `force_push_wedge` uses `completed_review_pair_post_force_push_rate` at 10%;
- `all_round_review_continuity` uses
  `checkpoint_pair_drift_without_observed_force_push_rate` at 20%;
- `commented_partial_attention` uses `commented_newer_commit_candidate_pair_rate` at 15%.

A signal is `pass` only when its Wilson lower bound meets or exceeds the threshold. It is `fail`
only when its Wilson upper bound is below the threshold. An interval crossing the threshold, an
insufficient denominator, or any failed gate is `inconclusive`. A gate failure is never silently
reported as product failure.

These signals select the next experiment; they do not validate the product. Reviewer value still
requires the separate counterbalanced human study with time, issue-recall, false-carry, and repeat-
use outcomes.

## Five-stage artifact pipeline

The reproducible chain contains five artifacts:

```text
sampling-plan.json -> sample.json -> capture.json -> manifest.json -> aggregate.json
```

- `sampling-plan.json` freezes disclosures, frame, rank, definitions, and analysis.
- `sample.json` freezes the complete candidate inventories and selected PRs; its independent
  contract is [`sample.schema.json`](sample.schema.json).
- `capture.json` stores the bounded, privacy-minimized provider facts; its contract is
  [`capture.schema.json`](capture.schema.json).
- `manifest.json` contains per-PR classifications under
  [`manifest.schema.json`](manifest.schema.json).
- `aggregate.json` contains rates, Wilson intervals, gates, and signals under
  [`aggregate.schema.json`](aggregate.schema.json).

Each artifact pins its upstream bytes. The CLI rejects duplicate JSON keys, validates its required
closed structure field by field, and deterministically recomputes derived artifacts. It does **not**
load or execute these JSON Schema files at runtime. The closed Draft 2020-12 schemas are independent
consumer contracts and were checked against CLI-produced fixtures when v1 was frozen; arithmetic,
ordering, hashing, pagination, and temporal invariants remain verifier responsibilities.

Only `sample` and `capture` use the network. The collector invokes an authenticated `gh`; automation
may supply `GH_TOKEN` through the environment, never as an argument or artifact:

```console
GH_TOKEN=... python3 tools/review-churn-census/review_churn_census.py sample \
  --plan benchmarks/review-churn-census-v1/sampling-plan.json \
  --output /tmp/review-churn-sample.json

GH_TOKEN=... python3 tools/review-churn-census/review_churn_census.py capture \
  --plan benchmarks/review-churn-census-v1/sampling-plan.json \
  --sample /tmp/review-churn-sample.json \
  --output /tmp/review-churn-capture.json

python3 tools/review-churn-census/review_churn_census.py classify \
  --plan benchmarks/review-churn-census-v1/sampling-plan.json \
  --sample /tmp/review-churn-sample.json \
  --capture /tmp/review-churn-capture.json \
  --output /tmp/review-churn-manifest.json

python3 tools/review-churn-census/review_churn_census.py evaluate \
  --plan benchmarks/review-churn-census-v1/sampling-plan.json \
  --sample /tmp/review-churn-sample.json \
  --capture /tmp/review-churn-capture.json \
  --manifest /tmp/review-churn-manifest.json \
  --output /tmp/review-churn-aggregate.json

python3 tools/review-churn-census/review_churn_census.py verify \
  --plan benchmarks/review-churn-census-v1/sampling-plan.json \
  --sample /tmp/review-churn-sample.json \
  --capture /tmp/review-churn-capture.json \
  --manifest /tmp/review-churn-manifest.json \
  --aggregate /tmp/review-churn-aggregate.json
```

Do not publish a partial capture as an observed result. A post-freeze change requires a versioned
amendment, not an in-place rewrite that hides the original rule.

## Privacy and release boundary

The API processor reads GitHub logins only in memory. Before constructing `capture.json`, it derives
the same PR-local key for the PR author and every review actor:

```text
actor- + first24hex(SHA-256(case_id || 0x00 || casefold(login)))
```

The artifact retains that opaque key and actor typename, never the login. It also omits access
tokens, authorization headers, names, emails, avatars, PR titles and bodies, review/comment text,
source, patches, diffs, and raw API responses. A conforming privacy-minimized capture may be checked
in for reproducibility.

This key is deterministic and unsalted within a named public PR. Repository and PR identifiers are
retained, so public metadata may permit reidentification. The dataset is pseudonymous, **not
anonymous**, and makes no anonymity claim. The aggregate contains no actor keys or per-reviewer
records.
