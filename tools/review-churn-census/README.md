# Review Churn Census v1 CLI

This standard-library-only CLI measures how often a GitHub review checkpoint no longer matches
the pull request's final head. It implements the frozen five-stage census pipeline:

```text
sampling-plan.json -> sample.json -> capture.json -> manifest.json -> aggregate.json
```

Only `sample` and `capture` access GitHub. They invoke the authenticated `gh api graphql`
command without placing a token in an argument or artifact. Every connection is paginated to
completion. Sampling recursively partitions the fixed merge window until every GitHub Search
shard is within the 1,000-result limit; one overflowing UTC day fails explicitly.

Run the pipeline from the repository root:

```console
python3 tools/review-churn-census/review_churn_census.py sample
python3 tools/review-churn-census/review_churn_census.py capture
# If capture was interrupted, validate and continue its exact checkpoint:
python3 tools/review-churn-census/review_churn_census.py capture --resume
python3 tools/review-churn-census/review_churn_census.py classify
python3 tools/review-churn-census/review_churn_census.py evaluate
python3 tools/review-churn-census/review_churn_census.py verify
```

Outputs default to `target/review-churn-census-v1/`. Use each command's `--help` to override
paths. Writes use a same-directory temporary file, `fsync`, and atomic replacement. API errors,
pagination inconsistencies, schema drift, missing selected PRs, duplicate JSON keys, hash
mismatches, and noncanonical frozen artifacts fail visibly.
The v1 executable also pins the canonical sampling-plan SHA-256, so changing narrative definitions,
the panel, thresholds, or analysis rules requires a versioned protocol and tool update.

`capture` saves a canonical, atomically replaceable checkpoint after each PR. `--resume` accepts
only a validated prefix for the exact sample and continues with the first missing case. Actor
logins are used only in memory and are replaced before persistence with PR-local opaque keys;
titles, review bodies, source code, and credentials are never collected.

## One-repository audit

`audit` applies the census capture and classification semantics to the newest merged pull requests
in one bounded repository window. It does not use the frozen panel or persist raw capture data:

```console
python3 tools/review-churn-census/review_churn_census.py audit \
  --repository OWNER/REPO \
  --hostname github.com \
  --limit 50 \
  --days 90 \
  --end-exclusive 2026-09-01T00:00:00Z
```

The default output is Markdown on stdout. Use `--format json` for the strict
`stratadiff-review-memory-audit-v2` report, or `--output PATH` to write either format atomically
with mode `0600`. A supplied `--end-exclusive` makes the half-open scan window reproducible.
Completed scans exit successfully with one of four report statuses: `no_eligible_reviews`,
`insufficient_evidence`, `no_observed_drift`, or `affected`. Missing checkpoint evidence is unknown,
not evidence of no drift. Drift findings include the reviewer's GitHub login and immutable user
node ID so the reviewer is actionable and remains identifiable across pull requests. Missing or
conflicting reviewer identity fails the audit instead of publishing an ambiguous finding.

The completed public v1 artifacts, observed metrics, signal decisions, and claim boundary are in
[`../../benchmarks/review-churn-census-v1/`](../../benchmarks/review-churn-census-v1/). They can be
verified offline with the same `verify` command by passing the four checked-in artifact paths.

## Frozen protocol

- Half-open merge window: `2026-06-03T00:00:00Z` to `2026-09-01T00:00:00Z`.
- Equal-quota panel: 10 named repositories, 50 deterministically selected PRs each, target 500.
- Global gates: complete capture with zero failures, at least 400 PRs, at least 8 repositories at
  target, and at least 200 PRs with a completed external peer review.
- Signal gates: denominator at least 100; head-drift continuity also requires at least 9,000
  basis points of checkpoint/final-head OID observability.

The primary unit is one PR by one external peer reviewer. A completed session is current
`APPROVED` or `CHANGES_REQUESTED`, or `DISMISSED` when its linked dismissal event records either
state as `previousReviewState`. Each pair uses its latest semantic completed session; a missing
commit OID never falls back to an earlier session. `COMMENTED` is partial-attention evidence only
and never replaces a completed checkpoint or establishes approval, completion, or team coverage.

## Metrics and decisions

The aggregate publishes these 13 descriptive metrics, both pooled and by repository:

1. `formal_peer_reviewed_pr_rate`
2. `completed_review_pr_rate`
3. `checkpoint_oid_observability_rate`
4. `checkpoint_pair_head_drift_rate`
5. `completed_review_pair_post_force_push_rate`
6. `checkpoint_pair_drift_without_observed_force_push_rate`
7. `stranded_reviewer_pr_rate`
8. `multi_round_completed_review_pr_rate`
9. `completed_review_dismissal_pr_rate`
10. `commented_only_pair_share`
11. `commented_newer_commit_candidate_pair_rate`
12. `completed_review_pair_force_push_rereview_rate`
13. `bot_review_session_share`

Every nonempty metric includes its integer numerator and denominator, an integer-basis-point point
estimate, and an outward-rounded two-sided Wilson 95% interval. Three product signals
(`force_push_wedge`, `all_round_review_continuity`, and `commented_partial_attention`) are
`evaluable` only after their gates pass. A signal is `pass` only when its Wilson lower bound meets
the frozen threshold, `fail` only when its upper bound is below it, and otherwise
`inconclusive`. These decisions describe the fixed panel and period; they do not establish causal
product effects or GitHub-wide prevalence.

Run the offline tests with:

```console
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/review-churn-census -p 'test_*.py' -v
```
