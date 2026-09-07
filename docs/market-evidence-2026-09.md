# Market evidence: verifiable merge proof across reviewers and candidate SHAs

Evidence captured: **2026-09-06**, with a competitor and platform refresh on **2026-09-08**. This is
a point-in-time, auditable product-research snapshot. Issue votes, reactions, comments, labels, and
states can change. Public issue reports are self-selected evidence of a failure mode or demand;
they are not prevalence, willingness-to-pay, retention, or market-size estimates. Vendor
documentation establishes documented behavior, not independent accuracy or adoption.

## Decision

The original single-provider thesis is falsified strongly enough to change the product. The
recurring commercial job is not “show a nicer diff,” “sell another reviewer model,” or “stop
CodeRabbit from approving an old head.” The immediate job is:

> Explain why this exact PR or merge-queue candidate is blocked: which effective rule expects which
> context from which producer, what appeared on the exact SHA, what never appeared, and what the
> developer should do next.

The strongest initial segment is a GitHub Cloud team with multiple AI, policy, CI, or human review
signals; required checks; and either strict up-to-date protection or a merge queue. Several pushes
per PR, finite review credits, stacked changes, and expensive CI sharpen the pain. The evidence does
not yet show how often these teams need a cross-provider proof or whether they will pay for it.

The primary acquisition wedge is therefore **Pull Request Doctor**, a read-only command that works
before an App install and produces an exact-head diagnostic bundle. Repeated incident evidence can
then justify the broader **Review Evidence Control Plane**: record provider, review object, reviewed
head, scope, policy generation, override provenance, and source App; evaluate PR-head obligations
separately from a synthetic `merge_group`; and withhold required success whenever evidence is
missing, stale, ambiguous, or from the wrong source. Review Cache and Review Resume remain
optimization and recovery surfaces, not merge authority. The hosted path remains an alpha thesis
until live multi-provider, ruleset, and merge-queue trials establish correctness and user value.

**No-Go:** a standalone “CodeRabbit stale-approval guard.” CodeRabbit's current official contract
already verifies that the latest commit was reviewed, the current head belongs to the reviewed
commits, and the head did not change immediately before approval. Building more infrastructure for
that claim alone would duplicate a fast-moving incumbent.

## Evidence-strength labels

- **Repeated demand:** multiple independent users, durable engagement, or the same request across
  hosts. This supports an initial customer job, not population prevalence.
- **Segment cluster:** multiple reports inside a specific workflow such as stacked PRs. This is
  strong ICP evidence but should not be generalized to append-only review workflows.
- **Measured case:** a real organization or pull request with a reported denominator or observable
  event history. It remains specific to that environment.
- **Individual case:** one reproducible report. It proves possibility, not frequency.
- **Platform fact:** current official documentation. It describes a mechanism or incumbent
  capability, not demand for StrataDiff.

## 2026-09-08 falsification and platform update

The following evidence is more decision-relevant than an unverified “stale AI approval” story. The
official sources establish platform or vendor behavior; community discussions establish concrete
reported failures only.

