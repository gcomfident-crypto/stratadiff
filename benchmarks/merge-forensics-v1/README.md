# MergeForensicsBench v1

MergeForensicsBench v1 is a deterministic, network-free benchmark for one narrow product
question:

> Given the raw GitHub evidence available while a pull request is in a merge queue, can the Pull
> Request Doctor identify the exact candidate and required-check producer, explain only what the
> evidence entails, and abstain or retry when that evidence is incomplete or unstable?

The corpus contains twelve controlled cases identified only as `c001` through `c012`. Its REST and
GraphQL bodies are constructed provider-shape fixtures, not live API captures or verbatim incident
records. Public incidents and official GitHub documentation motivate the failure families; they do
not label the fixture outcomes. Verification makes no network calls.

## Replay contract

`transcripts.json` records one strictly ordered exchange stream per case. REST and GraphQL calls
share the same sequence, so changing protocol, endpoint, variables, call order, status, headers, or
payload reference invalidates the transcript. Raw JSON bodies live in a deduplicated payload pool;
every body is bound to both its UTF-8 byte length and SHA-256 digest. The GraphQL operation is also
bound to the production query digest.

The Rust adapter consumes that single stream through `GithubReadinessApi` and
`GithubPullRequestDoctorApi`, then runs `collect_pull_request_doctor_snapshot_v3` and
`evaluate_pull_request_doctor_v3`. It therefore covers raw response bytes through collector and v3
report evaluation. It does not execute the `gh api --include` subprocess transport, authenticate to
GitHub, or exercise a live network.

The benchmark inputs and labels are separated by file:

- `cases.json` maps opaque case IDs only to transcript IDs. It contains no semantic labels,
  per-case provenance, or expected diagnosis.
- `transcripts.json` contains provider-shaped observations and no oracle fields.
- `oracle.json` contains ground truth, identifiability, allowed/forbidden behavior, and required
  evidence.
- `baseline-predictions.json` freezes the product adapter output used as the positive control.

The oracle, baseline, and this README are public regression artifacts, so the complete bundle is
not a participant-blind benchmark. Blind evaluation requires distributing only `cases.json` and
`transcripts.json` to the evaluated system and retaining the other assets with the evaluator.

The Python verifier independently reconstructs the candidate boundary, pinned requirement,
exact-SHA status, workflow inventory completeness, check-to-suite-to-run-to-job-to-workflow
binding, static producer uniqueness, merge-group trigger, and merge-group run evidence. Target
identity includes resolution, SHA, base SHA, queue-entry ID, and queue state. Requirement identity
includes the enforcing policy, and producer evidence includes source SHA, check/App identity, and
workflow-run path in addition to numeric linkage IDs. The verifier does not import production code.

## Controlled coverage

The manifest records coverage categories without mapping them to case IDs: complete unique
producer, merge-group run control, duplicate static producer, workflow-inventory cap, dynamic
matrix name, reusable workflow, exact job-link mismatch, candidate drift, producer drift, expected
App mismatch, default-branch-only definition, and unknown provider behavior. This preserves opaque
input identifiers while keeping corpus scope auditable.

Requirement-status coverage is intentionally limited to `missing` and `source_mismatch`. This
corpus makes no coverage claim for `satisfied`, `pending`, or `failed`; those statuses belong to the
separate required-check semantic benchmark.

The positive baseline makes three decisive predictions, seven abstentions, and two retries. All
ten report-producing cases remain `inconclusive` because the merge-group target comes from polling
and is provisional rather than webhook-cross-validated. An `inconclusive` report may still contain
a safely entailed workflow diagnosis.

## Metrics and gates

Rates are emitted in basis points, where 10,000 bp = 100%:

- selective coverage = decisive predictions / all cases
- selective accuracy = correct decisive predictions / decisive predictions
- abstention precision = correct abstentions / predicted abstentions
- abstention recall = correct abstentions / expected abstentions
- retry precision = correctly classified retries / predicted retries
- retry recall = correctly classified retries / expected retries
- evidence entailment = emitted evidence claims entailed by the transcript / emitted claims
- required-evidence recall = required claims recovered / required claims
- exact-target accuracy compares kind, provisional resolution, SHA, base SHA, queue-entry ID, and
  queue state where the target is identifiable
- requirement accuracy compares context, expected App, enforcing policy, and status where the
  requirement is identifiable
- false-confident rate counts decisive causes when cause identity is not supported or is wrong
- false-checks-clear rate counts forbidden `checks_clear` verdicts among unsafe report cases
- forbidden-action rate counts emitted repair actions outside the case allowlist

Every report must also retain the top-level `complete_collection` next action. Omitting it fails
per-case exactness even when every diagnosis field is otherwise correct.

Acceptance requires zero false-confident, false-checks-clear, and forbidden-action events; 100%
selective accuracy, abstention precision/recall, retry precision/recall, evidence entailment,
required-evidence recall, exact-target accuracy, and requirement accuracy; at least 2,500 bp
selective coverage; and exact agreement on all twelve cases.

## Bundle layout

- `manifest.json` binds the four machine-readable data assets by byte count and SHA-256, records
  source provenance and case coverage, and freezes the baseline metrics and claim boundary.
- `verify.py` enforces closed JSON shapes, canonical bundle JSON, unique keys, input leak controls,
  frozen payload identities, ordered replay traces, independent oracle derivation, scoring, and
  hard gates.
- `SHA256SUMS` covers every final bundle file except itself. The verifier requires the directory to
  contain exactly those regular files plus `SHA256SUMS`; extra files, directories, and symlinks are
  rejected. The checksum is not a publisher signature.

## Offline verification

```text
python3 -B benchmarks/merge-forensics-v1/verify.py verify
python3 -B benchmarks/merge-forensics-v1/verify.py self-test
python3 -B benchmarks/merge-forensics-v1/verify.py score benchmarks/merge-forensics-v1/baseline-predictions.json
(cd benchmarks/merge-forensics-v1 && sha256sum -c SHA256SUMS)
cargo test --test merge_forensics_v1
```

`score` accepts a semantically valid predictions JSON file without requiring canonical whitespace;
the frozen bundle assets themselves must use canonical JSON.

The two retry labels are emitted by the test adapter by classifying the current `anyhow` error
message (`target_drift` or `producer_drift`). They are not yet structured production error variants,
so changing those messages requires updating the adapter contract deliberately.

## Claim boundary

Passing the Python verifier demonstrates internal consistency among the controlled raw-provider
fixtures, independent derivation, oracle, manifest, and baseline. Passing the Rust adapter also
demonstrates that the production collector and evaluator reproduce those controlled outcomes.

It does not establish live API currentness, `gh` transport behavior, production accuracy, failure
prevalence, merge safety, developer time saved, market demand, adoption, or product-market fit.
Those claims require live transport tests, independently sampled incidents, and product evidence.
