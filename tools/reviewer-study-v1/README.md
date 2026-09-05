# Reviewer Pilot Kit

The Reviewer Pilot Kit runs the prospective, counterbalanced study defined by
[`PROTOCOL.md`](../../benchmarks/reviewer-study-v1/PROTOCOL.md). Its purpose is narrow: test whether
Review Resume reduces repeated review work without reducing seeded-issue recall. It is not product
telemetry, an approval system, or evidence that StrataDiff is safe to merge.

The pilot is an explicitly operated research workflow. Ordinary `stratadiff resume`,
`gh stratadiff resume`, Inbox, Audit, and Demo runs do not invoke this tool, write pilot state, or
upload measurements. There is no hidden enrollment and no automatic collection from normal product
use.

## Before recruiting participants

The operator is responsible for consent, any required ethics or legal review, provider
authorization, task construction, the seeded-issue oracle, and a separately protected identity
linkage record. None of those responsibilities is established by a passing CLI command.

Freeze these inputs before collection begins:

- the machine-readable
  [`preregistration.json`](../../benchmarks/reviewer-study-v1/preregistration.json);
- isomorphic A/B task bundles with the same fixed seeded-issue count;
- the task and bundle digests accepted by the pilot;
- the participant count and randomized counterbalanced assignment schedule;
- the operator public key and an externally timestamped record containing the plan digest.

The CLI requires Python 3, `ssh-keygen` with SSH signature support, a dedicated study-only Ed25519
operator key, one distinct study-only Ed25519 key per adjudicator, and a pinned StrataDiff binary.
Reusing a personal SSH key makes the public operator key linkable to that identity. For the
preregistered minimum of 100 pairs from 20 participants, a 20-slot plan needs at least five task
families because every activated participant receives every family.

Keep the runtime state directory outside the repository, source checkout, cloud-synchronized
folders, and shared temporary directories. The pilot sets the state directory to mode `0700` and
writes its control JSON and receipts with mode `0600`. Preflighted bundle copies may preserve their
source file modes and therefore rely on the private ancestor directory. The directory contains a
private task specification with local absolute bundle and binary paths plus operator-authored
argument arrays. It must not contain participant names, email addresses, GitHub logins, repository
or pull-request identifiers, issue prose, comments, access tokens, or the identity-linkage key.

Use each command's `--help` before operating a real study:

```console
python3 tools/reviewer-study-v1/pilot.py --help
python3 tools/reviewer-study-v1/pilot.py self-test
```

`self-test` uses synthetic local fixtures. It does not create evidence about reviewer time, recall,
false carries, repeat use, or product-market fit.

## End-to-end workflow

The expected order is:

```text
signed plan -> enrollment -> preflight -> paired sessions -> blind adjudication
            -> 28-day follow-up -> lock -> final attestation -> independent verification
```

Collection state changes append typed events to the private study state; plans, attestations, and
one-use receipts are separate canonical files. Do not edit any of them by hand. Inspect status,
repair the underlying input, and rerun the documented command after an interruption unless the CLI
has marked the arm terminal. A hash chain and canonical artifacts make later changes detectable;
they do not make self-reported observations true.

### 1. Create and verify the plan

Create the randomized assignment plan before enrollment, sign its canonical attestation with the
study operator key, and anchor the resulting digest outside the pilot state. The plan fixes the
protocol digest, task bundles, participant slots, assignment cells, and collection limits.

```console
python3 tools/reviewer-study-v1/pilot.py plan create \
  --state-dir /secure/stratadiff-pilot/state \
  --task-spec /secure/stratadiff-pilot/task-spec.json \
  --preregistration benchmarks/reviewer-study-v1/preregistration.json \
  --participant-slots 20 \
  --adjudicator-slots 3 \
  --seed-file /secure/stratadiff-pilot/randomization-seed.hex
```

`--participant-slots` must be a multiple of four. `--seed-file`, when supplied, contains exactly
64 lowercase hexadecimal characters; retaining it makes plan generation reproducible. The tool
derives a fixed-format opaque study ID from that seed so an operator cannot place identifying text
in a public artifact. Real runs must use the repository's byte-exact Reviewer Study v1
preregistration; only `--synthetic` exercises may use a reduced custom preregistration. Synthetic
output cannot support a product claim.