| Evidence | Current fact | Product decision |
|---|---|---|
| [GitHub required-check troubleshooting](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks), accessed 2026-09-08 | Required checks from earlier commits do not satisfy the latest commit SHA. When a test-merge commit has status, that candidate—not merely the PR head—must pass. A check and commit status with the same name must both pass. | Do not claim GitHub blindly accepts an old green SHA. Model PR head and final candidate explicitly. |
| [GitHub protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches), accessed 2026-09-08 | A required check can be pinned to an expected GitHub App; another actor's same-named result does not satisfy it. GitHub also warns that duplicate job names across workflows are ambiguous. | Stable Check identity and explicit source pinning are configuration hygiene, not novel product primitives. Rulesets use optional `integration_id`; branch protection uses `checks[].app_id`. Those fields authenticate the result source, not the review semantics behind it; if they are unset, do not claim App-bound enforcement. |
| [GitHub merge-queue configuration](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue), accessed 2026-09-08 | Actions must handle `merge_group`; third-party CI must handle `gh-readonly-queue/{base}`. The temporary candidate has a different SHA from the PR. | The proof target is the final candidate. Never copy a PR-head green result onto a merge-group SHA without a declared, independently checked carry rule. |
| [GitHub protected review behavior](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches#require-pull-request-reviews-before-merging), accessed 2026-09-08 | GitHub can dismiss approvals when its recorded diff changes and can require approval of the most recent reviewable push. | A generic stale-human-approval guard is also not a sufficient wedge. |
| [Copilot code-review documentation](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/request-a-code-review/use-code-review) and [2026-09-01 launch note](https://github.blog/changelog/2026-09-01-copilot-code-review-can-now-approve-pull-requests/) | Copilot does not re-review a new push unless configured to do so; it may repeat resolved feedback. Its public-preview approval is dismissed after a new commit. | Copilot needs reliable orchestration and evidence normalization, not a duplicate stale-approval feature. Its native capability may improve quickly. |
| [GitHub Community #190754](https://github.com/orgs/community/discussions/190754), created 2026-03-27, with reports through 2026-07-15 | One public PR repeated the same incorrect suggestion across three or four review rounds; commenters report the same failure and loss of trust. GitHub's own documentation acknowledges repetition is possible. | Finding disposition and deduplication are real follow-on jobs. This does not prove demand for a merge gate. |
| [GitHub Community #206810](https://github.com/orgs/community/discussions/206810), created 2026-09-04 | A public MLflow report found 78 Copilot reviews among 100 recent PRs, all `COMMENTED`; 18 displayed “Approval recommended” despite approval settings being enabled. | Evaluate API objects, not UI prose. This is one user-reported public-preview failure and not an official GitHub incident or prevalence estimate. |
| [CodeRabbit Request Changes Workflow](https://docs.coderabbit.ai/pr-reviews/request-changes-workflow), modified 2026-09-01 | CodeRabbit requires a completed review of the latest commit, checks that current HEAD is among reviewed commits, and rechecks HEAD immediately before approval. A rate-limited or changed head remains pending. Explicit `approve`/`resolve` commands can override latest-review and pre-merge-check requirements while still checking HEAD. | **No-Go:** do not sell a CodeRabbit stale-head guard. Treat CodeRabbit as one evidence producer and make override policy and provenance explicit across providers. |
| [CodeRabbit automatic-review controls](https://docs.coderabbit.ai/configuration/auto-review), modified 2026-08-31 | Incremental review is on by default but auto-pauses after five reviewed commits; each eligible push consumes review allowance, and the vendor recommends earlier pause/manual review for active branches. | Cost, noise, and final-coverage tension are credible. Measure safe coalescing and carry; do not assume savings. |
| GitHub Community [#103114](https://github.com/orgs/community/discussions/103114), created 2024-02-02 with reports through 2026-05-18, and [#43988](https://github.com/orgs/community/discussions/43988), with a 2026-04-16 adoption-blocking report | Required checks for PR and merge queue remain coupled in reported workflows, causing duplicate expensive runs or stuck queues and awkward same-name/conditional workarounds. | A canonical aggregate Check plus configuration audit is a stronger acquisition wedge than another reviewer. These reports do not quantify GitHub-wide frequency. |
| Graphite [merge queue](https://graphite.com/docs/graphite-merge-queue), modified 2026-01-22, and [setup](https://graphite.com/docs/set-up-merge-queue), modified 2026-06-22 | Graphite's queue optimizes stack throughput but is incompatible with GitHub's native queue; its fast modes can require App bypass permissions, and one repository has one trunk queue. | Do not build another queue. Remain a neutral proof layer that can sit beside the team's chosen merge path. Absence of a documented Graphite proof contract is not proof that no private capability exists. |

### Stuck-PR incident cluster

These reports make the first Doctor cause codes concrete. They prove that the failures occur, not
how often they occur or that a separate product will be adopted.

| Evidence | Observed failure | Doctor implication |
|---|---|---|
| [CodeQL Action #1537](https://github.com/github/codeql-action/issues/1537), [Read the Docs #10021](https://github.com/readthedocs/readthedocs.org/issues/10021), and [Danger JS #1427](https://github.com/danger/danger-js/issues/1427) | Independent projects report merge queues waiting forever because a required producer never publishes the expected status for the synthetic candidate. CodeQL #1537 had 43 reactions at capture. | Trace required context → producer → exact candidate SHA → absent event. |
| [GitHub Docs #8926](https://github.com/github/docs/issues/8926), current [required-check troubleshooting](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks), workflow syntax for [GitHub.com](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#git-diff-comparisons), [GHES 3.21](https://docs.github.com/en/enterprise-server@3.21/actions/reference/workflows-and-actions/workflow-syntax#git-diff-comparisons), and [GHES 3.22](https://docs.github.com/en/enterprise-server@3.22/actions/reference/workflows-and-actions/workflow-syntax#git-diff-comparisons), plus [pull-files REST documentation](https://docs.github.com/en/rest/pulls/pulls?apiVersion=2022-11-28#list-pull-requests-files) | A workflow skipped by path or branch filtering can leave its required check pending forever, while a skipped job reports success. GitHub.com and GHES 3.22 evaluate only the first 3,000 files of a larger generated diff; GHES 3.17–3.21 use 300. The pull-files endpoint returns at most 3,000 files. | Distinguish workflow-trigger reachability from job-level conditions and offer a patch preview. Bind the deployment/version before applying its limit; when that is unavailable, retain an unknown above the portable 300-file lower bound. A capped API response is not proof of a complete ordered evaluation set. |
| [Kubernetes SIG Prow #915](https://github.com/kubernetes-sigs/prow/issues/915) | A successful same-name Check can still fail policy when the requirement is bound to another App. | Preserve expected and observed App IDs; context strings alone are not identity. |
| [GitHub CLI #9839](https://github.com/cli/cli/issues/9839) | The CLI can collapse the reason to “base branch policy prohibits this merge.” | This generic error is the high-intent entry moment for one-command root-cause diagnosis. |
| [Flutter #162882](https://github.com/flutter/flutter/issues/162882) and [LLVM #151135](https://github.com/llvm/llvm-project/issues/151135) | Flutter reports completed state that never reached the gate; LLVM reports repeated rebases cancelling and rerunning CI. | The later flight recorder must separate producer failure, never-scheduled work, lost completion, and superseded candidates, then quantify duplicated work. |
| GitHub [Rules Insights documentation](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository#viewing-insights-for-rulesets) and [2026-08-25 GA announcement](https://github.blog/changelog/2026-08-25-rule-insights-dashboard-generally-available/) | GitHub provides native rule pass, fail, bypass and Evaluate results, with drill-down and CSV export. | **No-Go:** a ruleset dashboard or generic shadow mode is not differentiated. Doctor must connect the failed rule to the exact candidate, App, producer and workflow trigger. |

### Public-incident evaluation backlog

These incidents are candidates for the end-to-end replay corpus. The table separates cases Doctor
can decide from cases that should remain unknown. Public issue text and screenshots are not copied
into the dataset. A fixture may retain the URL, issue or PR number, immutable SHAs, workflow blob
IDs, App IDs, check contexts and a clean-room reproduction.

| Incident | Observed mechanism | Safe Doctor behavior |
|---|---|---|
| [actions/setup-node #1107](https://github.com/actions/setup-node/issues/1107) | A Node matrix or job-name change left an older required context pending. | Abstain until Doctor can expand the relevant static matrix and compare it with effective policy. |
| [openml-python #1573](https://github.com/openml/openml-python/issues/1573) and [PR #1572](https://github.com/openml/openml-python/pull/1572) | Deleted Docker and Windows matrix contexts remained required; the project added dummy jobs to unblock merges. | Treat this as a stale-context ground-truth case, but abstain with the current matrix parser. |
| [Cisco Catalyst WAN SDK #439](https://github.com/cisco-open/cisco-catalyst-wan-sdk/issues/439) | A documentation-only PR did not start a path-filtered workflow whose job was required. | Diagnose only after collecting the complete changed-file set and one unique static producer. |
| [Comfy Desktop #1483](https://github.com/Comfy-Org/desktop/issues/1483) and [PR #1479](https://github.com/Comfy-Org/desktop/pull/1479) | `paths-ignore` skipped a required job provided through a reusable workflow. | Keep the result unknown until the pinned reusable definition can be resolved. |
| [cargo-dist #1069](https://github.com/axodotdev/cargo-dist/issues/1069) | A required workflow did not subscribe to `merge_group`. | Diagnose when the merge-group SHA, App-bound context and unique exact-SHA workflow producer are all proved. |
| [Danger JS #1427](https://github.com/danger/danger-js/issues/1427) | The Actions job completed, but the external Danger status never appeared for the merge-group payload. | Report that the expected provider did not emit the status. Do not infer its internal failure from GitHub evidence. |
| [Prow #915](https://github.com/kubernetes-sigs/prow/issues/915) | A same-name green Check came from a different App than the required source. | Diagnose a source mismatch only when the policy exposes the expected App ID. |
| [GitHub CLI #9839](https://github.com/cli/cli/issues/9839) | A PR created with `GITHUB_TOKEN` did not trigger the later workflow, while the CLI showed only a branch-policy error. | Keep the cause unknown unless event and token ancestry are available; the missing check itself is still observable. |
| [Mergify #5139](https://github.com/Mergifyio/mergify/issues/5139) | An external controller retained an obsolete required-check name after GitHub configuration changed. | Expose an external-controller evidence gap. GitHub-only data cannot prove the controller's cache state. |
| [OpenTelemetry opamp-go #320](https://github.com/open-telemetry/opamp-go/issues/320) and [PR #313](https://github.com/open-telemetry/opamp-go/pull/313) | Closing and reopening the PR restored a check, but the original failure was not established. | Use this as an abstention control; recovery after reopen does not identify the original cause. |

The strongest counter-evidence is CodeRabbit's exact-head contract plus GitHub's latest-SHA and
expected-App enforcement. It removes the original single-provider security wedge. The public
evidence does show duplicated work, queue configuration brittleness, review noise, and preview
reliability problems; it does **not** yet establish willingness to pay for cross-provider
attestation. That demand must be measured before deeper control-plane scope is justified.

GitHub's [`merge_group` webhook](https://docs.github.com/en/webhooks/webhook-events-and-payloads#merge_group)
identifies the synthetic head and base but does not enumerate the member pull requests. A live proof
therefore needs separately validated queue/PR observations and must fail closed when membership
cannot be established. A first-time historical scan can check workflow triggers and still-resolvable
objects, but it cannot reconstruct an uncollected merge-group membership timeline.

## Real pain evidence

| Strength | Source and point-in-time status | First-hand statement or observable fact | What it supports | Boundary |
|---|---|---|---|---|
| Repeated demand | [GitHub Community #3478](https://github.com/orgs/community/discussions/3478), created 2021-03-04, open, updated 2026-05-04; 305 upvotes, 11 comments + 20 replies, 15 distinct commenters | After several force-pushes, a reviewer wrote that they “often have no choice but to re-review an entire PR, losing a lot of time” ([comment](https://github.com/orgs/community/discussions/3478#discussioncomment-11345452)). Another asks for a cumulative review-relative range rather than inspecting each force-push. | Automatic reviewer checkpoint recovery across multiple pushes is a long-lived, explicit job. | Self-selected community participation; no incidence, saved-time, or payment data. |
| Repeated demand | [GitHub Community #57513](https://github.com/orgs/community/discussions/57513), created 2023-06-08, open, updated 2026-08-28; 126 upvotes, 22 comments + 10 replies, 22 distinct commenters | The original stacked-PR report says “I had to ask my colleagues to reapprove.” Multiple companies report blocked or slower workflows; a 2026 commenter says approving one layer dismisses approvals above it ([comment](https://github.com/orgs/community/discussions/57513#discussioncomment-18140755)). | Approval invalidation caused by base movement is not confined to one team and remains relevant to stacks. | Reports span different settings and GitHub behavior revisions; they do not prove one root cause or current behavior for every repository. |
| Repeated demand | [GitLab #25559](https://gitlab.com/gitlab-org/gitlab/-/work_items/25559), created 2018-12-03, open, updated 2026-07-29; 69 upvotes and 14 notes | The author must infer the desired version from comment timestamps and memory and calls that “far from ideal”; the request is for GitLab to know each reviewer's last-reviewed version. | Reviewer-specific resume is a cross-host and multi-year need, not GitHub-specific wording. | An open backlog item and votes do not establish delivery priority, incidence, or willingness to pay. |
| Demand signal | [GitHub Community #141845](https://github.com/orgs/community/discussions/141845), created 2024-10-17, open, updated 2025-04-29; 28 upvotes and 2 comments | “Our team uses the changes since last review a lot, but it does not work when branches are rebased and force-pushed.” Workarounds require selecting commits, editing a URL, or using command-line tools. | The default flow must recover a checkpoint without asking for a SHA or manual range construction. | One team's detailed report with supporting votes; no measured frequency. |
| Segment cluster | [GitHub `gh-stack` #323](https://github.com/github/gh-stack/issues/323), created 2026-07-27, open, updated 2026-09-03; 74 reactions, 25 comments, 22 distinct commenters | A six-layer-stack report calls the behavior “an operational blocker, not an edge case” and describes O(N) human re-approvals when lower layers land ([comment](https://github.com/github/gh-stack/issues/323#issuecomment-5400713897)). Another team reports waiting hours for repeated deployment checks ([comment](https://github.com/github/gh-stack/issues/323#issuecomment-5486270077)). | GitHub's new native stack workflow creates a timely, concentrated design-partner segment where review and CI churn have visible cost. | The issue also contains stack-merge orchestration bugs unrelated to review continuity; counts must not all be attributed to StrataDiff's job. |
| Segment cluster | [GitHub `gh-stack` #354](https://github.com/github/gh-stack/issues/354), created 2026-07-29, open, updated 2026-09-05; 25 reactions, 7 comments, 6 distinct commenters | The report says `gh stack sync` force-pushes and “erases useful information about changes since last view.” A commenter says stacking then breaks “just about every feature designed to make reviewing a PR tenable” ([comment](https://github.com/github/gh-stack/issues/354#issuecomment-5228462868)). | Force-push-safe review memory is a current gap for users of GitHub's own stack tooling. | Early public-preview feedback can be fixed by GitHub; a product cannot depend on this specific bug remaining. |
| Measured case | [GitLab #439234](https://gitlab.com/gitlab-org/gitlab/-/work_items/439234), created 2024-01-24, open, updated 2026-06-19; 16 notes | The author reports a 15% incidence of “unwanted patch-id changes” after merge-from-parent in one active project with 1,000+ developers and 50,000 files. Nearby target-branch edits can change the patch ID even when there are no “changes to the changes.” | Whole-diff patch identity can still produce measurable false invalidation under base drift. | One organization's retrospective analysis; its method and result require independent replication. |
| Measured case | [Zephyr #43701](https://github.com/zephyrproject-rtos/zephyr/issues/43701), created 2022-03-11, closed as completed 2023-04-27; 8 comments, plus merged [PR #41626](https://github.com/zephyrproject-rtos/zephyr/pull/41626) | A maintainer wrote, “Reviewers just click on +1 automatically without actually re-reviewing,” describing overhead without realistic gain ([comment](https://github.com/zephyrproject-rtos/zephyr/issues/43701#issuecomment-1069290021)). The PR timeline contains 27 force-push events and 13 review-dismissal events at capture. | Coarse invalidation can cause mechanical reapproval rather than better review, especially in large multi-reviewer changes. | Deliberately selected project and PR; the counts prove this history, not a population rate or causal effect. |
| Individual case | [GitHub `gh-stack` #446](https://github.com/github/gh-stack/issues/446), created 2026-08-14, open, updated 2026-08-24; 1 reaction and 1 comment | The reporter says a restack rewrote commit IDs while PR content remained byte-identical, dismissed three approvals, and reset every CI check; “3 reviewers now have to re-approve and CI has to fully rerun.” | Exact content identity can survive rewritten commit identity and avoid demonstrably redundant work. | One current report; the exact behavior may be a fixable stack-preview bug. |
| Individual case | [GitLab #241509](https://gitlab.com/gitlab-org/gitlab/-/work_items/241509), created 2020-08-26, open, updated 2026-06-19; 9 upvotes and 2 notes | In a force-push workflow, “the details of what changed are obscured,” impairing reviewers' ability to understand the update at a glance. | Force-push retention and intelligible change context are cross-host requirements. | Low-volume feature request; GitLab already retains selectable diff versions. |
| Individual case | [GitLab #442454](https://gitlab.com/gitlab-org/gitlab/-/work_items/442454), created and closed 2024-02-20; 6 notes | Comparing a rebased MR version with an older one includes “all the changes that happened ... on the target branch”; the author proposes `git range-diff`. | A raw checkpoint-to-head comparison absorbs base noise; author residue and base influx must be separated. | Closed quickly, with no inference here about closure reason or roadmap commitment. |
| Individual case | [GitLab #594565](https://gitlab.com/gitlab-org/gitlab/-/issues/594565), created and closed 2026-03-24; 1 note | After a conflict-resolution rebase resets approvals, the reviewer is “forced to revisit all the files” and asks to preserve identical file or block approval. | Large reviews need finer invalidation than an entire review decision. | One proposal with no votes; closure means it is not an active demand aggregate. |
| Individual case | [GitLab #604779](https://gitlab.com/gitlab-org/gitlab/-/work_items/604779), created 2026-07-02, open, updated 2026-09-03; 5 notes | A commit in one CODEOWNERS domain clears approvals in unaffected domains, “forcing them to re-review code they have already validated.” | Coverage eventually needs reviewer × ownership-domain scope rather than one global checkpoint. | One recent enterprise-style request; GitLab already documents a separate affected-Code-Owner reset option in ordinary project settings. |
| Individual edge case | [GitHub Files Changed feedback comment](https://github.com/orgs/community/discussions/163932#discussioncomment-13622332), posted 2025-06-30; parent discussion remains open | A file was added and reviewed, then deleted; Changes since last review was “unable to render or display” the change. | Dropped or reverted reviewed work must remain explicit even when absent from the current PR range. | One report inside a broad feedback thread; it establishes possibility only. |
| Repeated implementation failure | VS Code GitHub extension [#4510](https://github.com/microsoft/vscode-pull-request-github/issues/4510), created 2023-02, still open with 15 reactions and 7 comments; related [#5455](https://github.com/microsoft/vscode-pull-request-github/issues/5455) and [#6281](https://github.com/microsoft/vscode-pull-request-github/issues/6281) were closed as completed | Reporters describe merges from the base branch injecting dozens of unrelated files into Changes since last review; one calls the view “near impossible in certain circumstances.” | Base-noise exclusion is a recurring implementation trap and should be an explicit benchmark stratum. | Two reports were fixed in one client; do not present them as a current GitHub Web limitation. |

## Automated-review rerun evidence

These cases were captured on 2026-09-06. Their cost and causal explanations are claims made in the
linked issues or pull requests unless explicitly described as directly observable. They establish
concrete failure modes, not expected savings or market prevalence.

| Strength | Source | Observed problem | Product implication |
|---|---|---|---|
| Reproducible multi-repository study | [`acoliver/vibetools` CodeRabbit throttling study at `d57789d`](https://github.com/acoliver/vibetools/tree/d57789dc2c17f2be39efe2c437f25ea98457fae8/research/ai-code-review-study/coderabbit/throttling) | Across three related repositories, the study records 1,024 CodeRabbit-touched PRs, 7,600 commits, an estimated 6,576 follow-up commit updates, and 256 distinct PRs retaining an exact `Review limit reached` comment. One repository briefly disabled incremental review, saw no blocks in a 47-hour window, then restored it because follow-up commits went unreviewed. | This independently exposes the same coverage-versus-capacity tradeoff at larger scale. A gateway needs both demand shaping and head-bound review continuity; the purposive repositories and mutable bot comments do not establish population prevalence or causal savings. |
| Individual cost case | [`fullsend-ai/fullsend#6911`](https://github.com/fullsend-ai/fullsend/issues/6911) | A missing App client ID caused prior-review provenance lookup to fail silently. The issue reports a `$0.62` initial review followed by a `$2.15` full review for one formatting fix, with the prior comment overwritten. | Receipt provenance is part of correctness. Missing or invalid provenance needs an explicit route and durable history, not a silent cache miss. |
| Individual cost case | [`fullsend-ai/agents#1092`](https://github.com/fullsend-ai/agents/issues/1092) | Two completed runs cost `$7.38` and `$8.15`; the second surfaced one new low-severity finding, while two intermediate runs were cancelled. | A residue route can have directly measurable value, but run supersession and completed-receipt state must be modeled separately. |
| Measured workflow case | [`nexpeakcore/deepseek-harness-pr-review#26`](https://github.com/nexpeakcore/deepseek-harness-pr-review/pull/26) | One PR reached 57 review rounds and 58 bot comments because every head SHA caused a rerun. The proposed implementation hashes provider file metadata and refuses to skip an incomplete prior run. | Head movement is a noisy trigger. Exact completed-input identity is the useful key; provider patches alone are insufficient for the strongest proof. |
| Individual correctness case | [`Expensify/App#100173`](https://github.com/Expensify/App/issues/100173) | Running AI standards review only at open/ready time leaves violations introduced by later human-review fixes unchecked; running on every push would increase cost and noise. | The gateway must solve both sides: never miss a changed input, while avoiding a full rerun when a complete closed input is unchanged. |
| Individual capacity case | [`dotCMS/core#36962`](https://github.com/dotCMS/core/issues/36962) | Four AI review workflows launch per push without cancellation; the issue reports 271 aggregate queued job-minutes and superseded runs against obsolete commits. | Cancelling stale work is baseline orchestration hygiene. It belongs in the gateway loop, though native workflow concurrency already solves the narrow case. |
| Measured team-spend case | [`mento-protocol/monitoring-monorepo#2229`](https://github.com/mento-protocol/monitoring-monorepo/issues/2229) | The repository recorded 672 review events in 30 days, 365 billed events, 48 blocked events, and 3.9 events per PR. Its `$500/month` CodeRabbit add-on cap was reached 16 days before reset, so the team disabled automatic incremental review and estimated `$200–250/month` after moving to open-plus-closeout reviews. | Teams will pay to review updates, but a per-push trigger can exhaust a finite review budget. The useful product is a head-aware dispatch governor that retains final-head coverage instead of merely switching incremental review off. The projected saving is the issue author's estimate, not an observed post-change result. |
| Measured workflow tradeoff | [`BinaryStudioAcademy/bsa-2026-transcripta#181`](https://github.com/BinaryStudioAcademy/bsa-2026-transcripta/pull/181) | A branch reworked five times accumulated five similar summaries; one cited PR received two summaries and twelve inline comments in seven minutes, with duplicate file/line comments. The repository removed the per-push trigger, estimating 28 avoided runs across seven or eight PRs, while explicitly accepting that later fixes would no longer be checked automatically. | The status quo forces a bad choice between repetitive review noise and stale final-head coverage. A useful gateway must suppress redundant work without treating an unreviewed later push as clean. |
| Multi-PR operational case | [`shakacode/agent-workflows#746`](https://github.com/shakacode/agent-workflows/issues/746) | Across 82 open PRs the repository counted 103 top-level CodeRabbit comments and ten ready PRs blocked by stale changes-requested objects, then proposed disabling automatic incremental re-review while keeping the reviewer available on demand. | Comment churn and stale review state are user-facing costs in addition to model spend. The gateway should publish one head-bound state surface and supersede old runs rather than append one review summary per push. |
| Product-convergence case | [`quadseven/grug#557`](https://github.com/quadseven/grug/issues/557) | A review-agent project asks for a stored last-reviewed head, commit-range-scoped delta reviews, automatic pause during rapid pushes, and explicit resume/review commands because its current open-once policy can leave later commits unreviewed. | Exact transition identity, quiescence scheduling, and explicit final-head refresh form one job. Delta selection without dispatch control solves only half of the repeated-review loop. |
| Lineage correctness case | [`opena2a-org/ai-trust#76`](https://github.com/opena2a-org/ai-trust/pull/76) | A prior-review lookup silently read the wrong GitHub endpoint after the bot changed from reviews to issue comments, so every push was reviewed from scratch. The repair also required full pagination and bot-identity filtering to avoid replaying stale or quoted review text. | Review history is not a trustworthy cache receipt by convention. Provenance, pagination, actor identity, and completion state must be explicit inputs, with a full/blocked fallback when they cannot be established. |

Taken together with the previously captured Fullsend, Minspec, Qwen Code, and Alibaba cases, the
more defensible first automation job is:

> Before an expensive reviewer runs, wait for a useful head, supersede obsolete work, prove the
> exact input transition, and emit one auditable `skip`, `residue`, `full`, or `blocked` route.

This is intentionally a reviewer-agnostic preflight. The product should integrate with existing
review agents and policy checks rather than ask a team to replace the model or workflow that
already produces its findings.

The recurring adoption signal is not simply “incremental review is cheaper.” Teams are disabling
per-push review to control spend and noise, then explicitly accepting a coverage gap on later
commits. That makes the first sellable outcome sharper: **keep automatic final-head coverage on
without paying for every unstable intermediate head or re-reading provably unchanged input.** The
transition router remains the correctness core; CI readiness, a short quiescence window, native
workflow concurrency, and one head-bound status surface are orchestration around it. A deferred run
is scheduling state, not a fifth evidence decision and never evidence that a head is clean.

## Native capability and competitor counter-evidence

These sources narrow the claim. A generic interdiff, persistent revision list, per-file review state,
or selective approval Check is already occupied product territory.

| System | Documented capability at capture | Consequence for StrataDiff |
|---|---|---|
| GitHub | [Protected-branch documentation](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches#require-pull-request-reviews-before-merging) says GitHub records the complete diff state at approval and can dismiss it when that state or merge base changes. [Viewed files](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/reviewing-proposed-changes-in-a-pull-request#marking-a-file-as-viewed) remain marked until that file changes. The [2025-12-11 changelog](https://github.blog/changelog/2025-12-11-review-commit-by-commit-improved-filtering-and-more-in-the-pull-request-files-changed-public-preview/) added review of all, selected, or individual commits and a commits-since-last-review option. | Do not claim all progress resets or that append-only incremental review is missing. The unresolved job is reviewer-specific continuity through rewrites, dropped work, and base drift. |
| GitHub counterexample | [Community #12876](https://github.com/orgs/community/discussions/12876), created 2022-03, open but answered; 98 upvotes, 8 comments + 18 replies. A GitHub employee stated in [November 2023](https://github.com/orgs/community/discussions/12876#discussioncomment-7445850) that code-identical local rebases, splits, and squashes had been supported since June 2023. A 2026 commenter reports further improvement. | “Keep approval after a byte-identical ordinary rebase” is neither an untouched gap nor a sufficient product wedge. Reproduce exact current behavior before using old reports. |
| GitLab | [Approval settings](https://docs.gitlab.com/user/project/merge_requests/approvals/settings/) use `git patch-id`, described as reasonably stable, to avoid some unnecessary resets after rebase or target merge; GitLab can remove only approvals from affected Code Owners. [Diff versions](https://docs.gitlab.com/user/project/merge_requests/versions/) retain one version per push, and [Viewed files](https://docs.gitlab.com/user/project/merge_requests/changes/#mark-files-as-viewed) remain hidden until their content changes. | Patch-aware reset, version comparison, and owner-selective invalidation are established. The distinct claim must be exact reviewer-bound evidence, dropped/base-drift accounting, abstention, and portable verification. |
| Graphite | [Pull Request Versions](https://graphite.com/docs/pull-request-versions), modified 2026-01-22, offers Hide reviewed changes by comparing a user's last-reviewed version with the latest. Its [stack-review guidance](https://graphite.com/docs/best-practices-for-reviewing-stacks), modified 2026-01-22, recommends disabling both stale-approval dismissal and latest-push approval to keep stacks moving. | Do not compete on stack navigation or generic last-reviewed interdiff. The exposed opening is evidence-backed continuity between unsafe blanket preservation and costly blanket invalidation. |
| Gerrit | [Review UI documentation](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#normal-and-rebase-edits) compares arbitrary patch sets, identifies rebase edits using both parents, and omits files changed only by rebase. [Label copy conditions](https://gerrit-review.googlesource.com/Documentation/config-labels.html#label_copyCondition) copy votes for configured kinds such as `NO_CHANGE`, `NO_CODE_CHANGE`, and `TRIVIAL_REBASE`. Gerrit also documents a [hazardous stacked squash](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#hazardous-rebases) whose inter-patch-set diff is empty while parent content enters implicitly. | Gerrit demonstrates that review carry is a mature policy primitive and that two-snapshot interdiff can still hide parent influx. StrataDiff's opportunity is a GitHub-native, portable four-snapshot proof, not invention of rebase-aware review. |
| Reviewable | [File review documentation](https://docs.reviewable.io/files) tracks each reviewer × file × immutable revision, retains force-pushed revisions using repository refs, compares a reviewer's last-reviewed revision with latest, and heuristically maps rebased commits. Its maintainer declined cross-revision line-review carry because it is “too imprecise and much too likely to hide unreviewed lines” ([issue comment](https://github.com/Reviewable/Reviewable/issues/414#issuecomment-1611899307), 2023-06-28). | Persistent review memory is not novel. Conservative line or hunk carry is safety-sensitive; explicit proof and abstention are necessary but still need human validation. |
| Aviator FlexReview | [Validation documentation](https://docs.aviator.co/flexreview/concepts/validation-in-flexreview) publishes a GitHub status check, stores each approver's owned-file state at the approved commit, and selectively dismisses approvals when those files change. | A coverage Check and owner-selective validation are not unique. Differentiate on exact cross-rewrite reconstruction, dropped reviewed work, base-influx evidence, local execution, and independently verifiable artifacts. |
| GitHub CLI and Rules Insights | [`gh pr checks`](https://cli.github.com/manual/gh_pr_checks) lists observed checks and can filter to required ones. Rules Insights shows rule outcomes and Evaluate results. | Do not claim that GitHub lacks blocker visibility. Doctor must explain missing signals that never appear in the check rollup and connect them to App and workflow evidence. |
| Mergify | [`mergify config simulate`](https://docs.mergify.com/configuration/file-format/#validation-and-troubleshooting) evaluates configuration without executing actions. Its [conditions](https://docs.mergify.com/configuration/conditions/#qualifying-checks-by-github-app) can qualify a Check by App, and its [merge-queue lifecycle](https://docs.mergify.com/merge-queue/lifecycle/) surfaces blocking conditions. | Do not claim a generic policy simulator, App-qualified check name, blocker explanation or replacement queue as novel. Doctor must explain GitHub-native policy and producers it does not own. |
| Aviator and Trunk merge queues | Aviator's [merge queue](https://docs.aviator.co/mergequeue/) and Trunk's [failure guide](https://docs.trunk.io/merge-queue/using-the-queue/handle-failed-pull-requests) show waiting checks and queue-specific failures. | Explaining a queue that the vendor owns is occupied territory. StrataDiff must work without moving a repository onto its queue. |
| Aviator Verify | [Verify](https://docs.aviator.co/verify) and its [audit trail](https://docs.aviator.co/verify/concepts/audit-trails-and-compliance) already connect intent, criteria, evidence, verdict, and reviewer decision. | “Immutable proof” alone is occupied positioning. StrataDiff must win on exact-candidate interoperability and operational root cause, not audit-language marketing. |
| Datadog PR Gates | [PR Gates](https://docs.datadoghq.com/pr_gates/) already offers non-blocking rollout and failure context for Datadog security, coverage, and flaky-test signals; its setup documentation says a required gate is incompatible with repositories using GitHub merge queue. | A vendor-specific shadow Check exists. Doctor can add value by detecting that documented incompatibility before a team turns it into a permanent blocker. |

“Not documented” is not treated as proof that a vendor lacks a capability. No independently audited
accuracy, adoption, saved-time, or retention result was found for the vendor mechanisms above.

## What is broadly supported, and what remains an anecdote

The evidence supports four recurring mechanisms across GitHub, GitLab, Graphite, Gerrit, Reviewable,
and Aviator:

1. Reviewers need a durable personal checkpoint after later pushes.
2. Rebase and force-push can make commit-ancestry or naive interdiff views noisy or unusable.
3. Whole-review invalidation creates a security-versus-flow tradeoff.
4. Base movement and stacked dependencies need separate treatment from author-authored residue.

It does **not** establish:

- that most pull requests encounter the problem;
- that all force-pushes lose review context on current GitHub;
- that developers value proof more than a familiar heuristic interdiff;
- that StrataDiff saves reviewer time or preserves issue recall;
- that any team will install an App, retain it, or pay for it.

The repository's separate 500-PR Review Churn Census found checkpoint drift in 88 of 488 comparable
reviewer/PR checkpoints. That bounded panel supports an event-driven product, not a universal daily
workflow. It must not be blended with self-selected issue votes to manufacture a market-size claim.

## Long-term product: Review Evidence Control Plane

### User-visible contract

For every protected PR, publish one quiet, App-bound required state that explains the evidence for
the current merge candidate instead of treating each provider's comments, reviews, and checks as
interchangeable:

```text
StrataDiff Merge Proof — collecting
PR head a842…: CodeRabbit complete; Copilot pending; policy generation 17

StrataDiff Merge Proof — verified
merge group e19c…: 2 PR heads bound; 3 required evidence snapshots verified; no override
```

The state machine is deliberately small:

1. **Collecting:** at least one policy obligation is pending, rate-limited, or not yet observable.
   This is not a clean result.
2. **Verifying:** versioned, content-addressed evidence snapshots are being checked against the
   current PR head, provider identity, policy generation, and any separately established merge-group
   membership.
3. **Verified:** every required obligation is supported for this exact candidate by an allowed
   source and declared carry rule.
4. **Blocking:** a reviewer requested changes, an override violates policy, or evidence is missing,
   invalid, incomplete, timed out, or contradictory.
5. **Superseded or quarantined:** a newer head, base, policy generation, or unrouteable signed event
   invalidated the observation; old work cannot make a later candidate green.

The CodeRabbit adapter is one input adapter, not the product boundary. Because CodeRabbit already
documents exact-head protection, its safe role is to contribute a provenance-bound evidence
snapshot to a multi-reviewer policy. Incremental `skip` or `residue` remains reserved for reviewers whose
complete input closure and result contract are available to Review Cache. A UI statement such as
“approval recommended” is never substituted for the provider's actual review or Check object.

### Immediate implementation implication

The transparent Action alpha must never execute pull-request code and must bind every evidence read
to repository, PR, head, base, provider, evidence object, scope, and policy generation. A production
App must publish a fixed Check name from the expected App identity and treat a `merge_group` as a
new candidate whose SHA and member set are established from retained live observations and
separately validated PR/commit relations. The webhook alone is not membership evidence; unavailable
membership fails closed. PR-head evidence may carry only through a declared proof; it is never
copied because the queue entry “belongs to the same PR.”

The first public proof must include two real providers, a same-head/base-moved case, a head race,
retarget, provider pause or rate limit, explicit override, `CHANGES_REQUESTED`, a same-name result
from the wrong App, and merge groups containing one and multiple PRs. It must report actual provider
invocations, CI runs, latency, and every fail-closed reason; the current three-PR replay's billed-
invocation count remains only a proxy.

## Secondary product wedge: Verified Review Resume Check

### User-visible contract

After an existing human review and a later head or base change, publish one quiet Check rather than
a routine bot comment:

```text
Review checkpoint changed

Alice: 18 files still match evidence
       2 files / 11 lines need review
       1 reviewed edit was dropped
       base drift was checked separately

[Resume review]  [Inspect evidence]
```

The Check has four explicit outcomes:

1. **Current:** the submitted review is already bound to the current state.
2. **Resume:** a conservative residue is available, with carried and unreviewed scopes separated.
3. **Full review:** the relation is unsupported, interacting, ambiguous, or exceeds a limit.
4. **Unavailable:** the checkpoint or required objects could not be recovered; never report this as
   clean.

GitHub remains authoritative for comments and approvals. The first App release must be
informational. A required review-coverage gate is a later opt-in only after correctness, human-value,
and operational gates pass.

### Why this survives native product improvement

- GitHub and Graphite can improve ordinary last-review comparisons without supplying a portable
  proof for every carried item.
- GitLab and Aviator can improve selective invalidation without showing reviewer-specific dropped
  work or a hazardous parent influx.
- Gerrit's strongest model still documents an empty-interdiff stacked-parent hazard.
- The open-core verifier and Passport let a team check the factual claim independently rather than
  trusting a hosted verdict.

The moat is the combined evidence contract and accumulated real-history corpus, not the four-lane
visual summary. If users do not value this stronger contract, the wedge is not differentiated.

### Immediate implementation implication

Capture the receipt at review time before polishing more diff presentation. On the Zephyr example,
GitHub's REST API currently returns `commit_id: null` for all 13 dismissed review records while the
two final non-dismissed approvals retain their commit IDs. This point-in-time observation does not
define a platform guarantee, but it proves a post-hoc tool cannot assume old checkpoints remain
recoverable.

The minimum event-driven vertical slice is:

1. On a completed `APPROVED` or `CHANGES_REQUESTED` review, record immutable reviewer identity,
   review ID, old base, reviewed head, file/blob identities, ownership and permission snapshot, and
   a signed receipt.
2. On `synchronize`, base change, or review dismissal, resolve old base `A`, reviewed checkpoint `B`,
   current base `C`, and current head `D`; compute the conservative residue and explicit failures.
3. Publish an informational Check with reviewer-scoped counts and a Resume action.
4. Open the local or private Workbench for source inspection; keep source-retention and execution
   boundaries explicit.
5. When the reviewer submits a review on the current head, close that resume loop without inventing
   or restoring an approval.

## Initial customer profile

### Primary ICP

- GitHub Cloud teams, initially about 20–300 engineers and at least 100 PRs per month, that already
  combine two or more AI, policy, CI, or human review obligations.
- A DevEx, platform, or security owner who controls rulesets, can install a read-only audit or
  shadow Check, and can compare real invocation, CI-minute, latency, and override telemetry.
- Repositories using strict up-to-date checks or native merge queue, especially with stacks,
  frequent rebases, expensive CI, scarce CODEOWNERS, or agent-authored PRs.
- Teams that need to answer “which exact candidate did each required reviewer cover?” for incident
  review or internal change-control evidence, without replacing their current reviewer products.

Use a read-only **Merge Readiness Audit** and non-blocking shadow Check to qualify each repository.
The audit should identify wrong-source or duplicate check names, missing `merge_group` triggers,
path-filtered required workflows, final pushes without required evidence, repeated provider runs,
and policy overrides. A low-churn, single-reviewer, correctly configured repository should receive
an honest “not useful here” result rather than an enforcement pitch.

The primary job is:

> When an AI- or human-authored change reaches the merge boundary, prove that the exact PR head and
> final merge candidate satisfy every configured review obligation, without rerunning unchanged
> work merely to compensate for missing provenance.

### Poor initial fit

- Small, append-only PRs whose native Changes since last review view is sufficient.
- Repositories with one informal reviewer, cheap or unlimited automated review, and little PR churn.
- Teams asking primarily for AI bug findings, summaries, stack creation, or syntax-aware diff
  presentation.
- Environments that cannot preserve an event-time checkpoint under an acceptable source-retention
  and privacy model.

## Falsifiable evaluation plan

These are proposed go/no-go gates, not achieved results. Freeze the protocol and thresholds before
examining pilot outcomes.

### 0. Retrospective PR audit and demand test

Build a non-mutating analyzer over at least 500 consecutive public or consenting private PR
timelines that use Copilot, CodeRabbit, or both. Record review `commit_id`, check-run source App,
PR-head SHAs, pushes after the last completed review, duplicate review/CI runs, explicit overrides,
and missing or ambiguous objects. Record merge-group SHAs and membership only when retained events
or still-resolvable records establish them; otherwise report that dimension as unknown. Pre-register
the primary demand test: at least 10% of qualified PRs must contain an actionable proof or
configuration gap and at least 30% of surfaced findings must cause an administrator to fix
configuration, request evidence, or keep shadow mode. If fewer than 2% contain an actionable gap,
stop leading with stale evidence.

The analyzer must also record its permission and field visibility. GitHub's
[rules endpoints](https://docs.github.com/en/rest/repos/rules?apiVersion=2022-11-28) can return
effective rules with metadata access, while branch-protection and
[rule-suite](https://docs.github.com/en/rest/repos/rule-suites?apiVersion=2022-11-28) reads require
repository Administration read. Rule-suite history is limited to the API's hour, day, week, or month
window. GitHub withholds `bypass_actors` unless the caller has ruleset write access, so an omitted
bypass list is unknown rather than evidence that no bypass exists. The direct per-ref Check Runs
endpoint considers only the 1,000 most recent Check Suites; use separate suite enumeration beyond
that bound or mark collection incomplete.

### 1. Live merge-proof trial

Run the dedicated App first on a sacrificial organization with a ruleset pinned to its
`integration_id`, then in shadow mode on at least five consenting repositories. Use real Copilot and
CodeRabbit evidence where available and preserve the complete event, evidence, policy-generation,
candidate, override, Check, and repair ledger. Exercise at least 80 cases: push during review,
force-push, retarget, close/reopen, duplicated and reordered delivery, POST response loss, wrong-App
same-name check, path-filtered workflow, one- and multi-PR merge groups, reordering, eviction,
destruction, base-only drift, and a semantic conflict.

**Merge-proof gate:** zero successful states on a stale, incomplete, wrong-source, wrong-policy, or
wrong-candidate input; zero uncovered merges when the Check is required; p95 convergence below 30
seconds and p99 below 60 seconds after complete evidence; recovery from a missed delivery below five
minutes at p99; and a measured reduction in at least one buyer metric without worse final-candidate
coverage. Five pilots establish feasibility, not a population saving estimate.

### 2. Prospective correctness corpus

Collect consecutive review-event histories prospectively rather than selecting only successful
examples. Freeze exact event IDs and `A/B/C/D` object identities. Stratify at least:

- append-only push;
- code-identical rewrite;
- force-push with amended changes;
- stack restack or parent merge;
- non-interacting and interacting base drift;
- dropped or reverted reviewed edits;
- rename, add/delete, mode change, binary, submodule, large-file and missing-object cases.

Two independent adjudicators label required attention without seeing StrataDiff output; resolve
disagreement before scoring. Compare against GitHub's current native Changes since last review,
plain checkpoint-to-head diff, `git range-diff`, and stable patch ID where each baseline applies.

**Integrity gate:** zero known false carries among supported adjudicated cases, 100% verifier replay
success, and every missing or unsupported input represented as fail-closed. Report the statistical
upper bound implied by the sample; zero observed errors is not a universal guarantee.

### 3. Counterbalanced reviewer study

Use the same seeded and naturally occurring PR histories with native GitHub review and Resume in
counterbalanced order. A full PR diff alone is an insufficient baseline because GitHub now offers
incremental views.

Collect at least **100 eligible sessions across at least 20 reviewers and 3 repositories**. Measure:

- active completion time, not elapsed notification latency;
- issue recall and severity, including seeded high-severity issues;
- false reassurance, reviewer confidence, and full-diff fallback;
- residue files and lines, but only as a diagnostic mediator.

**Human-value gate:** no missed seeded critical/high-severity issue; preregister a recall
non-inferiority margin before data collection; and require a statistically supported reduction in
active review time versus the native baseline. If time improves while recall degrades beyond the
margin, the product fails.

### 4. Prospective product funnel

Measure the complete denominator:

```text
repository connected
  -> retrospective Audit completed with unknowns explicit
  -> actionable finding acknowledged
  -> non-blocking shadow proof active
  -> administrator requests enforcement
  -> verified final-candidate merge
```

Also report complete-audit rate, actionable finding rate, finding disposition, shadow-to-enforce
conversion, unknown/fail-closed rate, p50/p95 event-to-Check latency, repair time, provider and CI
invocations avoided, override reasons, and 28-day verified-merge retention.

**Operational gate:** at least 95% of prospectively captured, complete-evidence events produce a
Check within two minutes at p95; every failure remains visible and actionable. Do not count cases
with missing prerequisites in that success denominator—report them separately.

**Retention gate:** do not promote beyond design partners until at least three of five repositories
request enforcement after a two-week shadow period and continue producing verified merges in week
four. Freeze any later threshold before its cohort begins.

### 5. Promotion and enforcement gate

Do not recommend a production required merge Check or claim saved time until all of the following
are true:

- the live Governor gate passes across at least five consenting workflows;
- the integrity and human-value gates pass on the frozen protocol;
- same-head base movement and merge-queue behavior have explicit, tested invalidation paths;
- the gate is bound to a dedicated expected App source rather than a shared Actions identity, while
  provider semantics are validated separately;
- at least two reviewer adapters feed one policy and every override retains actor and reason;
- no unresolved factual misclassification is known;
- event-time retention, deletion, encryption, and source-processing boundaries are documented;
- a clean install reaches the first useful residue without a manually supplied SHA;
- at least three independent design-partner repositories request continued use after the study.

If reviewers do not repeatedly open Resume, if native GitHub performs equivalently, or if the
proof-carry restriction leaves most eligible cases at full review, stop or reposition the product
rather than widening claims.

## Claim boundary

Safe external wording:

> StrataDiff combines provenance-bound evidence from configured reviewers into one App-bound Check
> for the current PR head or merge-group candidate. It fails closed on missing, stale, wrong-source,
> contradictory, or unsupported evidence and exposes the exact reason for independent inspection.

Do not claim that CodeRabbit currently approves stale heads, that GitHub accepts an earlier SHA or
lacks expected-App binding, that absence from public Graphite documentation proves absence of a
private capability, or that Copilot's public-preview bug is permanent. Do not claim current
benchmark dispatch proxies are real savings, that a person read every carried byte, that carried
code is behaviorally equivalent or safe, that a proof means “bug free,” that most PRs need the
product, or that cost, reviewer time, and defect recall improve before prospective pilots supply
those results.
