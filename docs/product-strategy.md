# Product strategy: Final-Head Review Governor, with verified continuity underneath

Evidence captured: **2026-09-06**. This is a falsifiable product thesis, not a market-size report or
a claim that the roadmap is already implemented. Prices, install counts, stars, and vendor claims
are point-in-time observations and must be refreshed before external use.

The source-by-source demand record, competitor counter-evidence, ICP, and falsification gates are
maintained in the [September 2026 market-evidence snapshot](market-evidence-2026-09.md).

## Decision in one sentence

**StrataDiff should become the Final-Head Review Governor in front of existing AI and policy
reviewers: do not pay to review an obsolete PR revision, and do not merge a final revision without
trusted review evidence bound to its immutable `(base SHA, head SHA)` input. Review Cache is the
incremental input compiler for open reviewers; Review Resume is the human explanation and recovery
surface. The durable moat is the deterministic evidence contract and real-history transition
corpus, not another reviewer model.**

This is deliberately not another AI reviewer. AI reviewers generate more judgments. StrataDiff
removes repeated work only when a narrower factual claim can be checked again by an independent
verifier. For a person, the value experience is `stratadiff resume <PR-URL>`. For an automated
reviewer, the target value experience is a preflight that receives a completed-review receipt and
emits one bounded immutable input route before the expensive job starts. A route never manufactures
or restores a code-host approval, and an open context closure can never reuse a model verdict.

The v0.3.0 release shipped the first verified native binary and PR-URL flow, but release publication
alone did not close the measured **clean-machine PR-URL activation** milestone. That proof remains
the release-quality gate for the human surface. New public evidence of per-push AI review spend,
rate limits, duplicate comments, and teams disabling re-review makes the automation gateway the
higher-frequency commercial wedge. It must share the same transition proof rather than branch into
a heuristic cache product. On a machine with Git, an authenticated `gh`, and a verified StrataDiff
binary, the human value flow starts outside any checkout:

```text
stratadiff resume https://github.com/OWNER/REPO/pull/123
```

The command must derive the host and repository, resolve the authenticated reviewer, materialize
the named commits in an isolated temporary bare repository, and open the Workbench without `-R`, a
commit SHA, repository administration, a GitHub App, or workflow YAML. An exact-SHA fetch can still
transfer the commit's reachable object closure. The unreleased development path now limits each
fetch-created file to at most 256 MiB, the complete scratch tree to 512 MiB, each isolated object
store to 1,000,000 objects, and remote Git work to two minutes. These bounds do not meter transport
bytes, and they are not release evidence until a verified artifact passes the fresh-environment
test. Source-level URL inference and repository-local tests do not by themselves prove that
activation contract. The automation equivalent is one
preflight step that wraps the team's existing reviewer: it delays unstable intermediate heads,
cancels superseded work, and emits an immutable selected payload for the latest eligible head.
Scheduling state and evidence routing remain separate; a deferred head is never presented as
reviewed or clean.

The user-visible outcome is not “a better diff.” It is: **keep final-head automated review enabled
without paying for every intermediate push, and make an unreviewed final input impossible to call
green.** GitHub remains the place for comments and approval. A person can open Resume when they
need to inspect the carried evidence, remaining residue, or changed base context. The product
succeeds only if live teams reduce reviewer invocations, spend, or queue time while final-input
coverage and finding recall do not regress.

### Review Churn Census v1 decision update

The frozen 500-PR census completed on 2026-09-05 with zero capture failures and all global gates
passing. It materially narrows this thesis:

| Observation | Result | Decision |
|---|---:|---|
| Comparable completed reviewer checkpoints differing from final head | 88/488 (18.03%; Wilson 95% 14.87–21.69%) | Checkpoint drift is observable, but it is an event-driven job rather than a daily workflow for every PR. |
| Fully comparable reviewed PRs stranding at least one reviewer | 74/401 (18.45%; 14.96–22.55%) | A repository audit can identify concrete affected PRs and eligible reviewers. |
| Completed pairs with an observed later force-push | 43/490 (8.78%; 6.58–11.62%) | The precommitted 10% acquisition signal is inconclusive; do not claim a universal force-push epidemic. |
| Drift without an observed force-push after the latest checkpoint | 59/488 (12.09%; 9.48–15.29%) | The precommitted 20% broad-continuity signal fails; do not broaden the headline to every ordinary push. |
| Later same-reviewer `COMMENTED` session on a different commit | 7/488 (1.43%; 0.69–2.94%) | The 15% candidate signal fails; do not infer completion or build implicit checkpoint carry from comments. |

The complete result is in
[`benchmarks/review-churn-census-v1/`](../benchmarks/review-churn-census-v1/). It does not measure
time saved, issue recall, willingness to pay, or GitHub-wide prevalence.

The census narrows the human Resume opportunity, but later market evidence changes the primary
go-to-market sequence. Re-review bots already run on push, so the first acquisition surface is a
**Review Governor** around an existing reviewer: debounce moving heads, cancel obsolete work, and
hold one required status until the current `(base, head)` input has trusted completion evidence.
Review Memory Audit, Inbox, and Resume remain useful qualification, explanation, and recovery
surfaces; they are no longer expected to create the high-frequency habit on their own. The product
must not post routine marketing comments or appear on repositories where it cannot measure avoided
work.

The early validation surface remains the local `gh` extension because it needs no organization
approval and keeps source local. If the human experiment passes, the distribution surface should
be a public GitHub App with a native Check and a single **Resume review** action. An App provides
zero-YAML installation, durable webhook state, and a direct Check surface. An Action can also create
a Check when its installation token has `checks: write`, but fork-token and approval restrictions
make it a weaker default onboarding path. The CLI, Action, schemas, and verifier remain the open
self-hosted and trust path. This separates “prove the job is valuable” from “remove installation
friction once it is worth scaling.”

### Product boundary: Check answers, Resume acts, Audit and Inbox route