The task specification is private and follows
[`pilot-task-spec.schema.json`](../../benchmarks/reviewer-study-v1/pilot-task-spec.schema.json). It
names a pinned StrataDiff binary, trusted local task bundle directories, fixed opaque
response/issue/reopen/carry IDs, and direct argument arrays for the preflight and run commands. A
run command contains one exact `{result}` argument, which the pilot replaces with its private result
path. Plan creation hashes every regular file in each bundle, rejects symlinks and unsupported
entries, and emits a sanitized task catalog without local paths or commands.

After externally recording the newly printed plan digest, attest the plan before enrollment. The
`--anchor-sha256` value is the lowercase SHA-256 of that external timestamped record, not an
operator-supplied timestamp:

```console
python3 tools/reviewer-study-v1/pilot.py plan attest \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --anchor-sha256 <64-lowercase-hex-external-anchor>
python3 tools/reviewer-study-v1/pilot.py plan verify \
  --state-dir /secure/stratadiff-pilot/state
```

A valid local signature only identifies the declared key. To make the “planned before collection”
claim auditable, publish or otherwise externally timestamp the plan digest before the first
session. A plan stored only beside its event log can be replaced together with that log by someone
who controls the machine and signing key.

### 2. Enroll without recording identity

Enrollment assigns a random study-local participant ID such as `p_...` to a frozen plan slot. Keep
the real-person-to-opaque-ID linkage, consent record, and contact information in a separate system
under the operator's retention policy. They are deliberately outside the pilot schema and event
chain.

```console
python3 tools/reviewer-study-v1/pilot.py enroll \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --receipt-out /secure/stratadiff-pilot/receipts/participant-001.invite.json
```

`enroll` activates the next frozen slot; it does not accept a person's identity. Give the resulting
invite receipt only to that participant. It contains a bearer credential and is not a public
artifact.

Record attrition explicitly rather than deleting or silently replacing an enrolled slot:

```console
python3 tools/reviewer-study-v1/pilot.py attrition replace \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json \
  --reason declined \
  --receipt-out /secure/stratadiff-pilot/receipts/participant-001-replacement.invite.json
```

Replacement is allowed only before that slot starts a session and invalidates the prior invite.
After a session starts, record withdrawal instead:

```console
python3 tools/reviewer-study-v1/pilot.py attrition withdraw \
  --state-dir /secure/stratadiff-pilot/state \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json \
  --reason participant_withdrew
```

A post-start withdrawal is irreversible and deliberately prevents a v1 collection lock; it is not
an exclusion mechanism.

The locked analysis population contains all schema-valid completed pairs frozen before analysis;
there is no post-hoc exclusion or imputation.

### 3. Preflight and run each assigned arm

Preflight checks the frozen assignment, expected bundle digest, arm order, state transition, and
required local inputs before a timer starts. It must succeed before the corresponding run; the
implementation checks the preloaded bundle digest but does not impose a freshness interval.

```console
python3 tools/reviewer-study-v1/pilot.py session status \
  --state-dir /secure/stratadiff-pilot/state \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json
python3 tools/reviewer-study-v1/pilot.py session preflight \
  --state-dir /secure/stratadiff-pilot/state \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json
python3 tools/reviewer-study-v1/pilot.py session run \
  --state-dir /secure/stratadiff-pilot/state \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json
```

For a real participant, omit `--yes`: the explicit `START` prompt defines the timer boundary. The
flag is intended for controlled automation, and `--no-open` is available for a headless run. If a
runner exits before writing a valid result, rerunning continues the original monotonic timer. A
boot change or a duration over 86,400 seconds permanently interrupts that arm rather than silently
starting a new measurement.

The pilot stores elapsed whole seconds from a monotonic clock and integer outcome counts. It does
not retain wall-clock session timestamps, source, file paths, issue descriptions, comments, or the
participant's GitHub identity. The retained arm measurements are:

- `completion_seconds`;
- `issues_found` and the frozen `seeded_issues` total;
- `reopened_files` and `reopened_lines`.

The task bundle is selected by opaque ID and verified digest. The private task specification holds
the local paths and argument arrays needed to preflight and run it; these fields never enter the
sanitized task catalog, plan, public dataset, or attestations. The runner invokes those
operator-authored arguments, so it is not a sandbox. Only use reviewed commands, a pinned
StrataDiff binary, and trusted task bundles. Do not accept a participant-provided executable, path,
argument, environment override, or repository URL.

Each pair uses both variants of one isomorphic task family. One arm uses the ordinary baseline flow
and the other uses Review Resume, in the order fixed by the plan. A participant must not receive the
same task family twice.

