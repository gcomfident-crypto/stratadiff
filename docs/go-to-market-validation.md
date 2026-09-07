# Go-to-market validation: earn enforcement with a verifiable merge proof

Evidence captured: **2026-09-06**, with a competitor and platform refresh on **2026-09-08**.
Prices, installation counters, repository counters, search results, and product behavior are
point-in-time observations. This document is a distribution and pilot plan, not evidence of
product-market fit. The technical thesis and prior-art boundary remain in
[`product-strategy.md`](product-strategy.md); they are not repeated here.

## Decision

StrataDiff should first be tested as **`gh stratadiff doctor` for stuck pull requests and merge queues**,
then as a verifiable merge-proof control plane across the AI, policy, CI, and human reviewers a
team already uses—not as another diff viewer, reviewer model, merge queue, generic inbox, or
approval bot:

> One command. One observed candidate. A proved blocker or an explicit unknown.

The immediate outcome is a local evidence bundle for one still-observable candidate: which
effective rule expects which context and App, what appeared on the exact SHA, which producer and
workflow can be proved, and where the chain becomes unknown. The later buyer outcome is an
App-bound state backed by prospectively recorded candidate evidence. The economic outcome to test
is lower duplicate review/CI spend and merge delay without worse final-candidate coverage or finding
recall. Dispatch proxies, files hidden, generated summaries, and installation counts are not value
metrics.

The most important go-to-market finding is an **event and trust problem**:

- Existing reviewers already receive PR events, but their comments, review states, Check Runs, and
  UI recommendations do not form one cross-provider proof automatically.
- Teams face a bad choice between paying on every push and disabling follow-up review; a deferred
  review must never masquerade as a clean result.
- GitHub's native queue creates a different candidate SHA, while required check names, workflow
  triggers, and source App identity are easy to configure inconsistently.
- The merge gate is trusted only if evidence binds the current head and final candidate, survives
  races and recovery, and comes from a dedicated expected App identity. Expected-App binding proves
  the Check source, not the meaning or quality of an upstream review.

Therefore acquisition starts with a queue-neutral, read-only `gh stratadiff doctor <PR>` at the
moment a merge is blocked, before an App install or queue migration. A platform owner who reuses and
verifies those diagnoses can run the administration-scoped but non-mutating Merge Readiness
Preflight. Only when reuse establishes a material post-hoc candidate-evidence gap should
the team install a lightweight App Recorder. Shadow evaluation is a later promotion of recorded,
replayable evidence; enforcement is later still. Review Resume remains the inspection path and the
Action the transparent self-hosted path. Routine marketing comments, a replacement review UI, or a
second reviewer model would add noise without solving the job.

