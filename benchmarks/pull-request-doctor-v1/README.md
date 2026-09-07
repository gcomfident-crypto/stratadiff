# Pull Request Doctor v1

Pull Request Doctor v1 is a deterministic, network-free semantic corpus for one narrow product
question:

> For this exact pull-request head, which required check is satisfied, pending, failed, absent, or
> produced by the wrong App—and is the evidence complete enough to say so?

The bundle contains 15 constructed cases and an independently derived oracle. It is a target
contract for the P0 required-check diagnosis core, not a sample of production pull requests. The
`acme/doctor-fixture` repository and all observations are controlled fixtures; verification makes
no network calls.

## Target contract

Every materialized snapshot binds one repository, pull request, base SHA, and head SHA. Check Runs
and legacy commit statuses in that snapshot are the current normalized signal rollup for that exact
head, not historical attempts.

The reference evaluator requires these behaviors:

- A policy requirement is keyed by `(context, expected_app_id)`. Repeated keys from rulesets and
  classic branch protection become one diagnosis while retaining every distinct policy reference.
- A pinned requirement is satisfied only by a Check Run from the expected App. A same-name success
  from another known App is `source_mismatch`.
- If both the expected and an unrelated App emit the same name, evidence from the expected App has
  precedence. An unrelated failure cannot override a matching success.
- An unpinned requirement accepts either Check Runs or legacy commit statuses. A failed signal wins
  over a pending or successful signal; a pending signal wins over success. Two different known Apps
  publishing the same unpinned name are `source_unknown`, even when both report success.
- A completed Check Run with `success`, `neutral`, or `skipped` is successful. Other frozen
  completed conclusions fail. `queued` and `in_progress` are pending.
- Absence is `missing` only when the relevant Check Run and commit-status surfaces are complete.
  With a gap on either signal surface, absence is `source_unknown`.
- A partial snapshot can never be `checks_clear`. A known failed, pending, missing, or source-mismatch
  blocker remains `checks_blocked` when its relevant signal surfaces are complete; otherwise the
  partial result is `inconclusive`. A matching success plus target-identity drift is inconclusive.
- A complete snapshot is `checks_clear` only when every deduplicated requirement is satisfied;
  otherwise it is `checks_blocked`.

## Controlled cases

| Case | Controlled fact | Oracle |
|---|---|---|
| `clean-pinned` | success from the exact required App | clear / satisfied |
| `clean-unpinned` | success from any App for an unpinned context | clear / satisfied |
| `missing-complete` | complete observation with no signal | blocked / missing |
| `missing-partial-inconclusive` | incomplete signal collection with no signal | inconclusive / source unknown |
| `wrong-app` | same-name success from a different known App | blocked / source mismatch |
| `right-and-wrong-app` | right-App success plus wrong-App failure | clear / satisfied |
| `pending-check` | matching Check Run is still in progress | blocked / pending |
| `failed-check` | matching Check Run completed with failure | blocked / failed |
| `legacy-success` | unpinned context has only a successful legacy status | clear / satisfied |
| `same-name-check-status-split-brain` | Check Run succeeds while legacy status fails | blocked / failed |
| `duplicate-policy-dedupe` | two policies emit the same pinned key | one clear result with both policies |
| `head-drift-incomplete` | a positive signal exists, but target identity drifted | inconclusive / satisfied |
| `source-ambiguous-unpinned` | two known Apps publish the same unpinned success | inconclusive / source unknown |
| `pinned-check-legacy-failure` | pinned-App check succeeds but legacy status fails | blocked / failed |
| `partial-known-failure` | requirements are partial but the observed signal failed | blocked / failed |

The frozen aggregate contains five clear controls, seven blocked diagnoses, and three inconclusive
controls. Across 15 deduplicated requirements it contains six satisfied, four failed, two source
unknown, and one each of pending, missing, and source mismatch.

## Bundle layout

- `cases.json` stores immutable snapshot defaults plus the evidence unique to each controlled case.
- `oracle.json` freezes only semantic outcomes: verdict, requirement status, selected evidence,
  retained policy provenance, and collection gaps.
- `manifest.json` binds cases and oracle by SHA-256, freezes coverage gates and aggregate counts,
  and encodes the non-negotiable claim boundary.
- `verify.py` validates closed shapes and privacy constraints, materializes all snapshots, derives
  outcomes without importing product code, checks named-case invariants, and rejects tampering.
- `SHA256SUMS` covers every human-readable and executable asset except itself. It detects local
  modification; it is not a publisher signature.

JSON objects use closed field sets and duplicate keys are rejected. Fixtures intentionally contain
no source, diff, patch, PR body, commit message, Check Run output, free-form status description,
credential, or token.

## Offline verification

```text
python3 -B benchmarks/pull-request-doctor-v1/verify.py verify
python3 -B benchmarks/pull-request-doctor-v1/verify.py self-test
(cd benchmarks/pull-request-doctor-v1 && sha256sum -c SHA256SUMS)
```

For an implementation adapter or manual inspection:

```text
python3 -B benchmarks/pull-request-doctor-v1/verify.py materialize
python3 -B benchmarks/pull-request-doctor-v1/verify.py derive-oracle
python3 -B benchmarks/pull-request-doctor-v1/verify.py summary
```

`materialize` emits complete snapshot objects so a Rust conformance test can feed the same inputs
through the product analyzer. The independent verifier establishes the fixture contract; by itself
it does not establish that the live collector or CLI conforms to that contract.

The self-test exercises the real validation path and rejects 13 mutations: a forged oracle, an
omitted scenario, false completeness, removal of the deduplication control, removal of the
head-drift gap, a weakened claim boundary, a stale asset hash, forbidden Check Run output, a
duplicate JSON key, checksum file substitution, collapsed App ambiguity, removal of a pinned
legacy failure, and erasure of a known blocker under partial collection.

## Claim boundary

Passing this bundle demonstrates internal agreement among the frozen controlled fixtures, semantic
oracle, manifest, and independent reference evaluator. Once a product adapter is added, its passing
conformance test can demonstrate agreement of that analyzer with these same controlled semantics.

It does **not** measure production accuracy, defect recall, failure prevalence, merge safety,
developer time saved, willingness to pay, adoption, retention, or product-market fit. It does not
exercise GitHub authentication, pagination, permission handling, network retries, live policy
collection, or the end-to-end CLI. Those claims require separate live collector tests, blinded
real-incident labels, and product experiments; they must not be inferred from this corpus.
