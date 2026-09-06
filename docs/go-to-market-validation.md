# Go-to-market validation: make review recovery discoverable at the moment of need

Evidence captured: **2026-09-06**. Prices, installation counters, repository counters, search
results, and product behavior are point-in-time observations. This document is a distribution and
pilot plan, not evidence of product-market fit. The technical thesis and prior-art boundary remain
in [`product-strategy.md`](product-strategy.md); they are not repeated here.

## Decision

StrataDiff should be sold as **review continuity after a PR changes**, not as another diff viewer,
AI reviewer, generic inbox, or approval bot:

> A PR changed after your completed review. StrataDiff recovers your exact checkpoint, proves what
> can still be accounted for, and opens everything that still needs your attention.

The user outcome is a faster, less error-prone return to an interrupted review. The economic
outcome to test is lower reviewer minutes and shorter time from a post-review update to the next
completed review, without lower issue recall. Files hidden, lines hidden, generated summaries, and
installation counts are not value metrics.

The most important go-to-market finding is a **discovery problem**:

- The repository census found real checkpoint drift, but not a universal daily job.
- The current CLI can prove value without administrator access, but it cannot notify someone before
  they remember to run it.
- A reviewer is unlikely to install a tool today for a rewrite that may happen weeks later.

Therefore the local CLI and Review Inbox are the validation and trust path, not the final mass
distribution surface. If the pilot passes, the scalable product is a narrowly triggered GitHub App
Check that appears only after an existing completed human review and a later head change, with one
**Resume review** action into the local Workbench. The App discovers the moment; the local engine
handles source and verifies the evidence. A broad PR bot, routine comments on unaffected PRs, or a
replacement review UI would add noise without solving discovery.

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

The first installer is defined by a current event, not by a broad persona:

1. They have an open PR on which they already submitted `APPROVED` or `CHANGES_REQUESTED`.
2. The PR head has since changed and both exact object IDs remain observable.
3. Native GitHub or their current review tool does not give them a trustworthy, sufficiently narrow
   path back into the review.
4. They already use `gh`, or will run one copyable command for a live PR without asking an
   organization owner.

The highest-probability initial users are staff engineers, code owners, and open-source maintainers
in rewrite-heavy, stacked, monorepo, migration, generated-code, or agent-heavy workflows. The first
economic champion is a DevEx or platform lead only after at least two reviewers in the same team
have completed useful Resume loops. This separates three roles:

| Role | Immediate job | Required proof before the next ask |
|---|---|---|
| Affected reviewer | Resume this review without reconstructing a range | The Workbench opens the exact event and exposes every unsupported case |
| PR author or maintainer | Get a changed PR reviewed again without asking for a blind re-approval | A reviewer-verified case card shows what was carried and what remained |
| DevEx/platform owner | Remove repeated review work without weakening policy | Multiple real sessions show time saved, no false carry, and repeat use |

**Inference:** acquisition should target a reviewer with an actionable event now. Targeting “all
developers who review code” wastes attention because many will see no value during onboarding.

### Anti-ICP for the first four weeks