### 4. Adjudicate carried units with commit/reveal blinding

Register adjudicators by opaque key ID, then assign each carried file-change unit to at least two
adjudicators. The adjudication record contains opaque task, carry, and adjudicator IDs—not a path,
code fragment, repository, or prose description.

```console
python3 tools/reviewer-study-v1/pilot.py adjudicator register \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --public-key /secure/stratadiff-pilot/keys/adjudicator-a.pub \
  --receipt-out /secure/stratadiff-pilot/receipts/adjudicator-a.json
python3 tools/reviewer-study-v1/pilot.py adjudication assign \
  --state-dir /secure/stratadiff-pilot/state \
  --adjudicator-key /secure/stratadiff-pilot/keys/adjudicator-a \
  --receipt-out /secure/stratadiff-pilot/receipts/adjudication-a.json
python3 tools/reviewer-study-v1/pilot.py adjudication commit \
  --state-dir /secure/stratadiff-pilot/state \
  --assignment /secure/stratadiff-pilot/receipts/adjudication-a.json \
  --adjudicator-key /secure/stratadiff-pilot/keys/adjudicator-a \
  --decision valid_carry \
  --reveal-out /secure/stratadiff-pilot/receipts/reveal-a.json
```

Each adjudicator first commits to a decision with a fresh 256-bit nonce:

```text
SHA-256(canonical JSON({context_sha256, decision, nonce}))
```

The decision and nonce stay in that adjudicator's private reveal receipt until commitments from at
least two distinct adjudicator keys exist; they do not enter the shared event log until reveal.
However, `--decision` is a command-line argument and may be visible in shell history or a local
process listing, so adjudicators must use mutually isolated accounts or hosts when that exposure
would break blinding. Reveals are checked against their commitments. Two matching independent
reveals resolve a unit. A disagreement requires a third distinct adjudicator to commit and reveal;
the operator must not edit the tally into agreement. Distinct keys do not prove distinct or
independent people, so the final artifact treats independence as an operator attestation, not a
cryptographic fact.

Every planned adjudication unit, including an explicit no-carry confirmation when a Resume arm has
no carried unit, must be adjudicated before lock. The preregistered release gate is zero confirmed
false carries.

Repeat `assign` and `commit` with the second assigned key. Only after both initial commitments
exist, submit their private reveal receipts:

```console
python3 tools/reviewer-study-v1/pilot.py adjudication reveal \
  --state-dir /secure/stratadiff-pilot/state \
  --reveal /secure/stratadiff-pilot/receipts/reveal-a.json
python3 tools/reviewer-study-v1/pilot.py adjudication status \
  --state-dir /secure/stratadiff-pilot/state
```

If the initial decisions disagree, the frozen resolver key can then use the same
`assign -> commit -> reveal` sequence. Keep every assignment and reveal receipt private.

### 5. Complete the 28-day follow-up

The operator records the invitation separately from the later outcome. The private state may retain
the follow-up deadline needed to enforce the frozen 28-day window, but the public study dataset
contains only three booleans: `invited_again`, `follow_up_complete`, and
`used_within_28_days`.

```console
python3 tools/reviewer-study-v1/pilot.py follow-up invite \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --invite /secure/stratadiff-pilot/receipts/participant-001.invite.json \
  --receipt-out /secure/stratadiff-pilot/receipts/participant-001.follow-up.json
python3 tools/reviewer-study-v1/pilot.py follow-up run \
  --state-dir /secure/stratadiff-pilot/state \
  --follow-up /secure/stratadiff-pilot/receipts/participant-001.follow-up.json \
  123 -R OWNER/REPOSITORY
```

Running Review Resume during an explicitly consented follow-up may use its normal GitHub path, and
an operator-authored session command may make its own network requests. The pilot does not upload
its state or copy command arguments, repository identifiers, URLs, logins, source, diffs, review
text, or GitHub responses into public artifacts or events. A normal Resume run outside this study
remains zero-telemetry.

`follow-up run` counts use only after the pinned native Resume process reports Workbench readiness;
it does not prove that the reviewer finished or submitted a GitHub review. After the complete
28-day window has elapsed, close it explicitly. Closing early fails:

```console
python3 tools/reviewer-study-v1/pilot.py follow-up close \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --follow-up /secure/stratadiff-pilot/receipts/participant-001.follow-up.json
```

