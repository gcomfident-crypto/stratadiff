# Reviewer Study v1 — prospective preregistration

## Claim boundary

This kit defines a prospective study. It contains no observed reviewer-performance result. The
repository currently contains no study dataset. Any future synthetic input may exercise validation,
but its aggregate must remain labeled synthetic and is not evidence of time savings, issue recall,
false-carry safety, repeat use, or product-market fit.

## Design and sampling

The unit of analysis is one completed, matched baseline/resume pair. A participant reviews the A
and B variants of one isomorphic task family, with the same number of seeded issues: one through the
ordinary baseline flow and one through Review Resume. The four assignment cells cross presentation
order with the baseline variant:

```text
baseline A then resume B     baseline B then resume A
resume B then baseline A     resume A then baseline B
```

The assignment schedule is generated and randomized before data collection. In a locked dataset,
the largest and smallest cell counts may differ by at most one globally, within each participant,
and within each task family. A task family must cover at least four observations, so family-level
balance cannot pass vacuously, and may be reused across participants. A participant must not see the
same task family twice because one pair already exposes both variants. These constraints balance
order, variant, participant practice, and task difficulty without pretending that a transiently
incomplete open dataset is already balanced. The analysis population is frozen when collection is
marked `locked`; no post-hoc exclusions or imputation are permitted.

Aggregation is forbidden until there are at least 100 complete paired observations from at least 20
distinct participants, at least one carried file-change unit has been independently adjudicated, and
28-day repeat-use follow-up is complete for every participant. The validator accepts a structurally
valid, temporarily unbalanced open dataset. A locked dataset must satisfy the counterbalance
contract, and the aggregator must additionally reject it until every sample-size and follow-up gate
is met.

## Measurements

Each arm records only integer counts:

- `completion_seconds`: elapsed task time from the fixed start event to submission;
- `issues_found` and `seeded_issues`: seeded issues correctly identified and the fixed seeded total;
- `reopened_files` and `reopened_lines`: files and lines deliberately reopened by the reviewer.

The resume arm also has a completed false-carry adjudication. Its unit is one carried file change.
At least two independent adjudicators inspect every carried unit; all disagreements must be resolved
before locking. Only counts are retained, never code, issue text, comments, repository names, URLs,
or reviewer identities.

Repeat use is measured once per study-local participant: invited again, 28-day observation complete,
and whether Review Resume was used again within that window. Opaque participant, pair, and task IDs
are random and meaningful only within this study.

## Frozen analysis

For completion time, reopened files, and reopened lines, each pair's reduction is:

```text
(baseline − resume) × 10,000 ÷ baseline
```

The quotient is rounded to integer basis points, half away from zero. The reported paired effect is
the median of those integer reductions, with an even-sized median rounded by the same rule. Issue
recall is the micro-average `sum(issues_found) / sum(seeded_issues)` for each arm. No missing value,
default, imputation, or continuity correction is allowed.

The preregistered go/no result is true only when all of these hold:

- median paired completion-time reduction is at least 20%;
- median paired reopened-file reduction is at least 40%;
- resume seeded-issue recall is not below baseline recall;
- independently confirmed false carries equal zero;
- at least 50% of eligible participants use Review Resume again within 28 days.

Reopened-line reduction is reported as an exploratory endpoint and is not a gate. Aggregate output
contains no pair- or participant-level records and pins both the input bytes and this machine-readable
preregistration by SHA-256.

## Privacy and operational controls

The JSON schema is closed: unknown properties fail validation. It intentionally has no field for
names, emails, GitHub logins, repository identifiers, pull-request URLs, code, issue descriptions,
comments, or absolute event timestamps. The study operator remains responsible for consent, local
ethics review, secure storage of any separately held linkage key, randomization execution, and
provider authorization. This repository stores no linkage key.