Do not recruit solo developers, teams dominated by tiny append-only PRs, people primarily seeking
bug findings, or organizations that require an administrator-approved App before any individual
experiment. Do not lead with security, semantic equivalence, automated approval, or AI review.

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
| GitHub native | Already present in every GitHub PR; no additional install or workflow migration | Bundled with the host | Any generic “changes since review,” viewed-file, approval, or PR Inbox pitch loses to a zero-install default |
| Graphite | New signups install or request the Graphite GitHub App on an organization, then personally authorize it; Graphite says organization owners install the App. Its PR Versions view offers **Hide reviewed changes**, and its Inbox can export shareable filter links. | Hobby free; Starter `$20/seat/month` and Team `$40/seat/month`, billed annually; 30-day Team trial. The Marketplace listing embedded `61,296` installations at capture. | A broad workflow suite can justify admin installation and per-seat pricing. StrataDiff cannot ask for that commitment before one recovered review proves value |
| Reviewable | GitHub OAuth sign-in requests broad scopes; a repo admin connects a repository. A connected repo automatically creates reviews and inserts a Reviewable link into PR descriptions. | Public/personal free; Team `$8` and Business `$16` per contributor/month billed annually; private repo connection may start a 30-day trial | Persistent review state is valuable, but workflow migration, write access, and automatic PR links are material adoption costs. The local pilot should remain no-admin and non-invasive |
| Aviator FlexReview / Review | Connect the Aviator GitHub App, activate a repo in read-only mode, wait for history indexing, test through slash commands/dashboard, and then activate selected teams; validation can later become a required check | Current pricing page advertises Free, Team `$20/dev/month` with a 14-day trial, Scale `$50/dev/month`, and custom Enterprise | Read-only shadow mode is an effective enterprise bridge. StrataDiff should copy the staged-risk pattern, not claim selective validation or owner routing as novel |
| CodeRabbit | GitHub login, organization/repository selection, App installation and authorization, then an existing PR can be selected for an immediate first review. Its docs require owner-level access for a repository and organization-owner access for an organization, and list repository read/write permissions. | Public/OSS free; Essentials `$24`, Team `$48` per developer/month billed annually, Advanced `$90` monthly. Its Marketplace listing embedded `318,416` installations at capture | Immediate value on an existing PR and visible PR output drive acquisition. Installation count demonstrates reach, not active users, review quality, revenue, or demand for StrataDiff |

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

### GitHub CLI extension: fastest proof, weak ambient discovery

GitHub's official extension documentation says extensions are local and user-scoped, third-party
extensions are not certified by GitHub, and users should audit their source. A repository must be
named `gh-*` and contain a same-named executable or release-attached precompiled binaries. Users can
then install and run it as:

```console
gh extension install OWNER/gh-stratadiff
gh stratadiff inbox
```

Current GitHub CLI `2.97.0` supports `gh extension search`; with no query it returns extensions
sorted by stars. On the capture date, the GitHub Search API returned `994` repositories with the
`gh-extension` topic, and the leading third-party result, `dlvhdr/gh-dash`, had 12,478 stars. A new
extension will not receive meaningful empty-query discovery. Keyword probes returned multiple
results for `review`, one low-star direct result for `diff`, several for `rebase`, and none for
`force-push`. These are transient search observations, not demand estimates.

The extension repository therefore needs a literal problem description, not category jargon:

> Recover reviewer checkpoints after rebase, force-push, restack, or new commits; inspect the
> evidence-backed review residue locally.

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

Before any broad launch post, reproduce install-to-Workbench activation on fresh supported Linux
and macOS environments and publish the exact failures and timings. GitHub CLI does not verify the
adjacent checksum or provenance bundle when installing an extension, and GitHub does not certify
or endorse third-party extensions, so wording such as “GitHub-verified extension” is forbidden.

### GitHub App and Marketplace: scale surface, not first validation

GitHub documents that a public App can be installed directly from the app owner without a
Marketplace listing. Organization owners can install Apps; repository admins may install an App on
repositories they administer only when it requests neither organization permissions nor repository
administration, and an organization owner can disable that ability. Other members can request an
installation, which still introduces an owner gate.

Marketplace is therefore not required for the first public App beta. It is also not an immediate
paid channel: GitHub's current requirements say a paid GitHub App needs at least 100 installations,
a verified publisher organization, purchase-event handling, and monthly plus annual billing.

**Decision:** use the remote CLI extension for the no-admin proof. Build a direct-install public App
only after the four-week pilot demonstrates real Resume use. Pursue Marketplace after retained
direct installs, not as a way to manufacture the first 100 installations.

### Channel priority