Rebase-safe approval is a real pain but too small a standalone category. Graphite's own
[`dismiss-stale-approvals`](https://github.com/withgraphite/dismiss-stale-approvals) README says the
request came from a relatively small number of customers and one enterprise trial. GitHub already
offers the whole-PR compromise “require approval of the most recent reviewable push,” while GitLab
can reset approvals only when a [`git patch-id`](https://git-scm.com/docs/git-patch-id) changes.
Competing on that binary switch alone would make StrataDiff a feature, not a product.

The qualification surface is a **Review Memory Audit** that requires only existing GitHub read access and no
checkout. It reports a bounded repository window, distinguishes missing evidence from a clean
result, and identifies affected PRs without collecting source, diffs, PR/review text, patches, or
commit messages. Its purpose is diagnosis and qualification, not a population estimate.

The personal routing surface is a **Review Inbox**. It scans open PR metadata for the
authenticated user, selects only the latest non-dismissed `APPROVED` or `CHANGES_REQUESTED`
checkpoint, and emits a Resume action only from a complete, revalidated current-base observation
with a changed head or an exact active re-review request. Because GitHub does not expose the
historical review-time base, a stable head without that base remains unobservable instead of being
declared clean. Later comments never become implicit completion; incomplete pagination, missing
IDs, or identity drift remain explicit failures or unknowns. Each action binds repository, PR,
reviewer, checkpoint, base, head, and request state into an unsigned, content-addressed event
envelope. Resume checks that envelope against repeated live provider observations before opening
the Workbench; the envelope is not a signature or standalone proof of authenticity. GitHub does not
provide an atomic repository-wide snapshot, so Inbox records a bounded advisory observation window
and collection has global resource budgets.

The current value surface is a **personal Review Resume** that requires no repository administrator
and does not replace GitHub's review UI. It resolves the reviewer's checkpoint and opens a local
workbench over the exact residue, using an existing checkout or isolated temporary repository. Git
may transfer the reachable object closure for each requested commit; the development path isolates
and cleans that state, applies the file, scratch, object-count, and time bounds above, but still has
no strict wire-byte quota. The canonical PR URL must become the demonstrated default so the user
does not need to understand repository selection or commit identity. This is the shortest path from
diagnosed pain to experienced value and avoids asking a team to trust an App before the reviewer has
saved any time.

The native destination is a **Verified Review Delta Check**, followed by an optional
**review-coverage firewall**. It maintains a SHA-bound ledger for each
required reviewer and CODEOWNERS domain, maps that coverage across ordinary pushes and history
rewrites, and rejects every file whose carry cannot be proved. GitHub remains the canonical place
for conversation and approval. The long-term promise is:

> Never re-review code we can prove unchanged. Never inherit an unproven approval.

The alpha now implements the repository Audit, personal Inbox, and underlying file-level vertical
slice: the existing single-reviewer Action, HMAC-authenticated webhook ingestion, an append-only
review ledger, exact-base CODEOWNERS resolution, user and team permission snapshots, reviewer ×
owner × file coverage, a signed Passport, offline recomputation, a local Passport viewer, and
deterministic Check Run request JSON. A separate
[runnable hosted GitHub App MVP](../services/governor-app/README.md) now receives signed webhooks,
binds the fixed `StrataDiff Final Head` Check Run to a dedicated App identity, and coordinates
PostgreSQL state, durable outbox work, and monotonic lease fencing. It creates a distinct gate for
each merge-group SHA but deliberately leaves it non-successful until merge-group-native provider
evidence exists; it never reuses a green PR-head gate.

That hosted implementation is not production validation. Its automated tests use an in-memory
PostgreSQL-compatible adapter and injected GitHub transport, while CI's real-PostgreSQL coverage is
limited to migration smoke testing. A live App/ruleset/strict-or-merge-queue/CodeRabbit end-to-end
run and a real-PostgreSQL concurrency and process-failure proof are still missing. Live permission
collection, partial-file review state, and evidence that the workflow saves human time also remain
open. These boundaries must remain visible in every launch claim.

The alpha gate now derives a separate `review-delta-v1` queue from five snapshots: old base `A`,
reviewed checkpoint `B`, current base `C`, current head `D`, and reconstructed reviewed baseline
`S`. When the reviewed patch and upstream patch commute byte-for-byte, the reviewer sees `S → D`,
not the noisier `C → D`. A dropped or reverted reviewed change remains in the queue even when
`C → D` is empty. Unsupported, interacting, binary, or unaddressable cases remain explicit
fallbacks or unresolved blockers. The repository-level `review-v1` report is still
producer-attested; the separate `review-coverage-v1` Passport is receiver-signed and independently
recomputed against exact offline Git objects.

### The human recovery job: resume, do not restart

The human product job is not generic semantic triage. It is the repeated-review loop: a reviewer
finishes a large PR snapshot, the author or coding agent pushes again, and the reviewer needs to
know which complete PR changes differ from the reviewed checkpoint. A caller-selected checkpoint
turns that question into a narrow comparison that does not require guessing intent or behavior.

A current file may be labeled `unchanged_since_checkpoint` through either of two proofs. The fast
path requires the same complete Git change identity: status, similarity, before and after paths and
encodings, modes, and object IDs. If the merge base changed, a unique same-path regular-file
modification may also carry through non-interacting four-way byte replay. The engine constructs the
reviewed and upstream patches from the old base, rejects touching or overlapping edits, translates
the patches in both directions, and requires both replay orders to produce the current blob exactly.
All conflicts, ambiguous candidates, unsupported file kinds, missing evidence, and replay failures
remain `needs_review_now`. This proves a narrow byte relation between four file snapshots. It does
not prove that a human reviewed the checkpoint, that cross-file effects are absent, or that the
change is safe to merge.

The report names the base-drift policy
`exact_git_change_identity_or_noninteracting_four_way_byte_replay`. Each carried file records its
actual `checkpoint_match_basis` as `exact_git_change_identity` or
`exact_noninteracting_four_way_byte_replay`; a needs-review file has no carry basis. This distinction
must survive into the Change Passport and any host check.

Native code hosts, Graphite, and Reviewable already support review state and comparisons across
pushes, so "show changes since last time" is not a novel category. StrataDiff's testable boundary is
an open, host-neutral review-memory gate: every carry needs a named deterministic proof, every
unproved change stays visible, and the evidence can be downloaded and checked independently. If
users do not value that portability or stronger claim boundary, this wedge is not differentiated.

## The product primitives

### Review Residue

Review Residue is the portion of a change that remains in the human-first lane after StrataDiff has
identified evidence-backed transformations. It is a prioritization surface, not permission to skip
review.

The initial lanes are:

1. **Review first:** changed behavior, additions, deletions, unsupported structure, ambiguity, and
   anything the engine cannot establish.
2. **Evidence-backed secondary review:** exact content relocation or syntax-preserved changes under
   a named parser and model. Path, build, configuration, and repository-level effects still require
   judgment.
3. **Unverified:** unsupported, malformed, oversized, binary, or failed analysis. Unverified always
   counts toward the human-first total; it is never silently dropped.

The product promise is a smaller *ordered attention queue*, not a smaller legal or engineering
responsibility.

The current alpha deliberately keeps all evidence classes in the first-pass queue. CST equality
alone is not a sound priority rule: source trivia is observable in examples such as Rust
`stringify!`, Python debug f-strings, C preprocessing, and HTML inline rendering. Evidence and
priority therefore remain separate fields; moving a class to secondary review requires a
context-specific policy, adversarial fixtures, and reviewer-recall evidence.

### Change Passport

The current `review-v1` artifact is a producer-attested focus summary, not a self-contained or
replay-verifiable Change Passport. It retains commit/blob provenance and digests of single-file
reports that were checked during production, but does not carry those reports. The term “Change
Passport” below names the target artifact contract, which requires a sidecar evidence bundle and an
offline verifier.

A Change Passport is a deterministic, portable artifact bound to an exact Git comparison. At
minimum it records:

- requested base, merge base, head commit, blob IDs, paths, file modes, and engine/schema versions;
- exact byte replay status and the hash of each supporting report;
- per-file lane, the predicate actually checked, and the reason for the classification;
- ambiguities, abstentions, unsupported files, limits reached, and explicit non-claims;
- enough data or references for an independent verifier to reproduce the factual claims offline.

The passport should be useful as a CLI artifact, CI check, release attachment, or input to another
review product. GitHub is the first distribution surface, not the owner of the data model.

## Why this problem is worth testing now

The evidence supports a real review-bandwidth problem, particularly as agents create more code. It
does **not** yet prove that Review Residue is the winning solution.

| Signal | Observation at capture | What it supports | Evidence quality |
|---|---|---|---|
| AI output is not trusted by default | In the [2025 Stack Overflow AI survey](https://survey.stackoverflow.co/2025/ai/), 46% of respondents distrusted AI accuracy versus 33% who trusted it; 66% cited “almost right” output as a top frustration, and 45% said debugging AI-generated code was more time-consuming. | A product that asks reviewers to trust another probabilistic verdict is poorly aligned with the stated pain. | Survey result. Percentages describe respondents, not all developers. |
| Review judgment is becoming the bottleneck | GitHub [reported](https://github.blog/ai-and-ml/generative-ai/agent-pull-requests-are-everywhere-heres-how-to-review-them/) that more than one in five reviews involved an agent and described review bandwidth as saturated. | More generated change increases the value of defensible attention triage. | Platform-owner report; methodology and denominator remain GitHub's. |
| Large agent changes are hard to consume | A GitHub engineering [case study](https://github.blog/engineering/turn-one-giant-ai-generated-pull-request-to-a-reviewable-stack/) turns a 1,721-line agent-generated PR into a reviewable stack. | Teams already reshape change to recover reviewability. | One case study, not prevalence evidence. |
| Maintainers report low-signal AI contributions | GitHub Community discussions document [low-quality AI-generated contributions](https://github.com/orgs/community/discussions/185387) and requests for [ways to filter them](https://github.com/orgs/community/discussions/159749). | Reviewers want control over attention and provenance. | Community anecdotes; useful for discovery, not incidence estimates. |
| Structural presentation can still mislead | A public issue reports [moved-code false positives](https://github.com/fullsend-ai/fullsend/issues/2019). | A polished move visualization is insufficient without evidence and abstention. | One issue report; it establishes possibility, not frequency. |
| AI review accuracy is unsettled | [Code Review Bench](https://github.com/withmartian/code-review-benchmark) publishes a 50-PR offline set with 173 human-curated comments plus an online LLM-judged pipeline. In the captured leaderboard/configuration, the best F1 was about 0.578, with GitHub Copilot around 0.451 and CodeRabbit around 0.406. | AI findings are complementary, but should not be treated as proof or a complete review gate. | Small benchmark; scores depend on category profile, F-beta, tool version, and judge. Not a universal ranking. |
| Developers adopt better diff experiences | [Difftastic](https://github.com/Wilfred/difftastic) had 25,855 GitHub stars, and the [SemanticDiff VS Code extension](https://marketplace.visualstudio.com/items?itemName=semanticdiff.semanticdiff) displayed 49,020 installs. | There is demonstrated interest in code-aware diffing. | Public counters; neither equals active teams, revenue, or willingness to pay. |
| Commercial review tooling has paid demand | At the 2026-09-06 capture, official plan pages listed Reviewable Team/Business at $8/$16 per contributor per month, Graphite Starter/Team at $20/$40 per seat per month billed annually, and CodeRabbit Essentials/Team at $24/$48 per developer per month billed annually; CodeRabbit Advanced was $90 monthly ([Reviewable](https://www.reviewable.io/pricing/), [Graphite](https://graphite.com/docs/billing-plans), [CodeRabbit](https://docs.coderabbit.ai/management/plans)). | Teams pay for review workflow and automation; pricing is plausible only if reviewer value is proved. | Vendor list prices are point-in-time observations, exclude discounts and usage add-ons, and do not establish customers, retention, or willingness to pay for StrataDiff. |
| GitHub Apps provide a large native review-tool distribution surface | The GitHub Marketplace pages displayed 317,789 installs for [CodeRabbit](https://github.com/marketplace/coderabbitai) and 74,392 for [Renovate](https://github.com/marketplace/renovate) at capture; Renovate pairs its hosted App with an open-source self-hosted engine. | If the reviewer experiment passes, a public App can remove YAML and local-install friction while the open CLI remains the trust path. | Marketplace counters are point-in-time acquisition proxies, not active users, retention, or revenue. |
| History rewrites destroy useful review context | GitHub's own `gh-stack` users report that sync force-pushes [erase “changes since last view”](https://github.com/github/gh-stack/issues/354), and that a byte-identical restack [dismissed three approvals and restarted CI](https://github.com/github/gh-stack/issues/446). | Exact state can survive rewritten commit identity and avoid demonstrably redundant work. | Concrete first-party issue reports; they establish failure modes, not prevalence. |
| Reviewers explicitly ask to resume after repeated force-pushes | GitHub Community [#3478](https://github.com/orgs/community/discussions/3478) had 305 upvotes at capture; a 2024 commenter said that after several force-pushes there was no way to see the cumulative change since their review and they had to review the whole PR again. | The strongest acquisition job is a reviewer-controlled resume command, not organization policy configuration. | Public demand signal and detailed anecdotes; not measured usage or willingness to pay. |
| Stacked PR churn turns one rewrite into repeated human work | GitHub `gh-stack` [#323](https://github.com/github/gh-stack/issues/323) had 74 reactions and 25 comments at capture; one six-layer stack report says a sync dismissed three approvals, while another reports CI waits measured in hours. | Stack-heavy repositories are the best first segment for a no-admin Review Resume experiment. | Concrete reports from self-selected users; magnitude is not population prevalence. |
| Reviewers cannot recover the right incremental range after a rebase | A GitHub Community request says the heavily used “changes since last review” view stops working after rebase and force-push, leaving reviewers to find the first unreviewed commit and edit a URL or use the CLI ([#141845](https://github.com/orgs/community/discussions/141845), 28 votes at capture). GitLab users separately request a reviewer-specific last-reviewed revision instead of choosing versions from memory ([#25559](https://gitlab.com/gitlab-org/gitlab/-/work_items/25559)). | The default experience must resolve a per-reviewer checkpoint automatically and open the residue in one action; asking for a SHA is a diagnostic fallback, not the product. | Two public requests across hosts; neither establishes incidence or willingness to pay. |
| GitLab users report rewrite-specific context loss | GitLab work items report useful comparison context disappearing after force-push ([#241509](https://gitlab.com/gitlab-org/gitlab/-/work_items/241509)) and target-branch noise entering post-rebase comparison, with `range-diff` proposed as an alternative ([#442454](https://gitlab.com/gitlab-org/gitlab/-/work_items/442454)). | Force-push retention and base-drift separation are concrete cross-host requirements, not GitHub-only wording. | The first report was open and the second closed at the 2026-09-06 capture; they establish failure modes, not frequency or roadmap commitment. |
| Approval invalidation is broader than the reviewed delta | GitHub Community requests ask for invalidation by the final diff or tree rather than commit ancestry ([#12876](https://github.com/orgs/community/discussions/12876), 98 votes at capture) and report stacked changes causing cascades of stale approvals ([#57513](https://github.com/orgs/community/discussions/57513), 126 votes at capture). Another report says a reviewer's own suggestion can trigger renewed approval across 12 organizations ([#78039](https://github.com/orgs/community/discussions/78039), 18 votes at capture). | The product should bind review state to exact evidence and invalidate only what it cannot carry. | Public requests and reported organization experience; vote counts are point-in-time signals, not prevalence. |
| Large-MR reviewers explicitly ask for narrow invalidation | A GitLab request says rebase forces the reviewer to revisit every approved file and asks to retain identical file/block approval ([#594565](https://gitlab.com/gitlab-org/gitlab/-/issues/594565)). A separate GitLab analysis reports a 15% incidence of unwanted patch-ID changes in one 1,000+-developer, 50k-file project ([#439234](https://gitlab.com/gitlab-org/gitlab/-/issues/439234)). | Whole-review invalidation is a costly, measurable problem; exact file identity is a plausible narrower primitive. | One user request and one organization-specific analysis; external replication is required. |
| Whole-PR invalidation ignores ownership boundaries | A GitLab request reports that a new commit can invalidate every approval even when only one CODEOWNERS domain changed, forcing unrelated domain owners to review again ([#604779](https://gitlab.com/gitlab-org/gitlab/-/work_items/604779)). | Coverage must eventually be tracked per reviewer and ownership domain; a single global checkpoint is only an alpha integration. | One public feature request; it establishes the workflow failure, not its frequency. |
| Force-push approval churn creates mechanical re-review | Zephyr's [stale-approval RFC](https://github.com/zephyrproject-rtos/zephyr/issues/43701) reports that re-approval after a force-push delayed merges while reviewers often responded with a mechanical `+1`; its public [PR #41626](https://github.com/zephyrproject-rtos/zephyr/pull/41626) contains 27 force-push events and 13 approval dismissals at capture. A later [review-workflow RFC](https://github.com/zephyrproject-rtos/zephyr/issues/53566) explicitly contrasts GitHub's large-PR experience with Gerrit. | Large, multi-reviewer OSS projects provide concrete design-partner cases and replayable event histories. | Purposefully selected public evidence; it proves the workflow can be painful, not population prevalence. |
| Host history is not a durable review ledger | On 2026-09-05, both GitHub GraphQL and REST returned `null` for the commit bound to all 13 dismissed review records still visible on Zephyr [PR #41626](https://github.com/zephyrproject-rtos/zephyr/pull/41626); the two final, non-dismissed approvals still exposed their commit. | A post-hoc Action cannot always reconstruct old coverage. The GitHub App must capture a signed or content-addressed review receipt when the review event arrives and retain dismissal as a later state transition. | Point-in-time API observation on one old PR; retention behavior may vary and is not a platform guarantee. |
| GitLab treats selective approval reset as a paid workflow primitive | GitLab documents an option to remove approvals only when a new commit changes the patch ID and a separate option to remove only approvals from Code Owners whose files changed ([approval settings](https://docs.gitlab.com/user/project/merge_requests/approvals/settings/)). | Patch-aware and owner-scoped invalidation are established buyer-facing capabilities; StrataDiff cannot treat policy configuration itself as novel. | Official product behavior. Git patch-id is only reasonably stable and ignores whitespace by default, so it is not equivalent to StrataDiff's evidence contract. |
| Aviator already sells per-approver selective revalidation | [FlexReview validation](https://docs.aviator.co/flexreview/concepts/validation-in-flexreview) compares each approver's last-approved commit over owned files, preserves no-code rebase approvals, selectively invalidates changed ownership scopes, and can publish a required status check. | `Coverage Firewall` and selective CODEOWNER invalidation are not unique claims. StrataDiff must lead with exact reviewer residue, local execution, and reviewer-specific reconstruction of dropped residue backed by downloadable verification. | Official capability documentation; no independent accuracy, adoption, or offline-evidence comparison was found. |
| Reviewable proves demand for persistent per-file review state | Reviewable records each reviewer's state per file and revision and carries state across rebases where it can map revisions ([file review state](https://docs.reviewable.io/files)). Its maintainer also warns that carrying line-level marks can hide unreviewed changes ([issue #414](https://github.com/Reviewable/Reviewable/issues/414#issuecomment-1611899307)). | Persistent human review memory is a real product, while conservative hunk carry remains technically differentiated and safety-sensitive. | Official documentation and maintainer statement; adoption and time-saving are not independently measured here. |
| Ownership policy has stand-alone paid demand | PullApprove sells path- and line-based approval rules and publishes a DoorDash account of use across hundreds of repositories and thousands of users ([product documentation](https://www.pullapprove.com/docs/), [pricing](https://www.pullapprove.com/pricing/)). | The buyer is DevEx/platform/security, and ownership-aware coverage can be a paid control-plane feature. | Vendor claims and pricing, not independently audited usage. |
| Native ownership enforcement is closing the policy gap | GitHub announced [required review from specific teams](https://github.com/orgs/community/discussions/178776) as generally available in February 2026, with path, team, and approval-count rules. | A CODEOWNERS matrix is not a sufficient wedge. StrataDiff must win on cross-rewrite review recovery and portable evidence, then integrate with native enforcement. | Official GitHub announcement; exact plan availability and behavior must be refreshed before launch. |
| AI re-review also forgets dispositions | GitHub's [Copilot code-review documentation](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/request-a-code-review/use-code-review) says re-review may repeat dismissed or downvoted comments, while a [community request](https://github.com/orgs/community/discussions/190754), with 27 votes at capture, documents the same incorrect suggestion recurring over three or four review rounds. | A later ledger should bind findings and human dispositions to exact evidence identity instead of rerunning a stateless reviewer. | Official limitation plus one detailed user report; this is a follow-on job, not proof that the current checkpoint implementation solves it. |
| Review state across pushes is an established job | [GitHub documents](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/reviewing-proposed-changes-in-a-pull-request#marking-a-file-as-viewed) that a viewed file is unmarked when it changes, while [GitLab exposes diff versions](https://docs.gitlab.com/user/project/merge_requests/versions/) specifically for merge requests with many or sequential changes. A [public editor request](https://github.com/wandersoncferreira/code-review/issues/146) describes large PR review spanning multiple sessions and asks to preserve file or hunk state. | Reviewers already expect incremental review and explicit invalidation. | Product documentation plus a user report; neither measures time saved. |
| Native viewed state already preserves some progress | GitHub's documentation says a file is unmarked only if that file changes, and a VS Code extension maintainer confirms that unchanged files should remain viewed ([#8755](https://github.com/microsoft/vscode-pull-request-github/issues/8755)). | Never market StrataDiff as fixing a total reset on every push. Its narrower job is rewrite-safe, reviewer-specific residue below whole-file granularity, with explicit evidence. | Official behavior plus maintainer confirmation; host behavior can still vary across rewrite and base-drift cases. |
| A dropped reviewed change can disappear from the incremental view | A Files Changed feedback report says a file added and reviewed, then removed from the PR, could not be rendered in “changes since your last review” ([GitHub Community #163932](https://github.com/orgs/community/discussions/163932#discussioncomment-13622332)). | Retired reviewed work must remain an explicit queue item even when it is absent from the current PR range. | One concrete user report; it establishes a failure mode, not frequency. |
| Native stacked PRs increase both opportunity and platform risk | GitHub's [Stacked PR public preview](https://github.com/orgs/community/discussions/201439) quotes reviewers struggling with growing agent PRs; users report losing track of reviewed work and having to rebase, re-review, and rerun CI after stack failures. | Stacks are a strong design-partner segment, but StrataDiff must complement the host rather than become another stack manager. | Official preview plus self-selected feedback; GitHub may absorb more continuity features. |
| Naive last-review diffs absorb base noise | VS Code's GitHub extension has tracked incremental review since [#363](https://github.com/microsoft/vscode-pull-request-github/issues/363). Follow-up reports show merges from the base branch introducing unrelated files into the view ([#4510](https://github.com/microsoft/vscode-pull-request-github/issues/4510), [#5455](https://github.com/microsoft/vscode-pull-request-github/issues/5455), [#6281](https://github.com/microsoft/vscode-pull-request-github/issues/6281)). | A useful residue must compare PR-relative changes and exclude upstream-only files. | Public issue reports establish concrete failure modes, not their frequency. |
| Rebase-aware review has capable incumbents | [Reviewable documents](https://docs.reviewable.io/files#file-review-state) matching a file against a prior rebased revision, and Git provides [`range-diff`](https://git-scm.com/docs/git-range-diff) for comparing two versions of a patch series. | Exact Review Resume must compete on portable evidence and deterministic invalidation, not claim invention of incremental review. | Capability documentation, not comparative accuracy or adoption evidence. |
| Standalone re-review UI is increasingly crowded | [Pyor](https://pyor.review/) markets agent-era PR grouping and its [interdiff guidance](https://pyor.review/blog/re-reviewing-pull-requests-interdiff) directly addresses re-review after force-push and rebase. | Another viewer is not a sufficient wedge. The first-run command must recover the checkpoint automatically and prove useful before a team installs policy infrastructure. | Vendor capability and positioning; no independent adoption or performance evidence was found. |
| Installation friction can kill a useful review tool | In the [Crocodile launch discussion](https://news.ycombinator.com/item?id=31841215), users called out the need for administrator approval and asked for proof of roughly an hour saved per engineer per week; [Crocodile later shut down](https://www.crocodile.dev/). | The first-run path must work through the user's existing `gh` authentication, and the pilot must measure saved time rather than ask teams to trust a feature list. | Anecdotal launch feedback and one discontinued product; useful as a constraint, not a causal postmortem. |
| Stacked workflows trade safety for flow | Graphite's [stack-review guidance](https://graphite.com/docs/best-practices-for-reviewing-stacks) recommends disabling both stale-approval dismissal and latest-push approval requirements for smoother stacks. | A deterministic residue check can occupy the gap between repeatedly clearing all review state and trusting every rewritten stack. | Official workflow advice; it does not prove demand for StrataDiff. |
| Mature review systems already copy votes under explicit change rules | Gerrit's [label copy conditions](https://gerrit-review.googlesource.com/Documentation/config-labels.html#label_copyCondition) distinguish `NO_CHANGE`, `NO_CODE_CHANGE`, `TRIVIAL_REBASE`, and `REWORK`, and describe copied votes as reducing turnaround time. | Evidence-based coverage carry is an established policy primitive; StrataDiff should make its narrower byte-level rule portable and independently checkable. | Official capability documentation; it does not show that Gerrit users need a separate product. |

The strongest market inference is therefore limited: review attention is scarce, existing products
monetize review workflow, and probabilistic review has a trust gap. We have not yet proved that
teams will adopt a proof-carrying residue layer or that it saves time.

## Ideal customer profile and jobs to be done

### Primary user and paid ICP hypotheses

The acquisition user is an individual reviewer or open-source maintainer who already uses GitHub
CLI and repeatedly reviews rebased, restacked, amended, or agent-updated PRs. They must be able to
reach value without repository administration.

The paid design partners should be GitHub Cloud or GHES teams, typically 30–1,000+ engineers, that
have several of these characteristics:

- frequent large or noisy PRs caused by formatting, file moves, code generation, migrations,
  dependency updates, broad refactors, or coding agents;
- multiple required reviewers or CODEOWNERS whose scarce resource is attention rather than access
  to another summary;
- stacked PRs, rebases, linear-history enforcement, or frequent amended/agent-generated pushes;
- Git-based CI and a willingness to retain a machine-readable review artifact;
- a DevEx, platform, staff-engineering, or security champion who values auditable local execution;
- enough risk that “the model says this is safe” is not an acceptable control.

Likely buyers are DevEx, platform, and security-engineering leads; daily users are staff engineers,
maintainers, and code owners. Early users should include active open-source maintainers and teams
with monorepos or agent-heavy workflows. This is a targeting hypothesis, not a claim about segment
size.

### Anti-ICP

Do not optimize the first product for tiny PRs, solo developers with no review bottleneck, teams
seeking an autonomous merge bot, or buyers whose primary need is bug discovery. Those users are
served better by native code-host review, AI review, or static analysis.

### Core JTBD

> When a PR I already reviewed is rebased, restacked, amended, or force-pushed, recover my last
> usable checkpoint and show only what still needs my attention, without asking me to reconstruct a
> commit range or trust an automatic approval.

Supporting jobs are:

- **Reviewer:** resume after a new push without rereading unchanged evidence.
- **Author:** explain a mechanical or generated change without asking for blind trust.
- **Platform owner:** enforce that unverified and ambiguous changes remain visible.
- **Auditor:** retain a commit-bound record of what was checked, by which engine and policy.
- **Tool builder:** consume a stable evidence format without adopting StrataDiff's UI.

## Competitive boundary

The category is occupied. The opportunity is a narrow combination, not a claim that nobody has
worked on semantic diffing, refactoring analysis, or review workflow.

| Category | What it already does well | Boundary for StrataDiff |
|---|---|---|
| GitHub and GitLab | Canonical conversation, permissions, approvals, viewed state, file navigation, and revision workflow. GitHub can dismiss approvals when its recorded diff changes or require approval of the latest reviewable push. GitLab keeps per-user Viewed files hidden until their content changes, stores one diff version per push, and uses `git patch-id` for smarter approval reset across rebase or target merges ([versions](https://docs.gitlab.com/user/project/merge_requests/versions/), [Viewed](https://docs.gitlab.com/user/project/merge_requests/changes/#mark-files-as-viewed), [approval reset](https://docs.gitlab.com/user/project/merge_requests/approvals/settings/#remove-all-approvals-when-commits-are-added-to-the-source-branch)). | Do not claim native hosts lack incremental review or approval gates. Recover an exact reviewer checkpoint across rewrite cases the host cannot explain, bind any dropped residue to that checkpoint, and make each carry independently inspectable. `patch-id` is a reasonably stable whole-diff identity, not a reviewer × change byte certificate. |
| [Reviewable](https://docs.reviewable.io/files) | Tracks each reviewer × file × immutable revision, exposes last-reviewed-to-latest comparisons, pins force-pushed commits, collapses base-only changes, and heuristically matches rebased commits. | Do not claim invention of persistent per-file review memory. Differentiate on a GitHub-native/no-migration entry, deterministic four-snapshot evidence, explicit fail-closed states, no additional third-party OAuth grant for the local path, local source analysis, and offline verification. |
| [Aviator FlexReview](https://docs.aviator.co/flexreview/concepts/validation-in-flexreview), GitLab, and Gerrit | Selectively retain or invalidate approvals after no-code rebases and file changes. Gerrit additionally compares arbitrary patch sets, separates mapped rebase edits, stores private reviewed flags by patch set × file × user, and copies votes under administrator-defined change-kind conditions ([review UI](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#normal-and-rebase-edits), [reviewed flags](https://gerrit-review.googlesource.com/Documentation/config-accounts.html#reviewed-flags), [copy conditions](https://gerrit-review.googlesource.com/Documentation/config-labels.html#label_copyCondition)). Its own documentation shows a [hazardous stacked squash](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#hazardous-rebases) whose patch-set interdiff is empty while parent content enters implicitly. | `Coverage Firewall`, selective invalidation, and rebase coloring are expansion capabilities, not primary novelty. The remaining combination is GitHub-native exact residue reconstruction, reviewer-specific dropped-residue and parent-influx evidence, strict replay, portable verification, and a no-migration local entry. |
| Graphite and stacked-PR tools | Make changes reviewable by splitting, restacking, navigating, and versioning PRs. Graphite's [PR versions](https://graphite.com/docs/pull-request-versions) can hide reviewed changes by comparing a user's last-reviewed version with the latest. | Do not compete on stack navigation or generic version interdiff. Analyze an arbitrary existing PR and prove which human checkpoint coverage survives a rewrite. The approaches are complementary. |
| Copilot, CodeRabbit, Graphite AI, and other AI reviewers | Suggest likely bugs, summaries, and fixes. CodeRabbit documents incremental analysis of commits added since its previous review; Copilot can re-review every push when enabled, but may repeat resolved or downvoted comments ([CodeRabbit](https://docs.coderabbit.ai/configuration/auto-review), [Copilot](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/request-a-code-review/use-code-review)). | Run these tools on the residue if useful. A newly generated model judgment is not evidence that a named human review remains valid, so it cannot enter the carried-coverage lane without deterministic support. |
| SemanticDiff, Difftastic, and Pyor | Provide substantially better structural presentation, moved-code navigation, grouping, or re-review views than line diff. | The overlap is real. Differentiate first on zero-admin checkpoint recovery across force-pushes, then on portable evidence, independent replay, and explicit abstention—not on visual syntax awareness alone. |
| RefactoringMiner / ASTDiff | Strong Java refactoring detection and mapping; its [PurityChecker](https://github.com/tsantalis/RefactoringMiner/blob/master/documentation/purity.md) evaluates nine documented refactoring kinds. | Reuse or ingest stronger language-specific evidence. Do not claim that the current multi-language CST matcher supersedes compiler-aware Java analysis. |
| Moderne / OpenRewrite | Deterministic source recipes with [recipe tests](https://docs.openrewrite.org/authoring-recipes/recipe-testing) and knowledge of the transformation that was requested. | For recipe-produced changes, producer provenance can be stronger than post-hoc inference. Import the recipe attestation; focus StrataDiff on vendor-neutral verification of changes from any source. |
| Static analysis and security scanners | Find known bug and vulnerability classes. | These tools answer “what may be wrong?” StrataDiff answers “what factual transformation can be replayed or checked?” Neither replaces the other. |

The defensible product loop is: **one-command checkpoint recovery → conservative residue → repeated
review use → portable passport → optional team ledger**. Any competitor can copy a four-lane
summary; distribution, strict failure behavior, the verifier, benchmark, artifact compatibility,
and accumulated lineage must provide the trust advantage.

## Claims we will and will not make

Allowed language must name the checked predicate:

- “The serialized patch replayed to the exact target bytes.”
- “These blob bytes are identical at a different path.”
- “This structure is preserved under parser X, grammar version Y, and model Z.”
- “The engine abstained; this file remains review first.”

StrataDiff must **not** claim:

- 100% correctness, historical author intent, or a unique canonical mapping;
- semantic or behavioral equivalence from byte, syntax, shape, rename, or relocation evidence;
- that a content-preserved file move is harmless to imports, build rules, ownership, or deployment;
- “safe to merge,” “review not required,” approval, security, or absence of bugs;
- complete support for every language, generated file, binary, submodule, or repository size;
- superiority to published tools from results measured on a different corpus or scoring universe;
- customer time savings, market size, or production reliability before those are measured.

The existing [DiffBenchmark result](benchmarks.md#diffbenchmark-literature-subset-result) is engine
evidence, not product validation. Its high precision and lower recall apply only to the declared
Java subset and scorable adapter universe; they do not establish safe PR triage or reviewer value.

## North-star outcome and guardrails

The north-star outcome is **median reviewer minutes saved per eligible PR without lower issue
recall** in a counterbalanced reviewer study. “Lines hidden” is not the goal and must never be the
headline metric.

Before that outcome can be measured honestly, the activation gate is **successful time to first
residue from a canonical PR URL on a fresh environment**. Report the denominator, platform,
installation method, authentication prerequisite, p50/p95 time, bytes transferred, and classified
failures. Do not count a source checkout, source build, preselected repository, or manually supplied
checkpoint as a successful clean-machine activation.

The online operating proxy is **accepted verified-secondary share**:

```text
accepted verified-secondary share =
  changed lines assigned to an evidence-backed secondary lane and not overridden
  / all changed lines in PRs with complete accounting
```

Always publish it with:

- eligible-PR rate and unverified/abstention rate;
- unsupported-downgrade and factual-claim error rate;
- seeded and naturally occurring issue recall by lane and severity;
- reviewer override rate and reason;
- analysis p50/p95 latency, peak memory, and failed/incomplete run rate;
- passport verification success across engine versions;
- reviewer time, confidence, and change-size strata.

No percentage target should be marketed until a baseline is reproduced. A product that reduces the
displayed diff while lowering defect recall is a failure even if adoption looks good.

## Evaluation program

Before building a broad SaaS surface, create a versioned Review Residue benchmark with three tracks:

1. **Controlled transformations:** exact rename/copy, file moves, formatting, line-ending changes,
   generated changes, rename-plus-edit, and known semantic edits across supported languages. The
   transformation oracle is generated and retained.
2. **Pinned real PRs:** permissively usable public commits stratified by ordinary fixes, migrations,
   mechanical refactors, formatting, generated code, dependency changes, and agent-authored work
   where provenance is explicitly disclosed. Preserve commit IDs, license/provenance, selection
   rules, and exclusions.
3. **Adversarial corpus:** path-sensitive behavior, Python indentation, macros and preprocessors,
   overloads and shadowing, duplicate blocks, mixed encodings, symlinks, submodules, binaries,
   malformed syntax, oversized input, rename-plus-edit, copy-plus-edit, and parser-version drift.

Ground truth has two separate layers: factual transformation labels and reviewer-priority labels.
Do not infer one from the other. Two independent reviewers adjudicate priority disagreements; the
machine-verifiable oracle adjudicates replay and predicate claims.

Run a counterbalanced study comparing the native code-host diff with the same diff plus StrataDiff.
Measure time to decision and issue recall; rotate condition order to limit learning effects. Publish
case-level outputs, tool versions, failures, exclusions, and confidence intervals. Compare against
at least raw Git diff, whitespace-ignored diff, and one structural viewer. AI-review benchmarks may
be reused for bug-finding context, but are not a substitute for this attention-triage evaluation.

The host-workflow acceptance matrix must include these end-to-end cases:

- rebase plus one genuine author edit: every genuine edit remains in the residue and upstream-only
  files shown to the reviewer remain zero;
- byte-identical restack or force-push: every supported current change carries, with its exact
  evidence basis recorded, without asking the reviewer to locate a commit SHA;
- adjacent target-branch edits that perturb ordinary patch context: false invalidation remains zero
  whenever strict four-way replay proves non-interaction;
- stacked parent rewrite or squash: an empty author residue must still expose the exact
  old-base-to-current-base context instead of presenting the session as an unchanged tree;
- two independent CODEOWNERS domains: changing one domain invalidates that domain's coverage while
  leaving the other domain's proven coverage intact;
- unavailable, malformed, ambiguous, or provider-unverifiable checkpoints: the check fails closed
  with an actionable diagnostic and never substitutes another revision.

## Roadmap

### P0: prove the wedge

1. Close the Governor safety contract. Every successful gate must bind the live PR's immutable
   `(base SHA, head SHA)` pair before and after evidence collection. A base retarget or target-branch
   update must invalidate an earlier success even when the head SHA does not move. The CodeRabbit
   adapter must verify provider identities, a substantive exact-head review, completion state, and
   bounded correlation; `CHANGES_REQUESTED`, paused, skipped, rate-limited, malformed, and stale
   evidence fail closed. No workflow may check out or execute pull-request code.
2. Run a sacrificial public-repository end-to-end trial from dispatch comment through required
   status. Record command acknowledgement, review/status objects, latency, rerun behavior, base
   movement, head movement, rate limits, and every terminal classification. Unit tests and public
   examples from unrelated repositories do not establish this integration.
3. Complete the frozen ReviewTransition-30 materialization, independent oracle, and two clean bare-
   repository replays. Require zero false `skip` and zero false carry. Then preregister the larger
   RT-300 selection and thresholds before observing its product outcomes.
4. Make Review Cache receipts survive multiple updates without laundering verdicts. A residue
   receipt must retain the complete current identity/outcome ledger, reverify prior signed lineage,
   execute a bound deterministic cross-item outcome rule, preserve blocking outcomes, and use a
   domain-separated signature. Open dependency closure routes to `full` or `blocked`, never reuse.
5. Measure the dispatch claim in at least five live reviewer workflows. Capture actual provider
   invocations, cancelled work, wall time, tokens or billed cost where available, final-input
   coverage, and false-gate incidents. The three-PR replay remains directional evidence only.
6. Use the composite Action as an installable alpha and enterprise escape hatch. Harden the
   runnable dedicated GitHub App MVP that now owns the expected Check source, PostgreSQL webhook
   state, durable outbox, and fenced leases. Before calling it a production control, prove the live
   App/ruleset/merge-queue/CodeRabbit path and race its workers on real PostgreSQL.
7. Keep the human trust path releasable. From outside a checkout, a verified binary plus authenticated
   `gh` must run `stratadiff resume https://github.com/OWNER/REPO/pull/N`, materialize bounded source
   in an isolated temporary repository, and open the evidence Workbench. Missing or unverifiable
   history must stop explicitly.
8. Freeze separate value studies: automation measures cost, latency, and final-input coverage;
   Resume measures completion time and issue recall. Repository-path reduction alone is diagnostic
   evidence, not user value.
9. Maintain deterministic artifacts and offline verification. Publish every benchmark case and
   refusal, retain exact provider observations, and produce a reproducible release before treating
   any alpha component as a production control.

P0 exits only when the exact-input gate survives head and base races in a live repository, the
automation route passes its preregistered zero-false-skip gate, at least five live workflows show a
measured economic or latency benefit, and no known factual misstatement remains. Passing unit
tests, one happy-path PR, simulated billed invocations, or the historical AST benchmark alone is
insufficient.

### P1: compound trust and distribution

1. Persist per-reviewer checkpoints, partial-file state, and finding dispositions across pushes.
   Carry state only when exact Git identity or a separately verified relation permits it; invalidate
   everything else visibly and never silently re-anchor a comment.
2. Expand the beta App into the optional team control plane with read-only contents and metadata,
   `checks: write`, organization-member read only when team ownership requires it, retention
   controls, policy, and audit logs. Keep the CLI, schema, and verifier open and offline-capable.
3. Extend the implemented multi-file Evidence Workbench with residue navigation and reviewer
   dispositions while keeping passport verification separate from GitHub's conversation UI.
4. Import producer evidence from OpenRewrite and language-specific evidence from tools such as
   RefactoringMiner. Preserve the source and strength of every claim.
5. Add incremental analysis, caching bound to blob IDs, cancellation, queueing, and large-PR load
   tests. Publish the eligibility and failure rates instead of hiding unsupported cases.
6. Add GitLab and local pre-review adapters through the same passport schema.
7. Offer a paid control plane only for team coordination: durable ledgers, SSO/RBAC, policy,
   organization analytics, and support. Do not paywall offline verification of a passport.

## Distribution and promotion flywheel

### Distribution architecture after value validation

The scale path is an open-core GitHub App, not an Action-only product. GitHub Apps can request
`checks: write` and publish Check Runs through the
[Checks API](https://docs.github.com/en/rest/checks/runs?apiVersion=2022-11-28). Actions can also
publish checks when their `GITHUB_TOKEN` has that permission; the App advantage is zero-YAML
installation, durable webhook state, and a native requested-action loop. Actions on forked pull
requests can lose secrets, receive a read-only token, or await approval. The intended surfaces are
therefore:

- **Hosted GitHub App:** a runnable private MVP today and the eventual public zero-YAML product
  surface. It owns per-PR dispatch leases, invalidates stale base/head evidence, and publishes the
  dedicated required final-input Check. Public installation still depends on live ruleset,
  merge-queue, CodeRabbit, and real-PostgreSQL concurrency validation.
- **Native `stratadiff`:** the permanent local-trust and recovery surface. It resolves the
  checkpoint, keeps source local, opens the Workbench, and verifies downloaded Passports. The public
  `gh-stratadiff` distribution repository can add the `gh stratadiff` spelling without changing this
  trust path once its first verified release and clean-install/update tests pass.
- **GitHub Action/self-hosted runner:** the transparent alpha, privacy path, and enterprise escape
  hatch. It is not the eventual default onboarding or scheduler.

Marketplace is a later amplifier rather than a launch dependency. GitHub's paid-listing requirements
include at least 100 installs and verified publisher status, so the beta should begin as a direct
public-App install and earn real retained use first
([Marketplace requirements](https://docs.github.com/en/apps/github-marketplace/creating-apps-for-github-marketplace/requirements-for-listing-an-app)). Public installation and Marketplace counts are acquisition
proxies, not evidence of active use or value. The operating north star is **Weekly Governed Final
Inputs**: unique PR `(base, head)` pairs for which StrataDiff either avoided an obsolete invocation
or verified the current input before merge. This metric must be reported beside false gates,
uncovered merges, added latency, and measured cost; activity alone is not value.

The MIT-licensed engine, schemas, CLI, Action, Passport export, and offline verifier remain free.
Paid scope begins only at coordinated operations: private hosted repositories, durable cross-repo
ledgers, CODEOWNERS policy, organization analytics, SSO/RBAC, audit export, support, and enterprise
deployment. Verification and evidence export cannot be paywalled without undermining the trust
model.

The distribution loop should begin inside a workflow the team already pays for:

1. A maintainer installs the transparent Action pilot in front of an existing reviewer; it never
   receives a model-sales pitch or asks the team to migrate review conversation.
2. Every run exposes one auditable result: obsolete work avoided, current input waiting, exact input
   verified, or an explicit blocker. A per-run counter shows measured invocations and latency, not
   an estimated percentage.
3. A reviewer can open local Resume only when they want to inspect evidence or continue manually.
4. Repositories with repeated measurable value move to the zero-YAML App for durable dispatch,
   required checks, and organization analytics; repositories without churn receive an honest “not
   useful here” result.
5. Opt-in real case studies and frozen failure cases improve the public benchmark and provider
   adapters, increasing trust and earning more installations.
6. Open reviewers and refactoring tools can emit compatible signed receipts, increasing safe reuse
   without weakening the exact-input gate.

Initial channels should be AI-review-heavy open-source maintainers, DevEx/platform teams, reviewer
vendors, and transparent engineering write-ups built around reproducible dispatch ledgers. Avoid
generic “AI reviews your PR” positioning, paid vanity benchmarks, and unsolicited PR-comment spam.

The initial search and Marketplace language should name the pain rather than the mechanism:
`AI code review every push`, `CodeRabbit review limit reached`, `cancel stale PR review`, and
`require review on latest commit`. Public case cards should show complete accounting—for example,
“11 pushes, 4 dispatched reviews, final `(base, head)` verified”—with a downloadable evidence
artifact instead of an unsupported savings percentage.

The memorable outcome message is:

> **Do not pay to review a commit that will never merge. Do not merge one that was never reviewed.**

## Precommitted stop and pivot conditions

These are internal decision rules, not claims that the thresholds have already been met. Freeze the
pilot protocol before observing results.

- **Safety stop:** any passport that passes independent verification but makes a false factual
  predicate blocks release. Repeated failures that cannot be isolated by the verifier end the
  “proof-carrying” positioning.
- **Recall stop:** if the assisted condition lowers critical/high-severity issue recall, do not ship
  automatic secondary-lane collapsing, regardless of time saved.
- **Value pivot:** after at least 100 eligible review sessions across at least 20 reviewers, if the
  median time saving in the primary ICP is below 20% or confidence intervals do not support a
  meaningful gain, stop investing in a standalone review UI. Retain the engine as a verifier or
  artifact format.
- **Coverage pivot:** if fewer than 20% of changed lines enter evidence-backed secondary review on
  the median *target-segment noisy PR*, the wedge is too narrow. Integrate producer attestations or
  focus on migration/refactoring workflows instead of weakening evidence.
- **Retention pivot:** after an eight-week design-partner beta with at least 20 activated teams, if
  fewer than 25% still use passports weekly, do not build the hosted control plane until the JTBD is
  revalidated.
- **Trust stop:** if teams consistently interpret a secondary lane as “safe to skip” despite UI,
  documentation, and policy controls, rename or remove the lane before expanding distribution.
- **Integration pivot:** if most target teams already possess stronger recipe provenance and do not
  value post-hoc verification, become the interoperable passport/verifier layer for those producers
  rather than competing with them.

The thresholds are intentionally demanding and may be revised only before a new study, with the
revision recorded. The objective is not to preserve the original idea; it is to discover whether a
defensible reduction in human review load exists.

## Immediate product test

The next milestone is not another diff feature. It is one adversarial end-to-end Governor proof:

```text
sacrificial GitHub repository with an existing AI reviewer
  -> install the pinned Governor without checking out PR code
  -> rapid pushes supersede or debounce obsolete work
  -> latest eligible (base SHA, head SHA) dispatches exactly once
  -> provider acknowledgement, substantive review, and completion evidence agree
  -> head change, base push, retarget, paused review, rate limit, and CHANGES_REQUESTED each fail closed
  -> only the still-live exact input publishes the required success
  -> measured invocation, latency, and terminal-state ledger survives download
  -> an independent replay reproduces the gate decision
```

After that proof, repeat the same measurement in at least five real reviewer workflows and finish
ReviewTransition-30 with zero false skips/carries. Resume, Audit, and Inbox explain and recover from
the loop; they must not compensate for a Governor that cannot safely invalidate a stale base or
prove real cost/latency value. If the loop produces no measurable benefit, the stop conditions
should force a narrower verifier/attestation product instead of a larger unproven review platform.
