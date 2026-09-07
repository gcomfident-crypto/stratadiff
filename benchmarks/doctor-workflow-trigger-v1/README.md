# Doctor Workflow Trigger v1

This bundle is a deterministic, network-free benchmark for one narrow question:

> When a required check is absent on an exact pull-request or merge-group target, can Doctor
> identify a workflow-trigger cause without claiming more than the collected evidence proves?

It contains 22 controlled cases. Inputs live in `cases.json`; expected diagnoses live only in
`oracle.json`. `verify.py` is an independent reference evaluator and imports no StrataDiff code.
The fixtures are constructed from documented GitHub semantics and public incident reports; they
are not copied production payloads and contain no source, patch, logs, credentials, or personal
data.

## Covered decisions

- missing `merge_group` trigger on a merge-queue target
- matching `merge_group` control
- `paths`, ordered negation, and `branches` filtering
- pull-request base-branch rather than head-branch matching
- `pull_request.types` excluding `synchronize`
- skipped job control: a job-level conditional is not a missing workflow
- merge conflict preventing `pull_request` workflows
- disabled and syntactically invalid workflows
- confirmed and merely possible fork approval gates
- stale required job/check name and duplicate job-name ambiguity
- dynamic or reusable job names that must remain unknown
- third-party merge-group incompatibility and runtime delivery gaps
- GitHub's 300-file path-filter evaluation boundary

## Bundle layout

- `cases.json`: normalized, immutable observations only; expected causes are forbidden.
- `oracle.json`: exact target-bound diagnoses, confidence, evidence codes, and repair actions.
- `manifest.json`: provenance, coverage gates, hashes, aggregate expectations, claim boundary.
- `verify.py`: closed-shape validation, independent derivation, oracle comparison, and mutations.
- `SHA256SUMS`: byte-level integrity for every asset except itself.

## Offline verification

```text
python3 -B benchmarks/doctor-workflow-trigger-v1/verify.py verify
python3 -B benchmarks/doctor-workflow-trigger-v1/verify.py self-test
(cd benchmarks/doctor-workflow-trigger-v1 && sha256sum -c SHA256SUMS)
```

To compare an implementation adapter, emit an object with the same closed shape as `oracle.json`
and run:

```text
python3 -B benchmarks/doctor-workflow-trigger-v1/verify.py score candidate.json
```

## Provenance boundary

Official GitHub documentation defines the normative trigger behavior. Public issues establish that
the failure modes occur in real projects; they do not establish prevalence. In particular:

- GitHub documents that workflow-level path, branch, or commit-message skips leave required checks
  pending, while a job skipped by a conditional reports success.
- GitHub documents that merge queues need the separate `merge_group` trigger and that conflicted
  pull requests do not trigger `pull_request` workflows.
- GitHub recommends unique job names across workflows because duplicates can make required results
  ambiguous.
- GitHub's fork approval documentation requires a maintainer action, but a fork alone does not
  prove approval is the blocker.

Public incidents include GitHub Docs' path-filter FAQ, Mantid Imaging's merge-queue/path-filter
deadlock, missing merge-group statuses from CodeQL and Read the Docs, Danger's split between a
successful Actions job and an absent required service status, and Flutter's runtime webhook or
handler loss. Every case lists the source identifiers that motivated it.

## Claim boundary

Passing this benchmark shows agreement with these controlled trigger semantics. It does not prove
live GitHub collection, YAML compatibility beyond this corpus, production accuracy, defect recall,
failure prevalence, merge safety, developer time saved, adoption, revenue, or product-market fit.
Those claims require live tests and blinded real-incident evaluation.