| Priority | Channel experiment | Why it fits the trigger | Attribution and success signal |
|---|---|---|---|
| 1 | Consent-based outreach to reviewers with a currently actionable public event and to maintainers of qualified repositories | Reaches the user while the cost is present; no prevalence claim is needed | Unique pilot code; install → Inbox → Resume → Workbench funnel |
| 2 | One reproducible public case study with downloadable, independently verifiable evidence | Lets technical users inspect the claim instead of trusting marketing | Source-tagged landing link, verified artifact downloads, qualified installs |
| 3 | `gh-extension` topic and keyword-accurate repository metadata | Makes the direct command credible and captures intent queries | Search position for exact pain terms, repository traffic, clean installs |
| 4 | Maintainer, DevEx, stacked-PR, and migration-tool communities where self-promotion is allowed | Concentrates likely workflows and possible integrations | Opt-in pilot applications and completed eligible sessions, not impressions |
| Later | Direct public GitHub App, then Marketplace | Removes speculative polling after value and permissions are justified | Direct App installs, activated repositories, weekly verified Resume loops |

Do not begin with paid ads, a broad Product Hunt launch, mass email, automated issue/PR comments,
or replies on unrelated force-push issues. Those channels create low-intent installs or spam before
the remote install and evidence loop work. GitHub repository traffic reports only a rolling 14-day
view/clone window; release downloads include upgrades and automation. Use them as diagnostics, not
activated-user counts.

## The minimum sharing and organization-spread loop

The loop must benefit two people without asking the whole team to migrate:

```text
post-review head change
  -> affected reviewer resumes locally
  -> reviewer exports an opt-in, source-free Review Recovery Card
  -> author or teammate sees exact accounting plus a verify/resume command
  -> second reviewer installs and verifies their own checkpoint
  -> repeated loops qualify the repository for a read-only App pilot
```

The Review Recovery Card is the smallest useful share object. It should include:

- provider/repository/PR identifiers, or privacy-preserving hashes for a private pilot;
- reviewer-specific checkpoint and current head IDs;
- current items carried by named proof, items requiring review, dropped reviewed changes, base
  drift, abstentions, and collection completeness;
- tool/schema versions, evidence digest, and an independent verification command;
- elapsed analysis time and the user's optional self-reported review minutes;
- one explicit non-claim: it is not an approval, safety verdict, or permission to skip review;
- a canonical installation and Resume command with a source attribution code.

It must omit source, patches, titles, bodies, review/comment text, emails, credentials, and the
reviewer's approval state unless the reviewer explicitly elects to disclose it. Generation is
manual and opt-in; StrataDiff must never auto-post it to a PR during the pilot.

The card is more defensible than a generic referral link because it carries the reason to try the
product. It also gives the author an incentive to share: faster re-review. The receiving reviewer
still independently resolves their own checkpoint; an author-produced card cannot carry a human
approval.

**Organization expansion trigger:** after at least five verified Resume loops by at least two
reviewers in one organization, offer a read-only App shadow trial. The App should report eligible
events without comments or policy changes. Team validation, required checks, and paid controls are
separate later asks.

## Instrumentation required before recruitment

Do not add default network telemetry. Record an append-only local pilot funnel only after explicit
participant consent, keep its pseudonymous raw log private, and export only the aggregate report:

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

This instrumentation cannot yet measure clean-install activation, human attention to delivered
output, repeat use by participant, or week-four retention. Do not infer those claims from the
post-delivery funnel.

## Four-week pilot

The four-week clock starts only after an installable remote extension release, supported release
artifacts, a clean-machine installation test, local-consent instrumentation, and a frozen analysis
plan exist. A source checkout does not count as onboarding.

Recruit 18–30 reviewers across at least three independent teams or maintainer groups. Each cohort
should contain 5–10 participants where possible, matching the size Graphite recommends for its own
team proof of concept while avoiding dependence on one organization. Pre-qualify each cohort with a
bounded repository Audit or at least one current complete Inbox event. The discovery pilot should
aim for at least 60 active reviewer-weeks and 30 eligible live sessions. It is a screening study;
the larger 100-session, 20-reviewer counterbalanced value study remains required before a broad
performance claim.

