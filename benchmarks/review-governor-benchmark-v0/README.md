# Review Governor benchmark v0

This is a reproducible, deliberately small experiment for one product question: can a stable-head
governor reduce avoidable reviewer invocations while still covering the final PR head?

## Frozen evidence

The selection starts from `acoliver/vibetools@d57789d`, whose frozen
`review-events.csv` hashes to
`220b894ba84340536e9101ae3f31aafea45f9bb15f16a44bfbb97f53c72aa08a`. It then freezes the public
GitHub timelines for three high-churn PRs:

- `vybestack/llxprt-jefe#30`
- `vybestack/llxprt-code#1947`
- `vybestack/llxprt-code#2011`

The fixture contains 55 head observations and 36 CodeRabbit review events. A force-pushed head uses
the public `head_ref_force_pushed` event time. Other heads use the earliest public GitHub Actions
check-suite creation time; if that check predates PR creation, PR creation is the lower bound. Every
head includes the SHA-256 and byte length of GitHub's raw immutable `base SHA...head SHA` diff.

`provider-contract-v0.json` independently freezes the public provider objects behind those 36
review events without copying review prose. All 36 use bot id `136622811`, carry CodeRabbit's
substantive-review marker, and pair with a successful `Review completed` status whose integration
avatar is `/in/347564`; observed completion skew is 1–18 seconds. The five-minute governor bound is
therefore conservative for this sample, not a promise about future provider behavior.

The same contract also records three public cases where `github-actions[bot]` posted
`@coderabbitai full review` in the PR conversation, CodeRabbit emitted a command-invocation
acknowledgement, and a substantive review followed. The acknowledgement's final update followed
the submitted review by 3–16 seconds in these cases, supporting the adapter's bounded
`Full review finished.` update correlation. A separate inline review-thread case records
CodeRabbit's `Skipped: comment is from another GitHub bot` response. These routes must not be
conflated: the evidence supports bot-authored PR conversation commands in the sampled deployment,
not arbitrary bot replies inside CodeRabbit review threads.

The event model has three external events:

1. `head`: a new PR head and exact whole-diff identity became publicly observable.
2. `observed_review`: CodeRabbit submitted a public review bound to a head SHA. These observations
   calibrate one fixed per-case replay duration; policies do not copy the observed dispatch choices.
3. `finalize`: the PR merged with the named final head.

At equal timestamps, the replay settles a review completion, then a timer dispatch, then the external
event. All pending work is drained after `finalize`, so `final_head_covered` means eventual coverage;
`final_head_covered_at_finalize` is the stricter pre-merge observation.

## Compared policies

| Policy | Dispatch rule | Obsolete work | Exact whole-diff reuse | Final signal |
| --- | --- | --- | --- | --- |
| `per_push` | immediately on every head | continues | no | none |
| `github_concurrency_5m_whole_diff` | after five quiet minutes | cancel | completed hashes only | none |
| `governor_stable_head_final_head` | after 15 quiet minutes | cancel | completed hashes only | flush current head |

The second row is a composite CI baseline. GitHub concurrency alone does not provide debounce or
whole-diff hashing.

## Frozen v0 result

| Policy | Dispatched | Completed | Cancelled | Superseded | Billed invocation proxy | Work-seconds proxy | Final heads covered | Covered at finalize |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Per push | 55 | 55 | 0 | 4 | 55 | 25,831 | 3/3 | 2/3 |
| Concurrency + 5m + hash | 54 | 35 | 19 | 19 | 54 | 22,801 | 3/3 | 2/3 |
| Governor stable + final | 24 | 15 | 9 | 9 | 24 | 11,775 | 3/3 | 2/3 |

In this fixed sample the Governor dispatches 31 fewer invocations than per-push (56.4%) and 30 fewer
than the composite baseline (55.6%), while all three policies eventually cover all three final heads.
This is a benchmark observation, not an estimated production saving. In particular, v0 does not show
better coverage at merge: every policy is 2/3 there.

## What the numbers do not prove

- The three cases are purposively selected high-churn PRs from one organization, not a random or
  representative population. The percentages must not be generalized.
- A check-suite timestamp is a public server-observation proxy, not the original private webhook
  delivery time. The trace is strongest for explicit force-push events.
- The per-case duration is the median public head-to-review-submission lag. It mixes queueing,
  inference, and posting time and is not measured compute.
- `billed_invocation_proxy` counts every dispatch, including a cancelled run. It is not real cost,
  tokens, or vendor billing. `work_seconds_proxy` is simulated active time, not observed resource use.
- The retrospective merge event stands in for an explicit final-head signal. A deployable Governor
  needs a required pre-merge check or merge-queue handshake; a post-merge flush is too late to gate
  defects.
- The raw GitHub diff hash proves byte identity under this renderer. It does not establish semantic
  equivalence across rebases, reviewer correctness, defect recall, latency acceptability, or adoption.
- The replay omits API failures, rate limits, reviewer capacity pools, parallel PR contention, and
  vendor-specific cancellation/billing rules.

## Verify

```bash
python3 -B tools/review-governor-benchmark/verify.py \
  benchmarks/review-governor-benchmark-v0
python3 -B -m unittest discover \
  -s tools/review-governor-benchmark \
  -p 'test_*.py'
```

Reconstruct the provider contract from the named public objects:

```bash
python3 -B tools/review-governor-benchmark/freeze_provider_contract.py \
  --trace benchmarks/review-governor-benchmark-v0/trace-v0.json \
  --output benchmarks/review-governor-benchmark-v0/provider-contract-v0.json \
  --frozen-at 2026-09-06T13:52:58Z \
  --check
```

`SHA256SUMS` covers the selection, policies, trace, provider contract, evaluator output, oracle output,
and manifest.
