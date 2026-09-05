# Product strategy: proof-carrying review memory and the Change Passport

Evidence captured: **2026-09-05**. This is a falsifiable product thesis, not a market-size report or
a claim that the roadmap is already implemented. Prices, install counts, stars, and vendor claims
are point-in-time observations and must be refreshed before external use.

## Decision in one sentence

**StrataDiff should become GitHub-native Human Review Memory: after every push, it should preserve
only the per-reviewer and per-ownership-domain coverage backed by deterministic evidence, route the
unproved residue to the right people, fail a required check while required coverage is missing, and
retain the result in an independently verifiable Change Passport.**

This is deliberately not another AI reviewer. AI reviewers generate more judgments. StrataDiff's
wedge is to remove noise from the reviewer's first pass only when a narrower factual claim can be
checked again by an independent verifier. It does not restore or manufacture a code-host approval;
it supplies the missing evidence gate that a branch-protection policy can require.

The user-visible outcome is not “a better diff.” It is: **after a rebase, restack, force-push, or
agent update, show exactly which parts of Alice's and Security's prior reviews still have evidence,
send each owner only their residue, and keep the merge check red until all required coverage is
current.** The product succeeds only if that loop reduces repeated reviewer work without lowering
issue recall.

### Product boundary: coverage firewall, not stale-approval bot