### Week 1: activation and baseline

- Assign source-specific pilot codes for direct event outreach, maintainer referral, case study,
  and extension search.
- Observe clean installation on supported Linux and macOS environments; record every failure and
  time from the install command to `workbench_ready`.
- Run Inbox and one canonical PR-URL Resume without a checkout, `-R`, or manual SHA.
- Capture the prior native workflow for the next eligible event: time, files revisited, confidence,
  findings, and whether the reviewer reconstructed a commit range.
- Interview only after the participant attempts the live job; ask what they would have done without
  StrataDiff, not whether the idea sounds useful.

### Week 2: live Resume loops

- Deduplicate and revalidate every event immediately before use.
- Randomize eligible seeded tasks between native GitHub first and StrataDiff first for the separate
  time/recall check; do not infer issue recall from live PR approval state.
- For live work, record discovery source, actionable-to-Resume conversion, Workbench readiness,
  review completion latency, self-reported active minutes, abstentions, overrides, and failures.
- Independently inspect every carried predicate. Any false carry pauses the pilot and blocks
  distribution.

### Week 3: sharing and within-team spread

- Let successful users manually export a Review Recovery Card for an opted-in public case or a
  private redacted channel.
- Invite one additional reviewer through the card, without automated PR comments.
- Publish one case study only after the participant and repository approve it. Show complete
  accounting and downloadable evidence, including cases where StrataDiff saved nothing.
- Offer a read-only organization shadow report only to a cohort that has already completed five
  loops across two reviewers.

### Week 4: repeat use and decision

- Measure event rate per active reviewer-week, conditional repeat use, unconditional week-four
  activity, and time from new head to next completed review.
- Conduct loss interviews with every non-activator and every user who skipped an actionable event;
  classify no pain, native flow sufficient, install/permission failure, evidence distrust, poor
  coverage, or workflow mismatch.
- Publish an anonymized funnel with denominators, exclusions, partial scans, and failure reasons.
- Apply the gates below without changing thresholds after viewing the result.

## Precommitted pilot gates and kill criteria

These gates screen the acquisition and distribution thesis. The stricter safety and human-value
gates in `product-strategy.md` still apply.

| Dimension | Advance when | Stop, narrow, or pivot when |
|---|---|---|
| Safety | Zero independently adjudicated false carries; all unsupported cases remain visible | Any verified false carry pauses release; repeated verifier-unsound behavior ends proof-carrying positioning |
| Installation | At least 80% of eligible supported-environment attempts reach `workbench_ready`; median install-to-value at most 5 minutes | Below 50% after one focused onboarding repair: stop promotion and fix distribution |
| Trigger frequency | At least 0.25 distinct actionable events per active reviewer-week in the qualified segment | Below 0.25 over at least 60 active reviewer-weeks: stop positioning Inbox as a recurring habit; pivot discovery to event-triggered App/author handoff |
| Product pull | At least 40% actionable-to-Resume conversion | Below 25% across at least 30 complete events, with native flow cited as sufficient by most skippers: the standalone wedge is weak |
| Repeat | At least 50% of participants who receive a second eligible event Resume again; at least 30% of activated participants use the product in week four | Fewer than 30% conditional repeats after 20 second-event opportunities: stop adding workflow surface and re-test the job |
| Human value | Directionally at least 20% median active-time reduction with no critical/high issue-recall loss; confirm later on 100 sessions | No time reduction, lower critical/high recall, or systematic over-trust: stop collapsing carried work and redesign |
| Organization spread | At least three independent teams complete live loops; at least two teams reach two reviewers | No second-reviewer activation after ten correctly delivered, useful cards: kill the card/referral loop |
| Channel quality | Direct event outreach produces completed eligible sessions; a case-study reader can reproduce the evidence | Channels produce stars/downloads but no `workbench_ready` or Resume event: do not scale spend or launch volume |
| Admin appetite | A team with repeated value requests or accepts a read-only App trial | No qualified team accepts App permissions after demonstrated value: keep a CLI product and do not build hosted policy |

