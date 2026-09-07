# Review Governor benchmark tool

This directory contains a deterministic scheduler replay, a separately implemented metrics oracle,
the public-GitHub freezer, and self-tests for `review-governor-benchmark-v0`.

Run the offline verification:

```bash
python3 -B tools/review-governor-benchmark/verify.py \
  benchmarks/review-governor-benchmark-v0
python3 -B -m unittest discover \
  -s tools/review-governor-benchmark \
  -p 'test_*.py'
```

Reconstruct the trace from immutable commits and public GitHub events (authenticated `gh` required):

```bash
python3 -B tools/review-governor-benchmark/freeze_github.py \
  --selection benchmarks/review-governor-benchmark-v0/selection-v0.json \
  --output benchmarks/review-governor-benchmark-v0/trace-v0.json \
  --check
```

Reconstruct the bounded provider fingerprint and command-route evidence:

```bash
python3 -B tools/review-governor-benchmark/freeze_provider_contract.py \
  --trace benchmarks/review-governor-benchmark-v0/trace-v0.json \
  --output benchmarks/review-governor-benchmark-v0/provider-contract-v0.json \
  --frozen-at 2026-09-06T13:52:58Z \
  --check
```

The online check may take several minutes because it downloads and hashes every selected
`base SHA...head SHA` diff. A mismatch is surfaced as a hard failure. The freezer has no silent
fallback from a missing check suite or unavailable diff.

`evaluator.py` emits a per-run ledger. `oracle.py` deliberately does not import it and independently
replays the policy contract into the compared metrics. `verify.py` checks byte hashes, regenerates
both outputs, compares every case × policy metric, and enforces accounting invariants.
It also verifies every frozen provider identity and body hash, recomputes review/status and
acknowledgement-update skew, checks the three PR-conversation command successes, and preserves the
separate inline-thread bot-skip boundary.

The benchmark's `billed_invocation_proxy` is exactly one unit per dispatched reviewer invocation.
It is not a dollar amount, token count, vendor invoice, or claim about how cancelled work is billed.
