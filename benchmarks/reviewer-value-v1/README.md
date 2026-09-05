# Reviewer Value v1

This evaluation asks a narrower product question than the correctness suites:

> On real force-pushed pull-request histories, how much current file-level re-review can strict
> carry avoid, and how badly does a naive checkpoint-to-head path diff distort the review surface?

It derives its counts from the independently frozen oracles and clean release evaluation in
`resumebench-github-live-v1`. The five histories are purposefully selected diagnostics, not a
random or representative sample.

## Result

- 29 of 47 current PR files had exact carry evidence, a 61.70% file-level recheck reduction.
- 18 current files remained in the human review queue.
- The naive checkpoint-to-head comparison exposed 1,815 paths outside the current PR and omitted
  24 current paths across the same cases.
- The complete exact resume queue also retained 67 dropped or reverted checkpoint changes. This is
  intentional safety behavior, not proof that every retained item costs the same review time.
- Three of five cases carried at least one current file; two correctly abstained on every current
  file. The product is therefore useful on some histories and deliberately offers no reduction on
  others.

These numbers do **not** measure reviewer minutes, issue recall, semantic safety, market frequency,
or population-wide savings. A human study must establish those outcomes before StrataDiff claims
that it makes review faster or safer.

## Reproduce

```console
python3 tools/reviewer-value-v1/reviewer_value_v1.py verify
python3 tools/reviewer-value-v1/reviewer_value_v1.py evaluate \
  --output /tmp/reviewer-value-v1.json
cmp /tmp/reviewer-value-v1.json benchmarks/reviewer-value-v1/evaluation-v1.0.0.json
```

The evaluator rejects a source benchmark with failed cases, false carries, false invalidations,
an inconsistent current-file partition, duplicate JSON keys, or an unexpected schema/version. The
checked-in output pins the exact SHA-256 digest of its source evaluation.

## Go/no-go experiment

The next dataset must be prospectively sampled from at least 100 eligible re-review sessions across
at least 20 reviewers. The product hypothesis advances only if it shows all of the following:

- zero independently adjudicated false carries;
- at least 40% median reduction in current files reopened for review on eligible sessions;
- at least 20% median reduction in reviewer completion time;
- no decrease in issue recall relative to the ordinary GitHub review flow;
- at least 50% of invited reviewers use Review Resume again within four weeks.

Until that study exists, v1 is evidence that the mechanism can remove substantial mechanical noise
in selected histories, not evidence of product-market fit.