The final standalone-product gate remains at least 100 eligible sessions across at least 20
reviewers, median reviewer-time reduction of at least 20%, issue recall non-inferior within the
predeclared margin, zero false carry, and credible fourth-week repeat use. A successful four-week
screen does not waive that study.

## Exact launch claims

### Allowed after the corresponding behavior is reproduced

- “Resume a GitHub review from your own completed checkpoint after the PR head changed.”
- “Analysis runs locally; the current CLI sends no source to a StrataDiff service.”
- “Every carried file names the deterministic predicate checked; unsupported cases remain in the
  review queue.”
- “The CLI path needs your existing GitHub read access and does not require repository
  administration.”
- “This case reduced the displayed current-file queue from X to Y,” only with the exact case,
  denominator, fallbacks, dropped changes, and downloadable evidence.

### Forbidden until separately proved

- “The first/only incremental review tool,” “GitHub has no changes-since-review,” or “we invented
  review memory.” Graphite, Reviewable, GitHub, GitLab, Gerrit, and Aviator already cover parts of
  this job.
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

1. Keep the published `v0.4.1` extension immutable, and reproduce install-to-Workbench activation
   on fresh supported Linux and macOS environments before a broad announcement. Publish the exact
   timings, transferred bytes, on-disk bytes, and failures instead of treating the existing Linux
   installation smoke test as proof of useful activation.
2. Finish and release-test the local consent-based funnel and content-addressed event plus unique
   attempt IDs before recruiting anyone; the current instrument measures only delivery-confirmed
   activation.
3. Keep the 60/60 Rust target-policy conformance adapter as a CI gate, and separately gate the
   stricter executable-Resume policy used by the live collector. Then freeze at least 30
   live/adversarial Inbox-to-Resume cases covering pagination, missing objects, base drift, dropped
   reviewed changes, and explicit failures.
4. Recruit current-event reviewers through consent-based direct outreach; run the four-week pilot.
5. Implement the opt-in Review Recovery Card only after the basic Resume funnel works.
6. If and only if the pilot gates pass, build a read-only direct-install GitHub App beta. Marketplace,
   policy enforcement, and pricing come after retained direct use.

## Sources

All web sources were accessed on 2026-09-06.

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
- Graphite: [GitHub App authentication](https://graphite.com/docs/authenticate-with-github-app),
  [PR Versions](https://graphite.com/docs/pull-request-versions),
  [PR Inbox](https://graphite.com/docs/use-pr-inbox),
  [team proof of concept](https://graphite.com/docs/onboarding-your-team),
  [billing](https://graphite.com/docs/billing-plans), and
  [Marketplace listing](https://github.com/marketplace/graphite-dev).
- Reviewable: [registration and GitHub authorization](https://docs.reviewable.io/registration),
  [repository connection](https://docs.reviewable.io/admincenter#connecting-repositories), and
  [pricing](https://www.reviewable.io/pricing/).
- Aviator: [FlexReview onboarding](https://docs.aviator.co/flexreview/getting-started),
  [read-only mode](https://docs.aviator.co/flexreview/concepts/read-only-mode), and
  [pricing](https://www.aviator.co/pricing/).
- CodeRabbit: [GitHub setup](https://docs.coderabbit.ai/platforms/github-com),
  [test-repository onboarding](https://docs.coderabbit.ai/guide/repository),
  [plans](https://docs.coderabbit.ai/management/plans), and
  [Marketplace listing](https://github.com/marketplace/coderabbitai).