### 6. Lock the population

Lock only after every assignment for every activated participant is complete, all corresponding
carried units have resolved adjudication, and every activated participant's follow-up window has
closed. Activated slots must form complete cohorts of four. Lock freezes the analysis population
and prevents additional collection or post-hoc exclusions.

```console
python3 tools/reviewer-study-v1/pilot.py lock \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --output /secure/stratadiff-pilot/public/study-data.json \
  --aggregate-output /secure/stratadiff-pilot/public/aggregate.json
```

The locked export must conform to
[`study-data.schema.json`](../../benchmarks/reviewer-study-v1/study-data.schema.json). Aggregation is
forbidden for a real evidence claim until the preregistered minimums are met: at least 100 completed
pairs, at least 20 participants, at least one independently adjudicated carried unit, and complete
28-day follow-up. `--synthetic` permits a smaller, visibly labeled lifecycle self-test; its output is
not study evidence.

### 7. Attest and verify

Create a canonical final attestation only after lock. The JSON attestation does not contain its own
signature; sign the exact bytes as a detached `.sig` file. Keep the corresponding public key and
opaque key fingerprint available to verifiers.

```console
python3 tools/reviewer-study-v1/pilot.py attest-final \
  --state-dir /secure/stratadiff-pilot/state \
  --operator-key /secure/stratadiff-pilot/keys/operator_ed25519 \
  --dataset /secure/stratadiff-pilot/public/study-data.json \
  --aggregate /secure/stratadiff-pilot/public/aggregate.json \
  --output /secure/stratadiff-pilot/public/final-attestation.json \
  --consent-obtained \
  --provider-authorized \
  --linkage-key-not-exported
python3 tools/reviewer-study-v1/pilot.py verify \
  --plan /secure/stratadiff-pilot/state/plan.json \
  --preregistration /secure/stratadiff-pilot/state/preregistration.json \
  --task-catalog /secure/stratadiff-pilot/state/task-catalog.json \
  --plan-attestation /secure/stratadiff-pilot/state/plan-attestation.json \
  --plan-signature /secure/stratadiff-pilot/state/plan-attestation.json.sig \
  --operator-public-key /secure/stratadiff-pilot/state/operator.pub \
  --dataset /secure/stratadiff-pilot/public/study-data.json \
  --aggregate /secure/stratadiff-pilot/public/aggregate.json \
  --final-attestation /secure/stratadiff-pilot/public/final-attestation.json \
  --final-signature /secure/stratadiff-pilot/public/final-attestation.json.sig
```

`plan verify` checks the private event chain while collection is in progress. Public `verify`
checks the frozen plan and catalog bindings, canonical public-key and detached-signature files,
artifact digests, every plan-to-dataset assignment and task-count binding, the supported public
privacy checks, and the independently recomputed aggregate. The public bundle
includes only the signed event-chain tip and operator flow-count claims, not the private events, so
an external verifier cannot recompute those two fields. Neither command can verify participant
honesty or infer facts that were never collected.

The existing analysis tool independently validates, aggregates, and verifies the privacy-minimized
study dataset:

```console
python3 tools/reviewer-study-v1/reviewer_study_v1.py \
  --preregistration /secure/stratadiff-pilot/state/preregistration.json \
  validate --input study-data.json
python3 tools/reviewer-study-v1/reviewer_study_v1.py \
  --preregistration /secure/stratadiff-pilot/state/preregistration.json \
  aggregate \
  --input study-data.json \
  --output aggregate.json
python3 tools/reviewer-study-v1/reviewer_study_v1.py \
  --preregistration /secure/stratadiff-pilot/state/preregistration.json \
  verify \
  --input study-data.json \
  --aggregate aggregate.json
```

## Privacy boundary

Treat the generated files in two groups:

| Artifact | Handling |
|---|---|
| `private-task-spec.json`, `events/`, `preloaded/`, `results/`, invite receipts, assignment receipts, reveal receipts, follow-up receipts | Private operational state. It may expose task source, local paths, bearer credentials, or blinded decisions. Never publish it. |
| `plan.json`, `preregistration.json`, `task-catalog.json`, `plan-attestation.json`, `plan-attestation.json.sig`, `operator.pub`, `study-data.json`, `aggregate.json`, `final-attestation.json`, `final-attestation.json.sig` | Candidate verification bundle. Publish only after `pilot.py verify` accepts the exact files and the operator has checked the disclosure policy. |

