# Market evidence: review continuity after a changed pull request

Evidence captured: **2026-09-06**. This is a point-in-time, auditable product-research snapshot.
Issue votes, reactions, comments, labels, and states can change. Public issue reports are
self-selected evidence of a failure mode or demand; they are not prevalence, willingness-to-pay,
retention, or market-size estimates. Vendor documentation establishes documented behavior, not
independent accuracy or adoption.

## Decision

The recurring job is not “show a nicer diff.” It is:

> After a human reviewed a pull-request state and the branch changed, recover that reviewer's exact
> usable checkpoint and show what still needs attention without asking the reviewer to reconstruct
> commit history or trust an automatic approval.

The strongest initial segment is teams that combine frequent rebases, force-pushes, or stacked pull
requests with strict review protection, multiple reviewers or ownership domains, and expensive CI.
The evidence does not support a claim that every pull request or every developer has this problem.

The recommended wedge is a **Verified Review Resume Check**: capture an immutable review receipt
when an existing human review is submitted, recompute reviewer-specific coverage after a later
push, and publish an informational Check with one **Resume review** action. It must expose current
author residue, dropped reviewed work, and changed-base context separately; every unsupported or
ambiguous case remains in review. It does not review code, restore a GitHub approval, or decide that
code is safe to merge.

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

## Product wedge: Verified Review Resume Check

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

- GitHub teams already using stacked PRs, Graphite, `gh-stack`, frequent rebases, or curated
  force-push workflows.
- Repositories with stale-review dismissal, latest-push approval, merge queues, CODEOWNERS, or
  multiple required reviewers.
- Monorepos or projects where CI/deployment reruns and specialist review round-trips are expensive.
- A DevEx/platform/security owner who can install a Check, paired with reviewers who experience the
  interruption directly.

Use `gh stratadiff audit` to qualify each repository. A low-drift repository should receive an
honest “not useful here” result rather than an installation pitch.

### Poor initial fit

- Small, append-only PRs whose native Changes since last review view is sufficient.
- Repositories with one informal reviewer and cheap CI.
- Teams asking primarily for AI bug findings, summaries, stack creation, or syntax-aware diff
  presentation.
- Environments that cannot preserve an event-time checkpoint under an acceptable source-retention
  and privacy model.

## Falsifiable evaluation plan

These are proposed go/no-go gates, not achieved results. Freeze the protocol and thresholds before
examining pilot outcomes.

### 1. Prospective correctness corpus

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

### 2. Counterbalanced reviewer study

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

### 3. Prospective product funnel

Measure the complete denominator:

```text
eligible checkpoint drift
  -> Check delivered
  -> Check viewed
  -> Resume opened
  -> later human review submitted on current head
```

Also report checkpoint recovery rate, unknown/fail-closed rate, p50/p95 event-to-Check latency,
analysis latency, network and storage use, reviewer override reasons, and 28-day repeat use among
reviewers who encounter at least a second eligible event.

**Operational gate:** at least 95% of prospectively captured, complete-evidence events produce a
Check within two minutes at p95; every failure remains visible and actionable. Do not count cases
with missing prerequisites in that success denominator—report them separately.

**Retention gate:** do not promote beyond design partners until reviewers voluntarily complete
repeat Resume loops when another eligible event occurs. Freeze a numeric repeat-use threshold only
after the first baseline cohort; changing it after inspecting outcomes invalidates the gate.

### 4. Promotion and enforcement gate

Do not enable a required merge Check or claim saved time until all of the following are true:

- the integrity and human-value gates pass on the frozen protocol;
- no unresolved factual misclassification is known;
- event-time retention, deletion, encryption, and source-processing boundaries are documented;
- a clean install reaches the first useful residue without a manually supplied SHA;
- at least three independent design-partner repositories request continued use after the study.

If reviewers do not repeatedly open Resume, if native GitHub performs equivalently, or if the
proof-carry restriction leaves most eligible cases at full review, stop or reposition the product
rather than widening claims.

## Claim boundary

Safe external wording:

> StrataDiff reconstructs a submitted human-review checkpoint after a later PR change, carries only
> relations supported by its declared deterministic policy, and shows the remaining review residue.

Do not claim that a person read every carried byte, that carried code is behaviorally equivalent or
safe, that an approval remains valid, that most PRs need the product, or that reviewer time and
defect recall improve before the prospective study supplies those results.
