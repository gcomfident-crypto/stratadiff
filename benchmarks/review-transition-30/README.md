# ReviewTransition-30 provider observation

[`provider-observation-v1.json`](provider-observation-v1.json) freezes the bounded GitHub REST
observation collected on 2026-09-06 for all 30 cases selected by the canonical ReviewTransition-30
plan. It records each merged pull request's requested base Q and frozen checkpoint/head B/D, plus
the summary and raw-response SHA-256 for Q...B and Q...D Compare requests. Raw provider responses,
tokens, login names, review text, comments, and source code are not included.

## Frozen bindings

| Artifact | SHA-256 |
|---|---|
| Canonical transition plan (deterministically regenerated) | `d0287a0319c7e2dd7298e7a141b00be0dbfc2db8dfd75ac2a42259ceb072fdb9` |
| Evaluation protocol | `b804e08ff34d69e4e4d7820abe35d9bb27b82165db6a5f5af3dbd2177c29c194` |
| Provider observation | `4c2a5e813c905016993e540578c339185cafc8fbc0c73360d9722edca50995b5` |

All 30 observations completed: 11 are `provider_attested_same_base`, 19 are
`requires_full_ancestry`, and 0 are `provider_metadata_unavailable`. This routing was frozen before
Git materialization or product evaluation and cannot be changed in response to a product outcome.

Reproduce the plan and verify the observation from the repository root:

```console
python3 -B tools/review-transition/review_transition.py select \
  --count 30 \
  --output /tmp/review-transition-30.json
python3 -B tools/review-transition/review_transition_eval.py verify-observation \
  --plan /tmp/review-transition-30.json \
  --observation benchmarks/review-transition-30/provider-observation-v1.json
```

Successful verification proves the artifact's internal bindings and exact canonical bytes against
the frozen plan and protocol. It does not authenticate GitHub as the historical publisher. The
provider exposes only one merge-base candidate per comparison, so this artifact does not prove A
or C. The 19 ancestry-routed cases still require offline-complete Git history and unique
`git merge-base --all` results; none is currently a product pass or an acquisition failure.