The task catalog is the sanitized, content-free counterpart of the private task specification.
Participant, adjudicator, assignment, reveal, and follow-up receipts remain private even after the
aggregate is published. Opaque study IDs are pseudonyms, not a proof of anonymity; anyone holding
the separate linkage record can still reconnect them to participants.

Private pilot state may contain only the minimum study-local operational data required to enforce
the protocol:

- the non-public task specification, including trusted local bundle and binary paths and argument
  arrays;
- preflighted task-bundle copies, which may contain source and other task material;
- private session-result, invitation, assignment, reveal, and follow-up receipts;
- opaque participant, pair, task-family, issue, carry-unit, and adjudicator IDs;
- the frozen plan and bundle digests;
- a monotonic session start marker and integer measurements;
- an opaque reopened-file or reopened-line accounting set where required to derive counts;
- private follow-up deadlines;
- adjudication commitments and validated reveals;
- canonical events, receipts, hashes, and signature metadata.

The separately held identity linkage is not pilot input. Source is confined to trusted private task
bundles and their preflighted copies. Except for the private task specification's operator-authored
local paths and argument arrays, GitHub logins, repository or pull-request identifiers, URLs,
review or issue prose, comments, captured command lines, environment snapshots, credentials, and
absolute session timestamps are prohibited. The declared task-specification, task-catalog, plan,
event-payload, attestation, and study-dataset structures reject unknown fields. The sanitized task
catalog exposes only opaque IDs, counts, and content digests.

The public export is smaller still: study-local opaque IDs, assignment cells, integer arm counts,
false-carry totals, and participant-level repeat-use booleans. Aggregate output contains no pair- or
participant-level rows and pins both the source dataset and preregistration by SHA-256. Published
task and bundle digests may reveal equality with the same material published elsewhere. Delete the
private runtime state and the separately held linkage according to the consented retention policy;
publishing a final aggregate does not authorize indefinite retention.

## Threat boundary

The pilot is designed to make these failures visible:

- collecting before a frozen, signed plan exists;
- changing assignments, task digests, observations, or exclusions after the fact;
- skipping required order, preflight, adjudication, follow-up, or lock transitions;
- using wall-clock adjustments to rewrite a timed session duration;
- revealing one adjudicator's decision before independent commitments exist;
- exporting unknown or directly identifying fields;
- presenting an incomplete, synthetic, unsigned, or tampered artifact as a verified study result.

It does not protect against:

- a compromised host, Python runtime, operating system, operator key, or external task harness;
- a dishonest or colluding operator, participant, or adjudicator;
- fabricated consent, identity, task difficulty, seeded-issue truth, reopened-work counts, or repeat
  use;
- two keys controlled by the same adjudicator;
- side-channel disclosure through screen recording, shell history, process inspection, backups, or
  an operator's separate linkage system;
- a malicious task bundle or operator-authored task command executed by the pilot without a
  sandbox;
- semantic errors, missed defects, safe-to-merge judgments, or GitHub approval validity.

A valid event chain proves internal consistency and binding to the frozen plan. The final
attestation signs its chain tip, making later event rewrites detectable relative to that signature.
A valid signature proves only that a key signed exact bytes. Neither mechanism proves that
collection happened at the claimed time, that different humans acted independently, or that the
recorded observations are truthful. Report those limits with every study result.

Preflight and run commands inherit the operator process environment and may access the network or
local machine according to their own implementation. Follow-up explicitly launches native Resume,
which normally contacts the selected GitHub host. Run the pilot under a dedicated account or
controlled environment, provide only the credentials the reviewed task requires, and do not treat
`STRATADIFF_PILOT_OFFLINE=1` as an operating-system sandbox or network control.

Timed sessions use the monotonic clock and bind a run to one Linux boot identity. The 28-day
follow-up also stores wall-clock bounds so it can survive a reboot; after the boot identity changes,
its elapsed-window enforcement necessarily trusts the host wall clock and operator environment.

## Frozen decision rule

The pilot is a go only if all preregistered gates pass:

- median paired completion-time reduction is at least 20%;
- median paired reopened-file reduction is at least 40%;
- Resume seeded-issue recall is not below baseline recall;
- independently confirmed false carries equal zero;
- at least 50% of eligible participants use Review Resume again within 28 days.

Reopened-line reduction is exploratory. Stars, installs, lines hidden, generated comments, and an
unpaired time comparison are not substitutes for these outcomes.