Rebase-safe approval is a real pain but too small a standalone category. Graphite's own
[`dismiss-stale-approvals`](https://github.com/withgraphite/dismiss-stale-approvals) README says the
request came from a relatively small number of customers and one enterprise trial. GitHub already
offers the whole-PR compromise “require approval of the most recent reviewable push,” while GitLab
can reset approvals only when a [`git patch-id`](https://git-scm.com/docs/git-patch-id) changes.
Competing on that binary switch alone would make StrataDiff a feature, not a product.

The broader, defensible job is a **review-coverage firewall**. It maintains a SHA-bound ledger for
each required reviewer and CODEOWNERS domain, maps that coverage across ordinary pushes and history
rewrites, and rejects every file or hunk whose carry cannot be proved. GitHub remains the canonical
place for conversation and approval; StrataDiff supplies the missing reviewer × ownership × change
coverage matrix. The long-term promise is:

> Never re-review unchanged code. Never inherit an unproven approval.

The current single-reviewer, file-level Action is only the first vertical slice of this contract.
Hunk-level carry, CODEOWNERS resolution, team membership, and durable multi-reviewer receipts are
not implemented yet and must not be implied by alpha results.

### The first job: resume, do not restart

The first product wedge is not generic semantic triage. It is the repeated-review loop: a reviewer
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
| Commercial review tooling has paid demand | [SemanticDiff pricing](https://semanticdiff.com/github/pricing/) displayed a $10/seat/month tier. [CodeRabbit pricing](https://www.coderabbit.ai/pricing) displayed $24/$48/$72 per developer/month annual-price points, and [Graphite pricing](https://graphite.com/pricing) displayed paid tiers around $20/$40 per user/month. CodeRabbit also reported roughly 17,000 customers and six million repositories on vendor-owned material captured during this research. | Teams pay for review workflow and automation; pricing is plausible if value is proved. | Pricing and adoption claims are vendor-reported and not independently verified. |
| History rewrites destroy useful review context | GitHub's own `gh-stack` users report that sync force-pushes [erase “changes since last view”](https://github.com/github/gh-stack/issues/354), and that a byte-identical restack [dismissed three approvals and restarted CI](https://github.com/github/gh-stack/issues/446). | Exact state can survive rewritten commit identity and avoid demonstrably redundant work. | Concrete first-party issue reports; they establish failure modes, not prevalence. |
| Reviewers cannot recover the right incremental range after a rebase | A GitHub Community request says the heavily used “changes since last review” view stops working after rebase and force-push, leaving reviewers to find the first unreviewed commit and edit a URL or use the CLI ([#141845](https://github.com/orgs/community/discussions/141845)). GitLab users separately request a reviewer-specific last-reviewed revision instead of choosing versions from memory ([#25559](https://gitlab.com/gitlab-org/gitlab/-/work_items/25559)). | The default experience must resolve a per-reviewer checkpoint automatically and open the residue in one action; asking for a SHA is a diagnostic fallback, not the product. | Two public requests across hosts; neither establishes incidence or willingness to pay. |
| Approval invalidation is broader than the reviewed delta | GitHub Community requests ask for invalidation by the final diff or tree rather than commit ancestry ([#12876](https://github.com/orgs/community/discussions/12876), 98 votes at capture) and report stacked changes causing cascades of stale approvals ([#57513](https://github.com/orgs/community/discussions/57513), 126 votes at capture). Another report says a reviewer's own suggestion can trigger renewed approval across 12 organizations ([#78039](https://github.com/orgs/community/discussions/78039)). | The product should bind review state to exact evidence and invalidate only what it cannot carry. | Public requests and reported organization experience; vote counts are point-in-time signals, not prevalence. |
| Large-MR reviewers explicitly ask for narrow invalidation | A GitLab request says rebase forces the reviewer to revisit every approved file and asks to retain identical file/block approval ([#594565](https://gitlab.com/gitlab-org/gitlab/-/issues/594565)). A separate GitLab analysis reports a 15% incidence of unwanted patch-ID changes in one 1,000+-developer, 50k-file project ([#439234](https://gitlab.com/gitlab-org/gitlab/-/issues/439234)). | Whole-review invalidation is a costly, measurable problem; exact file identity is a plausible narrower primitive. | One user request and one organization-specific analysis; external replication is required. |
| Whole-PR invalidation ignores ownership boundaries | A GitLab request reports that a new commit can invalidate every approval even when only one CODEOWNERS domain changed, forcing unrelated domain owners to review again ([#604779](https://gitlab.com/gitlab-org/gitlab/-/work_items/604779)). | Coverage must eventually be tracked per reviewer and ownership domain; a single global checkpoint is only an alpha integration. | One public feature request; it establishes the workflow failure, not its frequency. |
| Force-push approval churn creates mechanical re-review | Zephyr's [stale-approval RFC](https://github.com/zephyrproject-rtos/zephyr/issues/43701) reports that re-approval after a force-push delayed merges while reviewers often responded with a mechanical `+1`; its public [PR #41626](https://github.com/zephyrproject-rtos/zephyr/pull/41626) contains 27 force-push events and 13 approval dismissals at capture. A later [review-workflow RFC](https://github.com/zephyrproject-rtos/zephyr/issues/53566) explicitly contrasts GitHub's large-PR experience with Gerrit. | Large, multi-reviewer OSS projects provide concrete design-partner cases and replayable event histories. | Purposefully selected public evidence; it proves the workflow can be painful, not population prevalence. |
| Host history is not a durable review ledger | On 2026-09-05, both GitHub GraphQL and REST returned `null` for the commit bound to all 13 dismissed review records still visible on Zephyr [PR #41626](https://github.com/zephyrproject-rtos/zephyr/pull/41626); the two final, non-dismissed approvals still exposed their commit. | A post-hoc Action cannot always reconstruct old coverage. The GitHub App must capture a signed or content-addressed review receipt when the review event arrives and retain dismissal as a later state transition. | Point-in-time API observation on one old PR; retention behavior may vary and is not a platform guarantee. |
| GitLab treats selective approval reset as a paid workflow primitive | GitLab documents an option to remove approvals only when a new commit changes the patch ID and a separate option to remove only approvals from Code Owners whose files changed ([approval settings](https://docs.gitlab.com/user/project/merge_requests/approvals/settings/)). | Patch-aware and owner-scoped invalidation are established buyer-facing capabilities; GitHub lacks the same reviewer × path coverage primitive. | Official product behavior. Git patch-id is only reasonably stable and ignores whitespace by default, so it is not equivalent to StrataDiff's evidence contract. |
| Reviewable proves demand for persistent per-file review state | Reviewable records each reviewer's state per file and revision and carries state across rebases where it can map revisions ([file review state](https://docs.reviewable.io/files)). Its maintainer also warns that carrying line-level marks can hide unreviewed changes ([issue #414](https://github.com/Reviewable/Reviewable/issues/414#issuecomment-1611899307)). | Persistent human review memory is a real product, while conservative hunk carry remains technically differentiated and safety-sensitive. | Official documentation and maintainer statement; adoption and time-saving are not independently measured here. |
| Ownership policy has stand-alone paid demand | PullApprove sells path- and line-based approval rules and publishes a DoorDash account of use across hundreds of repositories and thousands of users ([product documentation](https://www.pullapprove.com/docs/), [pricing](https://www.pullapprove.com/pricing/)). | The buyer is DevEx/platform/security, and ownership-aware coverage can be a paid control-plane feature. | Vendor claims and pricing, not independently audited usage. |
| AI re-review also forgets dispositions | GitHub's [Copilot code-review documentation](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/request-a-code-review/use-code-review) says re-review may repeat dismissed or downvoted comments, while a [community request](https://github.com/orgs/community/discussions/190754) describes repeatedly re-explaining the same rejected suggestion. | A later ledger should bind findings and human dispositions to exact evidence identity instead of rerunning a stateless reviewer. | Official limitation plus user report; this is a follow-on job, not proof that the current checkpoint implementation solves it. |
| Review state across pushes is an established job | [GitHub documents](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/reviewing-proposed-changes-in-a-pull-request#marking-a-file-as-viewed) that a viewed file is unmarked when it changes, while [GitLab exposes diff versions](https://docs.gitlab.com/user/project/merge_requests/versions/) specifically for merge requests with many or sequential changes. A [public editor request](https://github.com/wandersoncferreira/code-review/issues/146) describes large PR review spanning multiple sessions and asks to preserve file or hunk state. | Reviewers already expect incremental review and explicit invalidation. | Product documentation plus a user report; neither measures time saved. |
| Naive last-review diffs absorb base noise | VS Code's GitHub extension has tracked incremental review since [#363](https://github.com/microsoft/vscode-pull-request-github/issues/363). Follow-up reports show merges from the base branch introducing unrelated files into the view ([#4510](https://github.com/microsoft/vscode-pull-request-github/issues/4510), [#5455](https://github.com/microsoft/vscode-pull-request-github/issues/5455), [#6281](https://github.com/microsoft/vscode-pull-request-github/issues/6281)). | A useful residue must compare PR-relative changes and exclude upstream-only files. | Public issue reports establish concrete failure modes, not their frequency. |
| Rebase-aware review has capable incumbents | [Reviewable documents](https://docs.reviewable.io/files#file-review-state) matching a file against a prior rebased revision, and Git provides [`range-diff`](https://git-scm.com/docs/git-range-diff) for comparing two versions of a patch series. | Exact Review Resume must compete on portable evidence and deterministic invalidation, not claim invention of incremental review. | Capability documentation, not comparative accuracy or adoption evidence. |
| Standalone re-review UI is increasingly crowded | [Pyor](https://pyor.review/) markets agent-era PR grouping and its [interdiff guidance](https://pyor.review/blog/re-reviewing-pull-requests-interdiff) directly addresses re-review after force-push and rebase. | Another viewer is not a sufficient wedge. Distribution and value must come from a required coverage check and reproducible artifact. | Vendor capability and positioning; no independent adoption or performance evidence was found. |
| Stacked workflows trade safety for flow | Graphite's [stack-review guidance](https://graphite.com/docs/best-practices-for-reviewing-stacks) recommends disabling both stale-approval dismissal and latest-push approval requirements for smoother stacks. | A deterministic residue check can occupy the gap between repeatedly clearing all review state and trusting every rewritten stack. | Official workflow advice; it does not prove demand for StrataDiff. |
| Mature review systems already copy votes under explicit change rules | Gerrit's [label copy conditions](https://gerrit-review.googlesource.com/Documentation/config-labels.html#label_copyCondition) distinguish `NO_CHANGE`, `NO_CODE_CHANGE`, `TRIVIAL_REBASE`, and `REWORK`, and describe copied votes as reducing turnaround time. | Evidence-based coverage carry is an established policy primitive; StrataDiff should make its narrower byte-level rule portable and independently checkable. | Official capability documentation; it does not show that Gerrit users need a separate product. |

The strongest market inference is therefore limited: review attention is scarce, existing products
monetize review workflow, and probabilistic review has a trust gap. We have not yet proved that
teams will adopt a proof-carrying residue layer or that it saves time.

## Ideal customer profile and jobs to be done

### Primary ICP hypothesis

The first design partners should be GitHub Cloud or GHES teams, typically 50–500+ engineers, that
have all of these characteristics:

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

> When a large change mixes mechanical edits with behavior-changing work, show which byte or
> parser/model predicates were preserved and place everything else in a review-first residue, so I
> can spend judgment on the risky part without trusting a black-box summary and can re-check the
> evidence after a push.

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
| GitHub, GitLab, Reviewable | Canonical conversation, permissions, approvals, viewed state, file navigation, and revision workflow. GitHub's [review API](https://docs.github.com/en/rest/pulls/reviews?apiVersion=2022-11-28#list-reviews-for-a-pull-request) exposes the commit attached to each review. | Integrate with these systems. Resolve reviewer checkpoints from host state, then add a portable evidence check and exact-head residue gate. |
| Graphite and stacked-PR tools | Make changes reviewable by splitting, stacking, routing, and tracking PRs. | Stacking changes the presentation and dependency graph; StrataDiff analyzes an arbitrary existing range. The approaches are complementary. |
| Copilot, CodeRabbit, Graphite AI, and other AI reviewers | Suggest likely bugs, summaries, and fixes across repository context. | Run these tools on the residue if useful. A suggestion is not a verified predicate, so it cannot enter the evidence-backed lane without deterministic support. |
| SemanticDiff, Difftastic, and Pyor | Provide substantially better structural presentation, moved-code navigation, grouping, or re-review views than line diff. | The overlap is real. Differentiate on a required coverage gate, portable Change Passport, independent replay, explicit ambiguity/abstention, and cross-push evidence lineage—not on visual syntax awareness alone. |
| RefactoringMiner / ASTDiff | Strong Java refactoring detection and mapping; its [PurityChecker](https://github.com/tsantalis/RefactoringMiner/blob/master/documentation/purity.md) evaluates nine documented refactoring kinds. | Reuse or ingest stronger language-specific evidence. Do not claim that the current multi-language CST matcher supersedes compiler-aware Java analysis. |
| Moderne / OpenRewrite | Deterministic source recipes with [recipe tests](https://docs.openrewrite.org/authoring-recipes/recipe-testing) and knowledge of the transformation that was requested. | For recipe-produced changes, producer provenance can be stronger than post-hoc inference. Import the recipe attestation; focus StrataDiff on vendor-neutral verification of changes from any source. |
| Static analysis and security scanners | Find known bug and vulnerability classes. | These tools answer “what may be wrong?” StrataDiff answers “what factual transformation can be replayed or checked?” Neither replaces the other. |

The defensible product loop is: **arbitrary Git range → conservative evidence → portable passport →
cross-push review ledger**. Any competitor can copy a four-lane summary; the verifier, benchmark,
artifact compatibility, and accumulated lineage must provide the trust advantage.

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
- two independent CODEOWNERS domains: changing one domain invalidates that domain's coverage while
  leaving the other domain's proven coverage intact;
- unavailable, malformed, ambiguous, or provider-unverifiable checkpoints: the check fails closed
  with an actionable diagnostic and never substitutes another revision.

## Roadmap

### P0: prove the wedge

1. Ship a local `stratadiff review BASE [HEAD]` path that resolves one unambiguous merge base and
   handles multi-file Git identity, unique exact-object relocations, modes, unsupported files, and
   resource limits. Keep edited or ambiguous rename/copy inference out until it has a separately
   bounded evidence model.
2. Emit deterministic JSON and concise Markdown with structural-delta, parser-model-matched,
   same-Git-object, and unverified evidence classes plus a separate attention priority. Keep every
   class human-first until a narrower policy passes adversarial and reviewer-recall gates.
3. Harden Exact Review Resume from a caller-attested checkpoint. Preserve the exact-identity fast
   path and the unique same-path, non-interacting four-way replay path across base drift. Expand the
   adversarial corpus before supporting more file kinds or interaction patterns.
4. Define and validate `review-v2` as the first self-contained Change Passport envelope, including
   commit/blob provenance, attached or content-addressed single-file reports, engine version,
   non-claims, and independent offline verification instructions. Keep `review-v1` labeled as a
   producer-attested focus summary.
5. Harden the minimal-permission GitHub Action into an approval-coverage check. The alpha now
   resolves one explicitly named reviewer's latest non-dismissed human decision and can fail on a
   non-empty residue. Next add provider-bound review receipts, a reviewer × CODEOWNERS × file matrix,
   prebuilt releases, and a Check Run surface that routes each residue to its owner. Source remains
   inside the runner; PR comments are opt-in to avoid bot noise. Hunk carry follows only after the
   file-level policy has independent oracles and adversarial false-carry tests.
6. Build and publish the three-track benchmark plus a multi-revision ResumeBench, including
   adversarial invalidation cases and dataset cards. Run them on every change to policy.
7. Dogfood on public repositories and recruit design partners from the primary ICP. Record lane
   overrides and reasons before adding more classifiers.
8. Make the public demo answer three questions in under a minute: what changed since my checkpoint,
   what was proved, and how can I reproduce it?
9. Make installation a GitHub-native, zero-UI-migration path: a free open-source Action first, then
   an optional Marketplace App for event ingestion, durable ledgers, policy, and audit. Do not make
   reviewers adopt a separate conversation interface.

P0 exits only when passport verification is stable, no known factual misstatement remains, and a
blinded pilot shows useful reviewer-time reduction without lower issue recall. Passing unit tests or
the existing AST benchmark alone is insufficient.

### P1: compound trust and distribution

1. Persist per-reviewer checkpoints, partial-file state, and finding dispositions across pushes.
   Carry state only when exact Git identity or a separately verified relation permits it; invalidate
   everything else visibly and never silently re-anchor a comment.
2. Add an optional GitHub App with read-only repository/check permissions, organization policy,
   retention controls, and audit logs. Keep the CLI, schema, and verifier open and offline-capable.
3. Add a multi-file Evidence Workbench centered on residue navigation, overrides, and passport
   verification rather than reproducing GitHub's conversation UI.
4. Import producer evidence from OpenRewrite and language-specific evidence from tools such as
   RefactoringMiner. Preserve the source and strength of every claim.
5. Add incremental analysis, caching bound to blob IDs, cancellation, queueing, and large-PR load
   tests. Publish the eligibility and failure rates instead of hiding unsupported cases.
6. Add GitLab and local pre-review adapters through the same passport schema.
7. Offer a paid control plane only for team coordination: durable ledgers, SSO/RBAC, policy,
   organization analytics, and support. Do not paywall offline verification of a passport.

## Distribution and promotion flywheel

The artifact itself should distribute the product:

1. A PR check exposes a small, factual residue summary rather than posting dozens of comments.
2. A reviewer opens one evidence item, verifies the exact claim, and sees the remaining residue.
3. The attached passport survives the code-host UI and can be reproduced locally.
4. Opt-in aggregate results become public, pinned case studies and benchmark improvements.
5. More real failure cases improve abstention and the benchmark, which increases trust and earns
   more installations.
6. Refactoring and migration tools emit compatible provenance, increasing coverage without
   weakening the claim boundary.

Initial channels should be open-source maintainers, DevEx/platform communities, migration tooling,
and transparent engineering write-ups built around reproducible before/after passports. Avoid
generic “AI reviews your PR” positioning, paid vanity benchmarks, and unsolicited PR-comment spam.

The initial search and Marketplace language should name the pain rather than the mechanism:
`GitHub smart stale approvals`, `review after rebase`, `CODEOWNERS approval after force push`, and
`Graphite restack approvals`. Public case cards should show the complete accounting—for example,
“13 approvals dismissed; N files retained evidence; M files and K hunks require re-review”—with a
downloadable passport instead of an unsupported percentage claim.

The memorable outcome message is:

> **Keep review coverage through rebases—without trusting rebases.**

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

The next milestone is not “GitHub parity.” It is one end-to-end proof:

```text
large PR at reviewed checkpoint R
  -> rewritten, rebased, or incrementally updated head H
  -> exact identity, then strict four-way replay where eligible
  -> conflicts and ambiguities fail closed
  -> upstream-only files excluded from the PR residue
  -> exact-head gate publishes the remaining review queue
  -> reviewer study measures time and issue recall
  -> portable evidence still verifies after download
```

If that loop produces measurable value, build the ledger and integrations. If it does not, the
stop conditions above should force a narrower verifier/attestation product instead of a larger but
unproven review platform.