The retention boundary is concrete. In [dcoapp/app #303](https://github.com/dcoapp/app/issues/303),
a required `DCO` result remained Expected for roughly an hour before queue ejection. The temporary
ref and Check Runs were gone by investigation time and the current public timeline did not expose
the exact old candidate SHA, so that event's SHA → Check chain could not be recovered from public
evidence. App-side records later exposed both missing `merge_queues: read` and a live registration
subscribed to `merge_queue_entry` instead of `merge_group`. Doctor must report only the identity and
missing signal actually observed, without guessing that internal cause. The case supports an opt-in
prospective recorder after reuse, not a mandatory App at first contact.

## Product correction: reject the single-provider stale-guard wedge

The former “CodeRabbit stale approval guard” pitch is **No-Go**. CodeRabbit now
[documents](https://docs.coderabbit.ai/pr-reviews/request-changes-workflow) that it requires review
of the latest commit, confirms the current HEAD is among reviewed commits, and checks HEAD again
immediately before approval. GitHub also requires the latest candidate SHA and can bind a required
check to an expected App source. That source constraint does not validate the review semantics
behind the Check. Copilot's 2026-09-01 public-preview approvals are dismissed after new commits.
These native capabilities can improve further and must not be presented as missing.

The viable hypothesis is composition: a stable Check that normalizes actual evidence from multiple
providers, records overrides, applies one versioned policy, and proves the PR-head-to-merge-group
transition. Review Cache may save spend and Resume may earn trust, but neither is the category.

## Supporting opportunity: Cache for savings, Resume for explanation

Public evidence captured on 2026-09-06 strengthens a second use of the same verified transition core.
The message should become **“do not re-review unchanged code”**; the product must then prove whether
the next review should be skipped, narrowed to a residue, or run in full.

- **Observed:** a reproducible [`acoliver/vibetools` throttling study at
  `d57789d`](https://github.com/acoliver/vibetools/tree/d57789dc2c17f2be39efe2c437f25ea98457fae8/research/ai-code-review-study/coderabbit/throttling)
  covers 1,024 CodeRabbit-touched PRs across three related repositories, 7,600 commits, an estimated
  6,576 follow-up commit updates, and 256 distinct PRs retaining an exact `Review limit reached`
  comment. One repository disabled incremental review and restored it two days later because
  follow-up commits were going unreviewed. The sample is purposive and the retained bot comments are
  mutable lower bounds, so this is workflow evidence rather than a population estimate.
- **Observed:** [`fullsend-ai/fullsend#6991`](https://github.com/fullsend-ai/fullsend/issues/6991)
  records three automated reviews of one nine-line documentation change after two manual rebases.
  The issue reports about `$7` total review cost and estimates that exact diff reuse would have saved
  `$4.70`. This is one public incident with self-reported cost, not a population estimate.
- **Observed:** [`nsheaps/agents#335`](https://github.com/nsheaps/agents/pull/335) implements a
  `skip / brief-refresh / full-review` dispatch decision using a cached whole-PR diff fingerprint
  after Renovate-style force-pushes. Its live verification remained unchecked at capture.
- **Observed:** [`bizimind/loxel#172`](https://github.com/bizimind/loxel/pull/172) passes prior
  review comments and an interdiff into later review agents so they do not repeat unchanged
  findings. Its stated test plan was also incomplete at capture.
- **Observed:** [`fullsend-ai/agents#1091`](https://github.com/fullsend-ai/agents/issues/1091)
  reports that a raw prior-SHA-to-current-SHA comparison pulled 14 unrelated base-branch commits
  into a three-line dependency update and contributed to a `$1.01` re-review cost increase. This is
  a self-reported single case, but it directly exercises StrataDiff's separate base-drift scope.
- **Observed:** [`QwenLM/qwen-code#9661`](https://github.com/QwenLM/qwen-code/pull/9661) records
  model review verdicts against per-file `(base, head)` blob pairs so byte-identical files can
  survive a rebase. The open PR demonstrates convergent implementation work, not validated user
  value or proof that StrataDiff's policy is superior.
- **Observed:** [`mergewatch/mergewatch.ai#519`](https://github.com/mergewatch/mergewatch.ai/issues/519)
  reports four review runs in roughly one hour on one PR, about `$1–2` of model spend, and a more
  damaging approved/dismissed/commented state churn. It separately identifies superseding stale
  in-flight runs and unchanged-diff skipping as correctness and cost controls.
- **Observed:** [`fullsend-ai/fullsend#6968`](https://github.com/fullsend-ai/fullsend/issues/6968)
  records two completed reviews around a force-push whose meaningful code diff was reported as
  unchanged. The runs cost `$1.13` and `$1.05` and produced the same verdict and finding. Its author
  explicitly requires comparison against the actual base-relative diff and zero false skips; the
  proposed `15–30%` saving is a forecast for a future 20-PR validation, not a measured result.
- **Observed:** [`AIClarityAU/minspec#1688`](https://github.com/AIClarityAU/minspec/issues/1688)
  records two PRs where updating a branch to satisfy a strict freshness gate caused an extra
  four-model review panel over unchanged reviewable content. The issue also states the central
  security boundary: event type, actor, and commit message are not sufficient evidence because a
  false cache hit could hide attacker-controlled content.
- **Observed:** [`alibaba/open-code-review#854`](https://github.com/alibaba/open-code-review/issues/854)
  reports one iterative PR with 19 review-gate rounds at roughly two million tokens per round and
  proposes cross-push reuse of per-file diff fingerprints. The discussion identifies an important
  limitation: unchanged file-local diff bytes do not prove that a prior model verdict remains valid
  when dynamically loaded repository context changes.
- **Observed:** [`fullsend-ai/fullsend#6911`](https://github.com/fullsend-ai/fullsend/issues/6911)
  reports a provenance lookup silently failing because an App client ID was absent. A `$0.62`
  initial review was followed by a `$2.15` full review for a one-file formatting fix, and the edited
  comment destroyed the earlier provenance trail. The amounts and diagnosis are self-reported in
  one issue, but the failure mode is directly relevant: a missing receipt binding must be visible
  and must never masquerade as either a cache hit or a clean first run.
- **Observed:** [`fullsend-ai/agents#1092`](https://github.com/fullsend-ai/agents/issues/1092)
  reports two completed reviews of one PR costing `$7.38` and `$8.15`, plus two cancelled
  intermediate runs. The second full review surfaced one new low-severity finding and explicitly
  asks for an incremental input path. This is one incident, not an expected savings estimate.
- **Observed:** [`nexpeakcore/deepseek-harness-pr-review#26`](https://github.com/nexpeakcore/deepseek-harness-pr-review/pull/26)
  describes a PR that accumulated 57 review rounds and 58 bot comments because every head SHA
  triggered a new review. Its proposed diff fingerprint also had to include blob IDs because GitHub
  can omit patches for binary and oversized files, and it refuses to skip an incomplete prior run.
  This converges on StrataDiff's fail-closed input identity, while still relying on provider file
  summaries rather than an offline object closure.
- **Observed:** [`Expensify/App#100173`](https://github.com/Expensify/App/issues/100173) documents
  the opposite failure: an AI standards review runs only at open/ready time, so violations added by
  follow-up commits can reach approval unless a human remembers to request another review. Its own
  options expose the product tension between paying for every push and missing the one push that
  matters.
- **Observed:** [`dotCMS/core#36962`](https://github.com/dotCMS/core/issues/36962) reports four AI
  review workflows launched on every PR push with no concurrency group. Superseded runs continued
  against commits that could no longer merge; the issue reports 271 aggregate queued job-minutes
  on one run. Native workflow cancellation addresses this specific waste and should be baseline
  gateway hygiene, not claimed as StrataDiff's differentiating proof.

These projects validate the job but also expose the boundary. A whole-diff hash can skip a byte-
identical PR, but cannot safely narrow a review through dropped work, changed merge bases, stacked
parent influx, or partially unchanged files. A raw checkpoint-to-head interdiff can include
upstream noise or omit retired reviewed work. StrataDiff should not compete on another reviewer
model; it should be the deterministic cache key and residue compiler in front of human or automated
reviewers.

That boundary changes the automation contract. StrataDiff may cache and narrow a declared review
**input**, but it must not silently replay an approval or model verdict. A `skip` decision means the
complete input scope named by the integration is unchanged under the declared policy. If the
downstream reviewer can load repository context beyond that scope, its cache key must also bind the
relevant base tree, prompt, rules, model/runtime, and prior finding dispositions, or choose
`residue`, `full`, or `blocked`. The first integration should therefore expose the decision and its
exact evidence while leaving execution policy with the caller.

These surfaces are one staged product, not separate roadmaps:

| Surface | Trigger | Immediate outcome | Distribution path |
|---|---|---|---|
| Pull Request Doctor | A developer encounters one blocked PR or queue candidate | Join effective rule, required context/App, exact observed SHA, Check/suite/run/job, and attributable workflow trigger; emit a proved blocker or explicit unknown | `gh` extension; no App install or queue migration |
| Merge Readiness Preflight | The same owner verifies and reuses Doctor | Explain repository-wide ruleset, Check-source, workflow-trigger, review-evidence, override, and duplicate-work risks from still-observable history | Non-mutating CLI using the owner's authorized read access |
| App Recorder | Reuse proves ephemeral candidate loss prevents diagnosis | Prospectively retain bounded candidate, delivery, policy, Check, suite, run, and job observations without gating merges | Narrowly permissioned GitHub App; no comments or required Check |
| Merge Proof | Recorder evidence replays correctly and the owner requests shadow evaluation | Evaluate configured obligations and publish one actual-API, App-bound shadow or required result | Recorder App promoted to shadow, then separately to enforcement; Action as self-hosted escape hatch |
| Review Resume | A person returns after a later PR update | Open only the evidence that still needs human attention | `gh stratadiff inbox --workbench`, then a narrowly triggered App action |
| Review Cache | A workflow would rerun an AI or policy review | Emit `skip`, `residue`, `full`, or `blocked` with exact reasons and bounded source inputs | Stable JSON/Action preflight before the existing reviewer job |

The control plane should be packaged as **bring-your-own-reviewer merge proof**, not as a model
vendor. Each adapter canonicalizes observable API facts into a versioned, content-addressed evidence
snapshot; this preserves what was observed without claiming the provider's judgment was correct. A
policy selects required providers, allowed states and overrides, scope, maximum age, and carry rules.
The canonical Check reports the candidate, evidence IDs and sources, policy generation, missing
obligations, and repair state. Cache routing remains an optional optimization before an expensive
reviewer and never manufactures proof.

The public evidence also rules out “run immediately on every `synchronize` event” as the default
product loop. One team reached a `$500/month` add-on cap sixteen days early after 672 review events;
another removed its per-push trigger after a five-push branch accumulated five similar summaries;
a third counted stale bot state blocking ten ready PRs. Their fallback was to turn incremental
review off, which can leave later fixes unchecked. The gateway should instead use two distinct
states:

1. **Dispatch state:** wait while the head is moving or required CI is pending, cancel work bound to
   an obsolete head, and schedule exactly one run for the latest eligible head. Waiting is never a
   clean result.
2. **Evidence route:** once scheduled, emit only `skip`, `residue`, `full`, or `blocked` for that
   immutable head. The final merge signal is valid only when its receipt is bound to the still-live
   head.

This yields a concrete acquisition promise: **see whether the code GitHub is about to merge has the
review evidence your policy actually requires, across all of your existing reviewers.** Measured
reductions in redundant runs are a supporting ROI claim, not the initial trust claim.

Doctor is the low-risk acquisition surface because it can explain one current incident without an
App or queue migration. Preflight expands only after the owner verifies that result. Neither can
reconstruct a merge-group history that GitHub no longer exposes. The recorder is the prospective
frequency surface only after repeat use validates the retention need; the shadow Check is a later
decision surface. Review Resume remains the human inspection path. Source identity, supersession,
receipt provenance, deterministic routing, bounded payloads, and telemetry form one loop; a
standalone diff viewer or cache key does not.

The automation surface must inherit the human product's fail-closed rule. `skip` is allowed only
when the current review residue is empty and base context is accounted for. Unsupported objects,
missing checkpoints, ambiguous replay, or unmaterialized base drift must choose `full` or `blocked`,
never an optimistic cache hit. A cached model verdict is not a GitHub approval.

Before promoting Review Cache as a product claim, evaluate it on the frozen ReviewTransition set:

| Gate | First slice | Launch claim requires |
|---|---:|---:|
| False `skip` / false carry | `0/30` | `0/300` on the preregistered core and challenge split |
| Reproducibility | identical selection and result digests on two clean runs | independently replayable artifacts |
| Attention reduction | report bytes, files, and changed lines; no minimum claimed yet | precommitted threshold before seeing RT-300 outcomes |
| Economic value | reproduce the decision on public histories | at least five live reviewer-workflow sessions with measured tokens, cost, or reviewer minutes |

This correction improves discovery: automated review already runs on every relevant event, so the
preflight does not depend on a person remembering a rarely used command. It also creates a visible
per-run value metric. It does **not** prove that the advanced residue policy beats a simple diff
hash often enough to justify adoption; ReviewTransition and live pilots must answer that question.

## Evidence notation and claim boundary

This document uses four labels:

- **Observed:** directly reproduced from the checked-in artifacts, current repository, provider
  response, or official page on the capture date.
- **Vendor fact:** a product's own documentation or listing; reliable for advertised behavior and
  list price, not for independent quality, retention, or revenue.
- **Inference:** a falsifiable product interpretation of the observations.
- **Gate:** a threshold chosen before the pilot. It is a business decision, not a population fact.

No observation below establishes GitHub-wide prevalence, willingness to pay, time saved, issue
recall, or active customer count.

## Who installs first

The first installer is defined by a current control-plane problem, not a broad persona:

1. They own GitHub rulesets or developer infrastructure for a repository with at least 100 PRs per
   month and two or more required review/check sources.
2. The repository uses strict up-to-date checks or native merge queue, or has measurable duplicate
   AI-review/CI work after pushes and base updates.
3. They will verify one Doctor result, reuse Doctor or Preflight, and compare every result with the
   underlying GitHub objects.
4. If ephemeral evidence proves material, they can authorize recorder-only App access and will not
   enable a shadow or required Check before retained-event replay passes.

The highest-probability initial teams have roughly 20–300 engineers, agent-heavy or stacked PRs,
scarce CODEOWNERS, and a DevEx/platform/security owner. Staff engineers, code owners, and open-source
maintainers remain important daily users of evidence details and Resume. This separates three roles:

| Role | Immediate job | Required proof before the next ask |
|---|---|---|
| DevEx/platform/security owner | Know whether current rules and reviewer evidence protect the actual merge candidate | Doctor proves one blocker or unknown; Preflight findings are verified; recorder replay precedes shadow evaluation |
| Affected reviewer | Understand why a provider obligation is missing or resume without reconstructing a range | Evidence IDs, source, candidate, policy, and every unsupported carry are inspectable |
| PR author or maintainer | Get a final candidate unblocked without blind re-review or hidden override | The Check gives one actionable reason and preserves override provenance |

**Inference:** acquisition should target an owner with a measurable repository configuration or
evidence problem now. Targeting “all developers who review code” wastes attention because many
repositories need neither a composite policy nor a merge queue.

### Anti-ICP for the first four weeks

Do not recruit solo developers, teams dominated by tiny append-only PRs, single-reviewer
repositories with correct native protection, people primarily seeking bug findings, or teams that
cannot inspect GitHub rule and evidence objects. Do not lead with semantic equivalence, autonomous
approval, or an allegation that CodeRabbit currently accepts stale heads.

## What is known about trigger frequency

### Durable repository evidence

**Observed:** the checked-in
[`review-churn-census-v1`](../benchmarks/review-churn-census-v1/README.md) is a prospective
hash-ranked panel of 500 merged PRs from a purposefully selected set of ten review-heavy public
repositories over a fixed 90-day window. It found:

| Observation | Result | What it can support |
|---|---:|---|
| Comparable completed reviewer checkpoints that differed from final head | 88/488, 18.03% (Wilson 95% 14.87–21.69%) | Changed-after-review events exist in the selected panel |
| Fully comparable reviewed PRs that stranded at least one reviewer | 74/401, 18.45% (14.96–22.55%) | Repository qualification can find concrete affected PRs |
| Completed pairs with an observed later force-push | 43/490, 8.78% (6.58–11.62%) | Force-push is only one acquisition phrase, not the entire job |
| Same-reviewer completed re-review after an observed force-push | 8/43, 18.60% (9.74–32.62%) | Repeated human work is observable, but this denominator is small |

The repositories were not sampled from GitHub, the panel excludes open and abandoned PRs, and the
actor identifiers are intentionally PR-local. It cannot estimate events per reviewer-week or rank
individual design partners. Four repositories had a descriptive checkpoint-drift point estimate
above 20%, but the strata were purposefully chosen and are only a recruitment hypothesis.

### Current open-queue probe

**Observed:** on 2026-09-06, the current `v0.4.0` development binary was rerun against six
convenience-selected public reviewers with `--limit 20`. The same command and classification rules
were used for every reviewer:

```console
target/debug/stratadiff inbox --reviewer LOGIN --limit 20 --format json
```

| Reviewer | Search candidates | Inspected | Resume available | Up to date | No completed checkpoint | Status |
|---|---:|---:|---:|---:|---:|---|
| `erwindouna` | 69 | 20 | 10 | 6 | 4 | partial |
| `eps1lon` | 168 | 20 | 10 | 4 | 6 | partial |
| `roblourens` | 53 | 20 | 1 | 10 | 9 | partial |
| `Shadowghost` | 74 | 20 | 7 | 7 | 6 | partial |
| `oddstr13` | 18 | 18 | 0 | 10 | 8 | complete |
| `andrewm4894` | 26 | 20 | 4 | 6 | 10 | partial |
| **Total** | **408** | **118** | **32** | **43** | **43** | **five partial** |

The six scans made 93 API calls and produced zero unobservable review PRs. The raw responses were
not frozen, and their byte count can drift with provider metadata even when every classification is
unchanged, so this table is a transient probe rather than an offline-reproducible benchmark. It is a
convenience sample selected during product development, five queues were truncated, the same
underlying platform supplies all metadata, and the snapshot has no time dimension. **It must not be
reported as “27% of GitHub reviews need StrataDiff,” a weekly event rate, or a market-size
estimate.** It proves only that the current command can find multiple live events in deliberately
chosen active-reviewer queues.

### The missing measurement

The question that determines whether a personal Inbox can become a habit is still unanswered:

```text
eligible event rate =
  distinct revalidated checkpoint-to-new-head events
  / active reviewer-weeks
```

An active reviewer-week requires at least one submitted formal review during that week. Event IDs
must deduplicate repeated scans of the same provider, repository, PR, reviewer, review node,
checkpoint, and head. A head change creates a new event only after provider revalidation.

**Gate:** in the target segment, require at least `0.25` actionable events per active
reviewer-week—the equivalent of one event per active reviewer-month—before treating Inbox as a
repeatable personal workflow. Below that level, keep Inbox as a diagnostic and make the App or an
author-supplied Resume link the primary trigger. This threshold is a precommitted business
criterion, not a rate inferred from the two datasets above.

## Competitive onboarding, pricing, and distribution

The relevant competitors do not merely have features; they remove discovery and onboarding
friction in different ways. List prices and installation counters below were captured on
2026-09-06 and can change.

| Product | First-value path and permission cost | Current packaging signal | Distribution lesson for StrataDiff |
|---|---|---|---|
| GitHub native | Already enforces latest candidate SHA, can pin an expected App source, can dismiss stale reviews when configured, and exposes a distinct `merge_group` event where merge queues are available. `gh pr checks --required` lists required checks; Rules Insights exposes rule pass/fail/bypass and Evaluate results. | Rules and checks are bundled with eligible plans; merge queues are limited to organization-owned public repositories and Enterprise Cloud organization-owned private repositories | Never claim GitHub has no blocker view. The current CLI still does not join expected/observed App identity or reconstruct a native queue candidate; the narrow hypothesis is the cross-layer, queue-neutral evidence chain and explicit abstention |
| Graphite | New signups install or request its GitHub App. Its own queue is incompatible with GitHub native queue and can require bypass permissions for optimizations; its external integration hands work to another queue. | Hobby free; Starter `$20/seat/month` and Team `$40/seat/month`, billed annually; 30-day Team trial. The Marketplace listing embedded `61,296` installations at capture. | Do not replace the queue. A neutral proof layer must integrate with the chosen merge path and earn admin trust in shadow mode |
| Mergify / Aviator / Trunk queues | Mergify `queue show` exposes blocking conditions. Aviator `av pr status` exposes the associated PR status and required status checks; its pending-workflow diagnosis is limited to parallel mode with GitHub Actions and does not distinguish required from non-required workflows. Mergify supports App-qualified checks, while Trunk's testing-details API exposes required-status sources and exact `testBranchSha`. Each path is tied to that vendor's installed queue or testing run. | Existing installation and queue adoption are prerequisites; packaging differs by vendor and was not independently audited here | Do not claim blocker explanation, App qualification, or exact candidate/check chains are unique. Win on pre-install, queue-neutral diagnosis and never infer vendor-internal causes |
| Reviewable | GitHub OAuth sign-in requests broad scopes; a repo admin connects a repository. A connected repo automatically creates reviews and inserts a Reviewable link into PR descriptions. | Public/personal free; Team `$8` and Business `$16` per contributor/month billed annually; private repo connection may start a 30-day trial | Persistent review state is valuable, but workflow migration, write access, and automatic PR links are material adoption costs. The local pilot should remain no-admin and non-invasive |
| Aviator FlexReview / Review | Connect the Aviator GitHub App, activate a repo in read-only mode, wait for history indexing, test through slash commands/dashboard, and then activate selected teams; validation can later become a required check | Current pricing page advertises Free, Team `$20/dev/month` with a 14-day trial, Scale `$50/dev/month`, and custom Enterprise | Read-only shadow mode is an effective enterprise bridge. StrataDiff should copy the staged-risk pattern, not claim selective validation or owner routing as novel |
| CodeRabbit | GitHub login, App installation, then immediate review. Its Request Changes Workflow already guards exact HEAD; its automatic-review controls expose finite allowance and pause tradeoffs. | Public/OSS free; Essentials `$24`, Team `$48` per developer/month billed annually, Advanced `$90` monthly. Its Marketplace listing embedded `318,416` installations at capture | Treat it as an evidence producer, not a broken stale guard. Reused Doctor/Preflight value and a confirmed retention gap must justify a second App installation |
| Copilot code review | Native rulesets can request review on each push; public-preview approvals are dismissed on new commits. Re-reviews may repeat resolved findings, and one public MLflow report observed zero actual approvals in 78 reviews despite enabled settings. | Bundled by Copilot plan; approval behavior is a fast-moving public preview as of 2026-09-08 | Read actual review objects and remain robust to product changes. A preview rollout defect is a test case, not a durable wedge |

Graphite's own [proof-of-concept guidance](https://graphite.com/docs/onboarding-your-team) recommends
starting with 5–10 engineers on one team, running for at least four weeks, and defining success
metrics. That is vendor advice, not independent evidence, but it is a useful benchmark: even a much
broader incumbent expects coordinated workflow adoption to take a month.

**Inference:** StrataDiff has no pricing evidence yet. Competitor prices show that review workflow
has budget, not that a narrow, event-driven recovery tool can charge per seat. The CLI, verifier,
and evidence export should stay free. A paid offer becomes testable only when durable team ledgers,
organization analytics, policy, SSO/RBAC, or deployment support produce repeated team value. Do not
copy a per-developer price before measuring who uses the product and how often.

## Distribution reality and cold-start channels

### GitHub CLI extension: pre-install Doctor and Resume, not the control-plane proof

GitHub's official extension documentation says extensions are local and user-scoped, third-party
extensions are not certified by GitHub, and users should audit their source. A repository must be
named `gh-*` and contain a same-named executable or release-attached precompiled binaries. Users can
then install and run it as:

```console
gh extension install OWNER/gh-stratadiff
gh stratadiff doctor https://github.com/OWNER/REPO/pull/123
```

Current GitHub CLI `2.97.0` supports `gh extension search`; with no query it returns extensions
sorted by stars. On the capture date, the GitHub Search API returned `994` repositories with the
`gh-extension` topic, and the leading third-party result, `dlvhdr/gh-dash`, had 12,478 stars. A new
extension will not receive meaningful empty-query discovery. Keyword probes returned multiple
results for `review`, one low-star direct result for `diff`, several for `rebase`, and none for
`force-push`. These are transient search observations, not demand estimates.

The extension repository therefore needs a literal problem description, not category jargon:

> Explain why this exact PR or queue candidate is blocked. Show the evidence chain or say what is
> unknowable; no App install and no queue migration.

Use the topics `gh-extension`, `code-review`, `pull-request`, `rebase`, `force-push`, and
`stacked-pr`. The README's first screen should contain one install command, one real before/after
recording, the no-network deterministic demo, and one canonical PR-URL command. Stars may improve
extension search rank, but stars are not activation or retention.

**Observed distribution state:** `gcomfident-crypto/stratadiff` and the separate public
[`gcomfident-crypto/gh-stratadiff`](https://github.com/gcomfident-crypto/gh-stratadiff)
distribution repository now publish matching immutable `v0.4.1` releases. The extension release
contains correctly named binaries for the four supported Linux and macOS targets, and its release
workflow completed successfully. In an operator-observed, non-benchmarked Linux x86-64 smoke run
on 2026-09-06, the pinned extension installed and `gh stratadiff --version` plus `--help` ran
successfully. The command log and environment were not captured as a reproducible artifact, and
the run reused an already provisioned GitHub CLI, Git, network, and authentication. It is therefore
not a fresh-machine activation, a comparable timing result, or proof that Resume reached useful
residue. At capture the main
repository was three days old, with one star and one fork; per-asset download counters were single
digits and include binaries, checksums, attestations, tests, retries, and upgrades. There is now a
working acquisition path, but no defensible activation or retention funnel yet.

**Observed release activation failure:** on 2026-09-06, the published immutable `v0.4.1`
Linux x86-64 binary was installed into an empty temporary directory through the documented
installer. SHA-256, GitHub artifact attestation, source tag, embedded build revision
`b10383a09793cb1c0003a5f5dd5691cbbdbe2244`, and reported version all verified. From that directory,
`stratadiff resume https://github.com/home-assistant/core/pull/176296 --no-open` failed its internal
exact-provider-commit fetch timeout after 120 seconds (`124.05` seconds wall time, `46288` KiB
maximum RSS). Live process inspection showed `index-pack` receiving a pack header with 1,362,095
objects. This is one reproducible large-repository failure, not a latency distribution; it proves
that release installation works while the released fetch path does not meet the large-repository
activation promise.

**Observed development smoke, not release evidence:** on 2026-09-06, the dirty `0.5.0`
development build resumed `home-assistant/core#176296` from its still-observable
`CHANGES_REQUESTED` checkpoint in an isolated repository. Cold start to a ready Workbench took
`33.152600050` seconds. The exact result contained 19 current PR files, 15 exact-identity carries,
and four residue files. All four file-level sessions independently regenerated a verified patch,
and all eight before/after source requests returned HTTP 200 with byte lengths matching their
records while the Workbench process had `GIT_NO_LAZY_FETCH=1` and no GitHub credential variables.
Interrupting the parent removed the child, listener, and temporary object store. This single,
operator-observed run validates the current end-to-end partial-clone path only; it is not a clean
install, release benchmark, latency distribution, reviewer-value result, or general residue-rate
claim.

The complete-ancestry path was separately smoke-tested on frozen ReviewTransition case
`github/gh-stack#185`. A development build reached Workbench readiness in `34.142823` seconds with
`262960` KiB maximum observed child RSS, exposed 24 resume entries and 7 base-drift entries, and
served all 62 corresponding before/after source requests with HTTP 200 and matching byte lengths.
Eleven resume entries and all seven base entries had independently verified structural reports;
13 unsupported resume entries retained their source snapshots and returned the designed 422 detail
response rather than a fabricated report. SIGINT left no temporary repository. This is another
single-case engineering smoke, not a distributional performance or safety result.

Before any broad launch post, reproduce install-to-Workbench activation on fresh supported Linux
and macOS environments and publish the exact failures and timings. GitHub CLI does not verify the
adjacent checksum or provenance bundle when installing an extension, and GitHub does not certify
or endorse third-party extensions, so wording such as “GitHub-verified extension” is forbidden.

### GitHub App and Marketplace: earned recorder, then live-validation surface

GitHub documents that a public App can be installed directly from the app owner without a
Marketplace listing. Organization owners can install Apps; repository admins may install an App on
repositories they administer only when it requests neither organization permissions nor repository
administration, and an organization owner can disable that ability. Other members can request an
installation, which still introduces an owner gate.

Marketplace is therefore not required for the first public App beta. It is also not an immediate
paid channel: GitHub's current requirements say a paid GitHub App needs at least 100 installations,
a verified publisher organization, purchase-event handling, and monthly plus annual billing.

**Decision:** keep the remote CLI extension as the pre-install Doctor, verification, and Resume
path. Run Preflight with the owner's existing authorized account rather than making an App the first
ask. Offer a direct-install public App first in recorder-only mode after verified reuse and a
confirmed ephemeral-evidence gap. Repository Administration read and Checks write are requested
only for later Preflight automation or shadow-proof promotion; they are not bundled into recorder
onboarding without need. The Check remains absent in recorder mode and non-required in shadow mode
until live evidence meets the enforcement gates. Pursue Marketplace after retained direct installs,
not as a way to manufacture the first 100 installations.

### Channel priority

| Priority | Channel experiment | Why it fits the trigger | Attribution and success signal |
|---|---|---|---|
| 1 | Consent-based outreach to DevEx/platform owners whose repositories use multiple review sources and strict checks or merge queue | Reaches the buyer while configuration and duplicated-work evidence is inspectable | Doctor verified and reused → Preflight completed → recorder requested |
| 2 | One reproducible public Doctor/Preflight and merge-group case study with downloadable proof | Lets technical users inspect exact GitHub objects instead of trusting security marketing | Verified artifact downloads, qualified recorder installs, remediated findings |
| 3 | Partner with reviewer vendors and merge-queue consultants on an open attestation adapter | Makes StrataDiff complementary rather than a replacement | Second provider connected, joint design partner, adapter replay conformance |
| 4 | GitHub Actions, DevEx, platform-engineering, and stacked-PR communities where self-promotion is allowed | Concentrates owners of the actual policy and queue problem | Opt-in audit applications and completed shadow weeks, not impressions |
| Later | Marketplace and broader content distribution | Removes install friction only after permissions and value are justified | Weekly Verified Merges, shadow-to-enforce conversion, week-four retained repos |

Do not begin with paid ads, a broad Product Hunt launch, mass email, automated issue/PR comments,
or replies on unrelated force-push issues. Those channels create low-intent installs or spam before
the remote install and evidence loop work. GitHub repository traffic reports only a rolling 14-day
view/clone window; release downloads include upgrades and automation. Use them as diagnostics, not
activated-user counts.

## The minimum Doctor-to-enforcement loop

The loop must create value before asking the team to block a merge:

```text
pre-install Doctor on one live PR
  -> the same owner verifies and reuses the diagnosis
  -> non-mutating repository Preflight confirms the repeated class
  -> dedicated App records future ephemeral candidates without gating
  -> retained events replay independently
  -> the App runs a non-blocking shadow proof
  -> false-red, latency, duplicate work, and override behavior are measured
  -> owner remediates configuration and requests enforcement
  -> exact final-candidate proof becomes the one required Check
```

The Doctor/Preflight Report is the smallest useful share object. A later recorder or Shadow Report
adds prospective lifecycle evidence. Each should include:

- repository and ruleset identifiers, or privacy-preserving hashes for a private pilot;
- required Check names and expected sources, PR-head and merge-group candidate IDs;
- provider evidence IDs, policy generation, overrides, missing obligations, carry predicates,
  abstentions, and collection completeness;
- tool/schema versions, evidence digest, and an independent verification command;
- evidence-to-Check latency, repair time, actual review/CI invocation counts, and any measured cost;
- one explicit non-claim: it is not a bug-free verdict and does not prove a person read every byte;
- canonical Audit, inspect, and shadow-install commands with a source attribution code.

“Read-only” describes an Audit that does not mutate repository state; it does not mean every field is
available with low privilege. Effective branch rules and repository ruleset listings can be read with
metadata access, while branch-protection and rule-suite endpoints require repository Administration
read. Rule-suite history is limited to an hour, day, week, or month. GitHub returns `bypass_actors`
only when the caller has write access to the ruleset. The report must record its granted permissions
and field visibility, and must classify an unavailable bypass list as unknown rather than as an
empty list. The direct per-ref Check Runs endpoint considers only the 1,000 most recent Check Suites;
an auditor must enumerate suites separately beyond that bound or mark collection incomplete.

It must omit source, patches, titles, bodies, review/comment text, emails, and credentials.
Generation is manual and opt-in; StrataDiff must never auto-post marketing material to a PR during
the pilot.

The report is more defensible than a generic referral link because it carries the exact
configuration or evidence gap, the candidate it affected, and an independent replay path. It gives
the repository owner a concrete remediation task without pretending to carry a human approval.

**Organization expansion trigger:** do not request an App after one curiosity click. Offer
recorder-only installation only after the same owner has verified and reused Doctor or Preflight,
and either an ephemeral candidate prevented a conclusive diagnosis or the owner explicitly asks to
retain future candidate evidence. Promote recorder mode to a non-blocking shadow trial only after a
retained event replays independently and permission/retention boundaries are accepted. A required
Check and paid controls remain separate later asks.

## Instrumentation required before recruitment

Do not add default network telemetry. Record an append-only pilot ledger only after explicit
participant consent, keep its pseudonymous raw log private, and export only the aggregate report.
The primary control-plane funnel is:

```text
qualified_repository
  -> doctor_result_verified
  -> doctor_reused
  -> preflight_completed
  -> finding_adjudicated
  -> recorder_requested
  -> candidate_recorded
  -> recording_replayed
  -> shadow_installed
  -> candidate_observed
  -> proof_converged
  -> enforcement_requested
  -> enforcement_enabled
  -> week_four_retained
```

Each event binds a repository pseudonym, App installation, policy generation, exact candidate type
and SHA, collection completeness, result, latency, and predecessor event. `proof_converged` requires
a terminal shadow result plus an independently replayable ledger; pending, unknown, quarantined, or
partially collected candidates do not count. `enforcement_requested` is an explicit owner decision,
not an inference from repeated dashboard use. Publish repository and candidate denominators for
every conversion, including clean Audits and removed Apps.

The separate local Resume track keeps its existing delivery-confirmed funnel:

```text
baseline
  -> gap_discovery
  -> inbox_delivery
  -> resume_invoked
  -> transition_bound
  -> covered_transition
  -> workbench_ready
  -> optional staged failure
```

Definitions:

- `baseline`: one successful Inbox collection recorded counts and whether its queue was complete.
  It is not evidence of a clean installation or of a failed scan that never reached collection.
- `gap_discovery`: Inbox found one exact, pseudonymous transition for a completed review whose
  checkpoint differs from the current head. This pre-output event does not claim delivery.
- `inbox_delivery`: the selected output sink accepted and flushed the complete Inbox. One marker
  confirms delivery for all discoveries in that scan; it does not prove that a person read them.
  A missing marker means delivery is unconfirmed, not necessarily that output failed.
- `resume_invoked`: the user deliberately selected Resume; a fresh random attempt ID is recorded
  before network and Git work starts.
- `transition_bound`: Resume revalidated that the attempt still names the same host, repository,
  PR, reviewer, review ID/state, checkpoint commit, and head commit.
- `covered_transition`: exact residue and session accounting completed, including fallbacks and
  abstentions. Discovery alone never counts as coverage.
- `workbench_ready`: the loopback Workbench listener became usable.
- `attempt_failed`: a terminal event classified as before binding, before coverage, before
  Workbench readiness, or after readiness.

The raw log excludes source, diffs, filenames, PR/review text, commit messages, credentials, raw API
responses, and plaintext identities. Its deterministic transition digest remains linkable and may be
reidentifiable for public repositories, so it is not a share artifact. The aggregate report exports
no transition IDs and explicitly disclaims clean-install success, time savings, issue recall, and
market prevalence. Future opt-in events may cover Recovery Cards, teammate activation, and a later
formal review, but those are not implemented in this instrument.

Every funnel rate must publish its denominator. In particular:

```text
delivered-gap-to-resume = unique transitions both delivered and resumed / unique delivered transitions
resume-to-covered  = covered attempts / resume attempts
covered-to-ready   = workbench-ready attempts / covered attempts
resume-to-ready    = workbench-ready attempts / resume attempts
```

The Resume funnel cannot by itself measure clean-install activation, human attention to delivered
output, repeat use by participant, or repository retention. Do not infer control-plane demand from
the post-delivery funnel; demand comes from the Doctor-to-recorder-to-shadow conversions above.

## Four-week post-reuse pilot

The four-week clock starts only after the same owner has verified and reused Doctor or Preflight,
the recorder schema and minimal App permissions are frozen, and the owner confirms the retention
need. Recruit at least five independent repositories matching the ICP, with at least two real
reviewer/check sources across the cohort. A source checkout, synthetic unit test, or repository that
cannot exercise a final candidate does not count as activation. The separate 100-session,
20-reviewer study remains required before a human-time or recall claim.

### Week 1: bounded retrospective Preflight and baseline

- Inventory effective rulesets, expected App IDs, duplicate context names, workflow triggers, path
  filters, visible bypass actors, reviewer providers, and still-resolvable PR evidence. Record hidden
  bypass configuration as unknown. GitHub.com's and GHES 3.22's path-filter boundary is 3,000 files,
  while GHES 3.17–3.21 use 300; bind the deployment/version before applying either limit. If it is
  unavailable, retain an unknown above 300. The pull-files endpoint returns at most 3,000 files, so
  a capped response is not proof that the ordered evaluation set is complete.
- Statically check whether required workflows declare `merge_group`. Audit a merge-group timeline or
  member set only from retained live events or other explicitly validated records. The `merge_group`
  webhook does not enumerate member PRs, and a first-time Audit cannot reconstruct an uncollected
  historical membership timeline; mark that coverage unknown.
- Ask the owner to adjudicate each finding against the underlying GitHub object: actionable,
  expected, false positive, or unknown.
- Record provider/CI invocation counts, merge latency, stale or missing evidence, and overrides for
  the same bounded historical window. Never translate a missing API object into a defect.
- Continue only with repositories that have a measurable problem or explicitly value the audit
  trail; report “not useful here” for the rest.

### Week 2: recorder-only observation

- Install the dedicated App with only the permissions needed to observe the agreed events. Publish
  no comments or Check; bind each record to exact provider objects, source identities, visible
  policy generation, PR head, and candidate SHA.
- Exercise a real native merge queue where available. Compare every recorded lifecycle with
  GitHub's later observable state and a human-adjudicated evidence ledger.
- Export and independently replay at least one retained candidate lifecycle. A missing delivery,
  partial policy view, or unproved membership stays unknown and blocks promotion to shadow mode.

### Week 3: non-blocking shadow proof and remediation

- Promote only replay-valid recorder installations to a non-blocking shadow Check. Record false
  green, false red, unknown, evidence-to-Check latency, repair latency, and every quarantine or
  override. Any false green pauses the pilot and blocks distribution.
- Let owners fix wrong-source checks, missing `merge_group` triggers, duplicate names, or provider
  policy gaps; verify each repair from a new snapshot rather than assuming success.
- Enable dispatch/cache optimization only in consenting repositories and compare actual reviewer
  invocations, CI minutes, billed usage, and merge latency with the frozen baseline.
- Publish a case study only with repository approval and downloadable evidence, including cases
  where StrataDiff found nothing or saved nothing.

### Week 4: ROI and enforcement decision

- Ask each owner whether to keep shadow mode, remove the App, or make the canonical Check required.
- Measure Weekly Verified Merges, shadow-to-enforce conversion, week-four repository retention,
  false-red rate, p95/p99 convergence, and repair latency.
- Conduct loss interviews with every non-enforcer; classify no pain, native/reviewer-native controls
  sufficient, install or permission cost, evidence distrust, excessive false red, latency, or
  workflow mismatch.
- Publish an anonymized funnel with denominators, exclusions, partial scans, and failure reasons.
- Apply the gates below without changing thresholds after viewing the result.

## Precommitted pilot gates and kill criteria

These gates screen the acquisition and distribution thesis. The stricter safety and human-value
gates in `product-strategy.md` still apply.

| Dimension | Advance when | Stop, narrow, or pivot when |
|---|---|---|
| Safety | Zero wrong-candidate or wrong-source green results; 100% independent replay; every unsupported case visible | Any verified false green pauses enforcement; repeated verifier-unsound behavior ends proof positioning |
| Availability | Evidence-to-Check p95 <30 seconds and p99 <60 seconds; missed-delivery repair p99 <5 minutes | False-red rate ≥0.5% or unbounded pending states after one focused repair: remain shadow-only |
| Audit value | ≥10% actionable-gap rate in the qualified sample and ≥30% finding-action rate | <2% actionable gaps across 500 qualified consecutive PRs: stop stale-evidence positioning |
| Product pull | At least 3 of 5 qualified repositories request enforcement after two shadow weeks | Fewer than 2 request enforcement and most cite native/provider controls as sufficient: do not expand the hosted policy surface |
| Economic value | At least one buyer metric improves; target ≥40% fewer review invocations or ≥25% lower merge-ready-to-merge time without coverage loss | No measured improvement or worse final-candidate coverage: remove the optimization claim |
| Human value | Directionally at least 20% median active-time reduction with no critical/high issue-recall loss in the separate study | Lower critical/high recall or systematic over-trust: stop collapsing carried work and redesign |
| Retention | At least 3 repositories retain shadow or enforcement through week four and produce verified candidates | No retained qualified repository: keep only the offline verifier and stop hosted expansion |
| Channel quality | Direct audit outreach produces completed shadow trials; a case-study reader can replay the evidence | Channels produce stars/downloads but no completed Audit or shadow week: do not scale promotion |

The human optimization claim still requires at least 100 eligible sessions across at least 20
reviewers, median reviewer-time reduction of at least 20%, issue recall non-inferior within the
predeclared margin, and zero false carry. A successful control-plane pilot does not waive that
separate study.

## Exact launch claims

### Allowed after the corresponding behavior is reproduced

- “Audit the required-check sources, reviewer obligations, and native merge-queue triggers that
  determine whether this repository can produce a complete merge proof.”
- “Bind reviewer and Check evidence to this exact PR head or synthetic `merge_group` candidate,
  with provider identity, policy generation, overrides, and missing obligations visible.”
- “Replay this shadow decision from its signed evidence ledger,” only when the exported proof
  independently reproduces the Check result and unsupported or unavailable evidence remains
  explicit.
- “One canonical Check expresses the repository's cross-provider evidence policy,” only for a
  repository that has opted into enforcement after meeting the published shadow gates.
- “Resume a GitHub review from your own completed checkpoint after the PR head changed.”
- “Analysis runs locally; the current CLI sends no source to a StrataDiff service.”
- “Every carried file names the deterministic predicate checked; unsupported cases remain in the
  review queue.”
- “The CLI path needs your existing GitHub read access and does not require repository
  administration.”
- “This case reduced the displayed current-file queue from X to Y,” only with the exact case,
  denominator, fallbacks, dropped changes, and downloadable evidence.

### Forbidden until separately proved

- “CodeRabbit leaves stale approvals valid after new commits,” “GitHub accepts an old successful
  Check for the latest candidate,” or “GitHub has no stale-review control.” Current CodeRabbit and
  GitHub contracts directly contradict those blanket claims.
- “Copilot approvals survive later commits,” or any claim that a public-preview defect is a stable
  product contract. New commits dismiss Copilot approvals; observed missing or duplicate reviews
  are test cases, not a durable category definition.
- “Every reviewer approved the merge,” “all required evidence exists,” or “merge proof complete”
  when collection is partial, the provider identity is ambiguous, a `merge_group` event was not
  evaluated, or policy-generation identity is missing.
- “The first/only incremental review tool,” “GitHub has no changes-since-review,” or “we invented
  review memory.” Graphite, Reviewable, GitHub, GitLab, Gerrit, and Aviator already cover parts of
  this job.
- “GitHub cannot show required checks or rule failures,” “queue vendors cannot explain blockers,”
  “App-qualified checks are unique,” or “nobody exposes an exact queue candidate.” GitHub CLI and
  Rules Insights, Mergify, Aviator, and Trunk each cover those claims in their own scope. The
  remaining hypothesis is pre-install, queue-neutral evidence composition with explicit unknowns.
- “Safe to merge,” “review unnecessary,” “approval preserved,” “semantically equivalent,” “zero
  risk,” or “no bugs missed.” A deterministic byte relation is not a safety verdict.
- “Works on every rebase/force-push/repository.” Commits can be unavailable, histories ambiguous,
  scans partial, file kinds unsupported, and GHES transport constrained.
- “No data leaves your machine” or “offline.” The CLI contacts GitHub and fetches Git objects;
  the narrower true claim is that current source analysis is local and no StrataDiff service
  receives source.
- “No admin required” without saying **CLI path**. Organization App installation can require an
  owner.
- “GitHub-verified extension.” GitHub explicitly says third-party CLI extensions are not certified,
  signed, or endorsed by GitHub.
- “27% of reviews need StrataDiff,” “18% of GitHub PRs,” or any weekly frequency extrapolated from
  the convenience probe or purposefully selected census.
- “61.7% less review time.” The current selected-case result is file-level recheck reduction, not
  measured human time.
- Marketplace installations, stars, release downloads, or repository clones described as users,
  active teams, retention, or revenue.
- `COMMENTED` treated as completed review, formal approval treated as proof every line was read, or
  a partial/truncated scan described as globally clean.

## Immediate execution order

1. Ship and replay `gh stratadiff doctor <PR>` against current public incidents. For each case,
   publish the exact observed candidate and evidence chain, and score false-confident diagnoses and
   correct abstentions. Do not require an App or queue migration.
2. Freeze the Merge Readiness Preflight schema and run it over a consented 500-PR retrospective
   sample only after owner-verified Doctor reuse. Publish denominators, incomplete collections,
   owner-adjudicated findings, and clean results. Do not treat absent historical merge-group
   membership or hidden bypass actors as clean evidence.
3. Offer recorder-only App installation to owners whose repeated use proves an ephemeral-evidence
   gap. Independently replay at least one retained candidate lifecycle before enabling a shadow
   Check bound to exact candidate, provider identities, policy generation, and evidence ledger.
4. Before recruitment expands, pass at least 80 live/adversarial GitHub cases that include expected
   App sources, duplicate context names, delivery reordering, head races, CodeRabbit and Copilot
   review objects, and a real native `merge_group` candidate. Unit-only synthetic cases do not
   satisfy this gate.
5. Recruit five qualified repositories through consent-based DevEx/platform-owner outreach and run
   the frozen four-week post-reuse recorder-to-shadow pilot. Publish false green, false red,
   unknown, convergence, repair, finding-action, and shadow-retention results with their
   denominators.
6. Offer the canonical required Check only to repositories that request enforcement and only after
   the safety, availability, product-pull, and retention gates pass. Marketplace and pricing follow
   retained direct installs, not the other way around.
7. Keep `v0.4.1` immutable and continue fresh-machine CLI/Resume validation as the local verification
   and reviewer-recovery track. It does not gate the first shadow App pilot and must not be used as
   evidence that the cross-provider control plane works.

## Sources

Sources in the original channel and pricing capture were accessed on 2026-09-06. Platform and
provider contracts below were refreshed on 2026-09-08.

- StrataDiff evidence: [Review Churn Census v1](../benchmarks/review-churn-census-v1/README.md),
  [Review Inbox v1](../benchmarks/review-inbox-v1/README.md), and
  [Reviewer Value v1](../benchmarks/reviewer-value-v1/README.md).
- GitHub CLI: [using extensions](https://docs.github.com/en/github-cli/github-cli/using-github-cli-extensions),
  [creating extensions](https://docs.github.com/en/github-cli/github-cli/creating-github-cli-extensions),
  and [`gh extension search`](https://cli.github.com/manual/gh_extension_search).
- GitHub extension ecosystem: [`gh-extension` topic search](https://github.com/search?q=topic%3Agh-extension&type=repositories&s=stars&o=desc)
  and [`dlvhdr/gh-dash`](https://github.com/dlvhdr/gh-dash). Repository count and stars are API
  snapshots, not install counts.
- GitHub Apps: [installing a third-party App](https://docs.github.com/en/apps/using-github-apps/installing-a-github-app-from-a-third-party),
  [Marketplace listing requirements](https://docs.github.com/en/apps/github-marketplace/creating-apps-for-github-marketplace/requirements-for-listing-an-app),
  and [repository traffic API](https://docs.github.com/en/rest/metrics/traffic).
- GitHub merge controls: [troubleshooting required status checks](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks),
  [about protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches),
  [managing a merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue),
  the [`merge_group` webhook](https://docs.github.com/en/webhooks/webhook-events-and-payloads#merge_group),
  [rules endpoints](https://docs.github.com/en/rest/repos/rules?apiVersion=2022-11-28), and
  [rule-suite endpoints](https://docs.github.com/en/rest/repos/rule-suites?apiVersion=2022-11-28).
- Native diagnosis: [`gh pr checks`](https://cli.github.com/manual/gh_pr_checks),
  [Rules Insights](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository#viewing-insights-for-rulesets),
  and [dcoapp/app #303](https://github.com/dcoapp/app/issues/303).
- Graphite: [GitHub App authentication](https://graphite.com/docs/authenticate-with-github-app),
  [PR Versions](https://graphite.com/docs/pull-request-versions),
  [PR Inbox](https://graphite.com/docs/use-pr-inbox),
  [Graphite Merge Queue](https://graphite.com/docs/graphite-merge-queue),
  [team proof of concept](https://graphite.com/docs/onboarding-your-team),
  [billing](https://graphite.com/docs/billing-plans), and
  [Marketplace listing](https://github.com/marketplace/graphite-dev).
- Reviewable: [registration and GitHub authorization](https://docs.reviewable.io/registration),
  [repository connection](https://docs.reviewable.io/admincenter#connecting-repositories), and
  [pricing](https://www.reviewable.io/pricing/).
- Aviator: [FlexReview onboarding](https://docs.aviator.co/flexreview/getting-started),
  [read-only mode](https://docs.aviator.co/flexreview/concepts/read-only-mode), and
  [pricing](https://www.aviator.co/pricing/).
- Queue diagnostics: [Mergify queue monitoring](https://docs.mergify.com/merge-queue/monitoring/),
  [Mergify App-qualified checks](https://docs.mergify.com/changelog/2026-06-02-scope-check-conditions-to-a-specific-github-app/),
  [Aviator `av pr status`](https://docs.aviator.co/aviator-cli/manpages/av-pr-status-1),
  [Aviator pending workflows](https://docs.aviator.co/mergequeue/concepts/pending-workflow-runs), and
  [Trunk testing details](https://docs.trunk.io/merge-queue/reference/merge/get-details-about-testing-that-merge-queue-is-performing).
- CodeRabbit: [GitHub setup](https://docs.coderabbit.ai/platforms/github-com),
  [test-repository onboarding](https://docs.coderabbit.ai/guide/repository),
  [Request Changes Workflow](https://docs.coderabbit.ai/pr-reviews/request-changes-workflow),
  [automatic-review controls](https://docs.coderabbit.ai/configuration/auto-review),
  [plans](https://docs.coderabbit.ai/management/plans), and
  [Marketplace listing](https://github.com/marketplace/coderabbitai).
- Copilot code review: [approval public preview](https://github.blog/changelog/2026-09-01-copilot-code-review-can-now-approve-pull-requests/),
  [duplicate re-review report](https://github.com/orgs/community/discussions/190754), and
  [zero-approval field report](https://github.com/orgs/community/discussions/206810).
- GitHub Community merge-queue reports: [duplicate check runs](https://github.com/orgs/community/discussions/103114)
  and [checks coupled to `merge_group`](https://github.com/orgs/community/discussions/43988).
