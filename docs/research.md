# Structured code differencing and review-memory survey

Engine survey date: 2026-09-05. Review-workflow competitor snapshot: 2026-09-06. This document
distinguishes mapping accuracy, edit-script size, human-readable presentation, persistent review
state, approval policy, and replayability; they are different objectives. Prices and product
behavior below are point-in-time vendor or platform documentation, not independent adoption or
accuracy evidence, and must be refreshed before external use.

## Executive finding

No snapshot-only differencer can guarantee the developer's true edit history. Two identical blocks
can be exchanged, independently deleted and recreated, or left untouched while producing the same
before and after snapshots. More generally, semantic program equivalence is undecidable, and tree
edit distance with unrestricted moves has hard optimization cases.

The strongest practical direction is therefore evidence-bearing and abstention-aware:

- make the byte transformation exactly replayable;
- verify every equality predicate independently;
- separate structural equivalence from historical identity;
- preserve ties rather than select an arbitrary AST mapping;
- add language semantic evidence incrementally.

## Review memory and base drift

Incremental review is established product territory. GitHub, GitLab, Gerrit, Graphite, and
Reviewable already preserve review state or show changes across revisions, and Git provides
[`range-diff`](https://git-scm.com/docs/git-range-diff) for patch-series comparison. The open problem
for StrataDiff is narrower: carry human review state only when a host-neutral proof survives push,
force-push, rebase, restack, or base drift, then fail closed on everything else.

Public reports captured on 2026-09-05 show why a direct checkpoint-to-head diff is insufficient:

- GitHub users ask for approval invalidation based on the final diff or tree rather than commit
  ancestry ([discussion #12876](https://github.com/orgs/community/discussions/12876)) and report
  cascades of stale approvals in stacked changes
  ([discussion #57513](https://github.com/orgs/community/discussions/57513)).
- A GitHub enterprise user reports that applying a reviewer's own suggestion can force another
  approval across 12 organizations
  ([discussion #78039](https://github.com/orgs/community/discussions/78039)).
- The VS Code GitHub extension's incremental-review request dates to 2018
  ([issue #363](https://github.com/microsoft/vscode-pull-request-github/issues/363)). Later reports
  show base-branch merges adding unrelated files to the "changes since review" view
  ([#4510](https://github.com/microsoft/vscode-pull-request-github/issues/4510),
  [#5455](https://github.com/microsoft/vscode-pull-request-github/issues/5455), and
  [#6281](https://github.com/microsoft/vscode-pull-request-github/issues/6281)).
- GitLab [issue #439234](https://gitlab.com/gitlab-org/gitlab/-/issues/439234) reports unwanted
  patch-ID invalidation for about 15% of merge-from-parent events in one organization with more
  than 1,000 developers and 50,000 files.
- GitHub's own `gh-stack` users report that a sync can erase changes-since-view
  ([#354](https://github.com/github/gh-stack/issues/354)) and that a byte-identical restack can
  dismiss three approvals and restart CI ([#446](https://github.com/github/gh-stack/issues/446)).
- Graphite's [stack-review guidance](https://graphite.com/docs/best-practices-for-reviewing-stacks)
  recommends disabling stale-approval dismissal and latest-push approval requirements for smoother
  stacks. This makes the unresolved tradeoff explicit: avoid repeated work or retain a strict gate.

### Documented capability snapshot

The following comparison uses official product or platform documentation captured on 2026-09-06.
“Not documented” means no claim was found in the cited material; it does not prove that an internal,
preview, or later capability is absent.

| System | Documented review-memory or re-review behavior | Rewrite and approval boundary | Activation or commercial friction at capture |
|---|---|---|---|
| GitHub | Files can be marked Viewed and are unmarked when that file changes ([reviewing proposed changes](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/reviewing-proposed-changes-in-a-pull-request#marking-a-file-as-viewed)). | GitHub can dismiss stale approvals when its recorded PR diff changes, including through base movement, or require approval of the most recent reviewable push ([protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)). These are whole-review merge controls, not per-change carry certificates. | Native review requires no third-party installation. Plan and repository visibility affect availability of protected-branch features; no pricing inference is needed for the technical comparison. |
| GitLab | Each push creates one retained MR diff version; users can compare prior versions. Per-user Viewed files remain hidden until their contents change ([versions](https://docs.gitlab.com/user/project/merge_requests/versions/), [Viewed files](https://docs.gitlab.com/user/project/merge_requests/changes/#mark-files-as-viewed)). | Approval reset evaluates `git patch-id` so rebases or target merges that preserve the patch can avoid unnecessary reset; projects can instead retain all approvals or remove only affected Code Owner approvals ([approval settings](https://docs.gitlab.com/user/project/merge_requests/approvals/settings/)). Patch ID is described by GitLab as reasonably stable; it is not reviewer × change byte provenance. | Diff versions and Viewed state are documented across Free, Premium, and Ultimate. Required approval rules and Code Owner enforcement have separate tier/configuration requirements. |
| Gerrit | Changes retain patch sets, reviewers can compare patch set A to B, and private reviewed flags are keyed by patch set, file, and user ([review UI](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#patch-sets), [reviewed flags](https://gerrit-review.googlesource.com/Documentation/config-accounts.html#reviewed-flags)). | Gerrit maps and colors rebase edits, can omit files changed only by rebase, and copies votes when an administrator-defined `copyCondition` matches change kinds such as `NO_CHANGE`, `NO_CODE_CHANGE`, or `TRIVIAL_REBASE` ([rebase edits](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#normal-and-rebase-edits), [copy conditions](https://gerrit-review.googlesource.com/Documentation/config-labels.html#label_copyCondition)). Its documentation also shows a [hazardous stacked squash](https://gerrit-review.googlesource.com/Documentation/user-review-ui.html#hazardous-rebases) whose A-to-B diff is empty while parent content enters implicitly. | Gerrit is open source, but adopting it means operating a review server; the official installation path requires Java, a WAR, site initialization, and daemon operations ([installation](https://gerrit-review.googlesource.com/Documentation/install.html)). |
| Reviewable | Tracks reviewed state for each file at each revision for each reviewer, exposes that reviewer's last-reviewed revision to latest, retains immutable force-pushed revisions, and heuristically matches rebased commits ([files](https://docs.reviewable.io/files)). | This is the closest persistent review-state incumbent, but the cited documentation does not claim a byte-level, independently replayable carry certificate for a human checkpoint. | GitHub registration uses OAuth; private repository access requires the broad `repo` scope, and organization OAuth policy can block it ([registration](https://docs.reviewable.io/registration)). Team and Business list prices were $8 and $16 per contributor per month ([pricing](https://www.reviewable.io/pricing/)). |
| Graphite | `gt submit` creates PR versions; after a user reviews and the PR changes, Hide reviewed changes compares that user's last-reviewed version with the latest ([PR versions](https://graphite.com/docs/pull-request-versions)). It also provides native stack navigation and restacking. | Version interdiff and stack navigation are documented; reviewer × file proof and safe carry across rewritten identity are not. Graphite's own [stack-review guidance](https://graphite.com/docs/best-practices-for-reviewing-stacks) recommends disabling GitHub stale-approval dismissal and latest-push approval requirements for smoother stacks. | New organization setup requires its GitHub App and organization-owner approval, plus personal authorization; the CLI has a separate install/activation path ([App authentication](https://graphite.com/docs/authenticate-with-github-app), [CLI install](https://graphite.com/docs/install-the-cli)). Starter and Team were $20 and $40 per seat per month billed annually ([billing](https://graphite.com/docs/billing-plans)). |
| CodeRabbit | A new PR receives full analysis; later pushes default to incremental reviews focused on commits added since its previous review. Automatic incremental review defaults on, pauses after five reviewed commits by default, and manual commands choose incremental or full review ([review overview](https://docs.coderabbit.ai/overview/pull-request-review), [automatic review](https://docs.coderabbit.ai/configuration/auto-review)). | This is continuity of the bot's analysis, not evidence that a named human review still covers unchanged bytes. The cited material does not document dropped-reviewed-edit or four-snapshot base-drift proof. | GitHub organization installation requires an organization owner and repository authorization ([GitHub setup](https://docs.coderabbit.ai/platforms/github-com)). Essentials was $24 annually/$30 monthly, Team $48/$60, Advanced $90 monthly, with rolling per-developer review limits ([plans](https://docs.coderabbit.ai/management/plans)). |
| GitHub Copilot code review | Automatic review can optionally run on every new push; otherwise later changes require a manual re-request. GitHub warns a re-review may repeat comments already resolved or downvoted ([usage](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/request-a-code-review/use-code-review)). | Reviews are Comments by default. Copilot approvals are a public preview that can satisfy required approvals only when explicitly enabled; a later commit dismisses that Copilot approval. Neither behavior proves continuity of an earlier human review. | Organization policy must enable code review. Reviews consume both AI credits and GitHub Actions minutes; current plan and credit prices are documented separately ([concept](https://docs.github.com/en/copilot/concepts/agents/code-review), [plans](https://docs.github.com/en/copilot/get-started/plans)). |

This snapshot rules out two broad positions. StrataDiff is not a generic interdiff: Reviewable,
Graphite, GitLab, and Gerrit already retain revisions or show last-reviewed-to-latest changes, and
Gerrit already separates many rebase edits. It is also not another AI reviewer: CodeRabbit and
Copilot already regenerate findings on later pushes. The remaining testable boundary is narrower:
start from an exact human checkpoint; distinguish current author residue, dropped reviewed edits,
and old-base-to-current-base influx; carry only a named deterministic relation; expose all
unsupported cases; and let another implementation verify the evidence offline.

### Public demand signals and their limits

In addition to the GitHub and `gh-stack` reports above, the following GitLab work items were
captured on 2026-09-06:

- [#25559](https://gitlab.com/gitlab-org/gitlab/-/work_items/25559), opened in 2018, asks GitLab to
  remember each reviewer's last-reviewed version rather than requiring manual timestamp/version
  matching; it displayed 69 upvotes and 14 notes at capture.
- [#241509](https://gitlab.com/gitlab-org/gitlab/-/work_items/241509) reports that a force-push can
  remove useful comparison context; it displayed 9 upvotes at capture.
- [#442454](https://gitlab.com/gitlab-org/gitlab/-/work_items/442454), closed at capture, reports
  target-branch noise in rebase comparison and proposes a `range-diff`-style view.

These are self-selected requests, not a prevalence sample, revenue evidence, or a promise that the
platforms will not close the gaps. They support testing rewrite-heavy reviewers as an initial
segment; they do not support claiming that most PRs need StrataDiff. The repository's completed
Review Churn Census remains the bounded incidence evidence for the selected public panel.

### Resume activation implication

The competitor snapshot also shows that capability and activation are separate. Native hosts have
zero incremental install cost; Reviewable needs broad OAuth access for private-repository use,
Graphite and CodeRabbit require organization-owner App approval for organization deployment, and
Gerrit requires operating a different review system. A local, no-admin entry is valuable only if it
is genuinely easier than those paths.

At this repository snapshot, [`resume`](../src/resume.rs) contains canonical PR-URL repository
inference and an isolated temporary bare repository. Its exact-SHA fetch asks Git for named commits,
but Git may transfer their reachable object closure; the current implementation caps wall time and
captured subprocess output, not network bytes or disk usage. That source-level path and its
repository-local tests are necessary but do not establish distributable activation: the
[release procedure](releasing.md) explicitly distinguishes release infrastructure from an actually
published, installed artifact. The acceptance test for this secondary human surface is therefore:

```text
fresh environment with Git + authenticated gh
  -> install a verified prebuilt StrataDiff release
  -> run stratadiff resume with one canonical GitHub PR URL
  -> infer host/repository/reviewer with no checkout, -R, or SHA
  -> materialize the named commits in an isolated temporary bare repository
  -> open the first verified residue, or fail closed with an actionable reason
```

Measure activation success and time to first residue before adding more classifiers or a hosted
control plane. Record transferred and on-disk bytes, and establish an enforceable resource policy
before making any bounded-download or bounded-storage claim. A source build, pre-cloned repository,
hand-selected checkpoint, or organization administrator install does not satisfy this activation
contract.

StrataDiff therefore treats the current PR range, not the raw checkpoint-to-head snapshot delta, as
the source of the residue after a base change. It tries complete Git identity first. A unique
same-path regular-file modification can then carry only if the reviewed patch and upstream patch
have no touching or overlapping byte edits and both translated replay orders reproduce the current
blob exactly. Unsupported modes, NUL-containing content, missing or oversized blobs, conflicts,
and ambiguous candidates remain in review. Upstream-only files are excluded from the residue.

This four-way check establishes a byte-level commuting relation among the old base, reviewed file,
new base, and current file. It does not establish semantic equivalence, cross-file safety, or that
the checkpoint was reviewed. Reviewable's documented
[file review state](https://docs.reviewable.io/files#file-review-state) provides a capable product
comparison; StrataDiff's distinct claim is portable proof and explicit fail-closed behavior, not
the invention of incremental review. [Pyor](https://pyor.review/) also markets an agent-era review
surface and [interdiff workflow](https://pyor.review/blog/re-reviewing-pull-requests-interdiff), so a
standalone “show me less diff” UI is not a sufficient product wedge. The defensible integration is
a required review-coverage check backed by the portable evidence.

## Leading systems

| System | Primary objective and approach | Strong point | Important limitation | License |
|---|---|---|---|---|
| [RefactoringMiner / ASTDiff](https://github.com/tsantalis/RefactoringMiner) | Refactoring-aware, multi-stage AST matching using declarations, references, detected refactorings, and local GumTree matching | Current public accuracy leader on the Java DiffBenchmark; supports cross-file and multi-mapping cases | Published accuracy evidence is primarily Java; still not perfect, especially around repeated statements and difficult nested changes | MIT |
| [GumTree 4](https://github.com/GumTreeDiff/gumtree) | Hash-based top-down anchors followed by bottom-up recovery and edit-script generation | Fast, broadly reused baseline; GumTree Simple greatly improves speed and script size | Greedy one-to-one mapping, duplicate-code ambiguity, weak semantic roles, and short-script bias | LGPL-3.0 |
| [GumTree-Spoon](https://github.com/SpoonLabs/gumtree-spoon-ast-diff) | GumTree over Spoon's higher-level Java model | Rich Java model and convenient analysis API | Java-only and inherits matcher ambiguity; its documentation explicitly notes that an oracle may not be unique | Apache-2.0 |
| [Difftastic](https://github.com/Wilfred/difftastic) | Finds a minimum-cost path over syntax-tree positions using Dijkstra | Excellent human-facing structural display and broad Tree-sitter language coverage | Optimizes its display cost, not developer intent; no applicable patch or consumable mapping certificate | MIT |
| [SemanticDiff](https://semanticdiff.com/docs/what-is-semanticdiff/) | Proprietary language-specific parsing and invariance rules | Strong review UI and useful suppression of non-semantic syntax | Closed engine, no public accuracy or speed benchmark, text fallback on parse failure | Proprietary |
| [diffsitter](https://github.com/afnanenayet/diffsitter) | Extracts Tree-sitter leaves, removes whitespace, and applies Myers | Small, understandable syntax-aware token diff | Tree hierarchy does not drive matching; project describes itself as not production-ready | MIT |
| [Graphtage](https://github.com/trailofbits/graphtage) | Typed IR, ordered edit distance, and bipartite matching for unordered structures | Principled optimal-cost diff for JSON/XML/YAML/TOML and related data | Its guarantee is relative to its cost model; unordered matching can be expensive and it is not a source-code semantic differ | LGPL-3.0 |
| [Tree-sitter](https://github.com/tree-sitter/tree-sitter) | Incremental concrete syntax parsing with changed-range support | Fast, lossless-enough frontend with a large grammar ecosystem | A parser, not a node matcher; changed ranges do not establish move, rename, or identity | MIT |

Historical baselines include
[ChangeDistiller](https://doi.org/10.1109/TSE.2007.70731),
[MTDIFF](https://doi.org/10.1145/2970276.2970315), and
[IJM](https://doi.org/10.1109/ICSME.2018.00036). They introduced useful change taxonomies,
move-oriented recovery, and Java-name-aware partitioning, but are no longer the best maintained
general foundation.

## Best public accuracy evidence

The 2025 RefactoringMiner AST-diff study and
[DiffBenchmark](https://github.com/pouryafard75/DiffBenchmark) provide the strongest directly
annotated comparison found in this survey. DiffBenchmark contains 800 Defects4J commits and 188
refactoring commits. Reported fine-grained overall precision/recall are:

| Matcher | Precision | Recall |
|---|---:|---:|
| RefactoringMiner 3.0 ASTDiff | 99.7% | 99.3% |
| GumTree Simple | 95.2% | 90.0% |

RefactoringMiner reports a perfect-diff rate of 82.9% overall and 70.2% on the refactoring subset,
which is excellent but materially below 100%. On Defects4J, its reported median time is 22.5 ms,
compared with 8.75 ms for GumTree Simple. These figures show the practical accuracy/latency tradeoff
and also why a system should expose abstention instead of hiding the remaining errors.

Sources:

- Alikhanifard and Tsantalis, “Refactoring-aware Abstract Syntax Tree Differencing,” TOSEM,
  [DOI 10.1145/3696002](https://doi.org/10.1145/3696002),
  [open manuscript](https://users.encs.concordia.ca/~nikolaos/publications/TOSEM_2024.pdf).
- Frick et al., “GumTree Simple: A Fine-grained, Accurate and Scalable Source Code Differencing
  Approach,” ICSE 2024,
  [DOI 10.1145/3597503.3639148](https://doi.org/10.1145/3597503.3639148),
  [open manuscript](https://hal.science/hal-04855170v1/file/GumTree_simple__fine_grained__accurate_and_scalable_source_differencing.pdf).
- Fan et al., “A Differential Testing Approach for Evaluating Abstract Syntax Tree Mapping
  Algorithms,” ICSE 2021,
  [DOI 10.1109/ICSE43902.2021.00108](https://doi.org/10.1109/ICSE43902.2021.00108),
  [preprint](https://arxiv.org/abs/2103.00141).

GumTree Simple reports a 50 to 281x matching-stage speedup over the older `opt-1000` configuration in
its evaluated settings and substantially shorter scripts. That makes it the speed baseline, but
shorter edit scripts are not evidence that every selected mapping reflects the true change.

## Measured StrataDiff result

The provenance-complete v6 run, produced by StrataDiff 0.2.0 on DiffBenchmark's fixed 285-case
intra-file literature subset, is described in the
[benchmark notes](benchmarks.md#diffbenchmark-literature-subset-result), with the complete case-level data in the
[official evaluation report](../benchmarks/diffbenchmark-literature-evaluation-v6.json). Of the 285
selected cases, 283 were evaluated, independently verified, and replayed byte for byte. One
digest-pinned malformed oracle and one digest-pinned malformed source were classified separately;
there were no unexpected case errors, and `benchmarkComplete` is `true`.

Within this run's fixed scorable adapter universe, program-element relations achieved 99.993%
micro precision, 93.600% micro recall, and 96.691% micro F1. Fine mappings achieved 99.948% micro
precision, 92.559% micro recall, and 96.112% micro F1. The adapter made 15,499 of 15,680 raw
program-element relations and 143,454 of 145,435 raw fine-mapping relations scorable. It excluded
the remaining 181 and 1,981 oracle relations, respectively, and recorded predictions outside that
universe as unscored rather than correct or incorrect. Multi-relation recall remained limited:
0/22 for program elements and 22/2,256 for fine mappings, with no ambiguity candidate covering a
scorable gold relation.

For scoring in the v6 evaluation, the adapter flattens only explicit `possible_pairs` into an
edge-union coverage view; that union is not a jointly selectable mapping. Symbolic abstention
scopes make no pair claims and contribute no candidates.

These measurements characterize StrataDiff 0.2.0, this adapter, and this protocol on the fixed
literature subset. They are not directly comparable with published full-corpus results or results
produced under a different protocol, including the RefactoringMiner and GumTree figures above.
Differences in corpus coverage, node taxonomy, adapter exclusions, relation categories, and scoring
denominators preclude a valid head-to-head ranking or improvement claim from these numbers alone.

## Design lessons adopted by StrataDiff

1. **Use Tree-sitter as a frontend, not an oracle.** Parser success and grammar identity are part of
   the report. Syntax errors fail explicitly in conservative mode.
2. **Steal GumTree Simple's fast path, not its forced commitment.** Unique identical Merkle subtrees
   and local child alignment are cheap anchors; duplicate buckets remain ambiguous.
3. **Adopt RefactoringMiner's hierarchy.** Future adapters will solve repository, file, declaration,
   statement, and expression levels separately, then feed verified refactoring evidence downward.
4. **Use Graphtage-style exact local optimization selectively.** Small ambiguity components can use
   ordered dynamic programming or min-cost matching; large repeated regions stay symbolic.
5. **Treat Difftastic and SemanticDiff as presentation references.** A useful terminal/UI view can
   suppress trivia, but the machine report must retain raw bytes and uncertainty.
6. **Add explicit one-to-many relations in a future report model.** Verified report-v3 relations
   remain one-to-one; extract, inline, copy, split, and merge are not yet represented.
7. **Keep language semantics pluggable.** Exact byte replay is universal. CST structure exists only
   for loaded native grammars; the Universal mode is a byte-defined line/token-run tree. Bindings,
   overload resolution, macro expansion, and typed equivalence require compiler-grade language
   adapters.

## What “100%” means here

For every report it accepts, StrataDiff's contract is exact replay and independently rechecked
serialized predicates. This is a verifier contract, not empirical proof that every possible input
or parser implementation is flawless. StrataDiff does not claim 100% historical-identity accuracy,
semantic equivalence, correspondence recall, or canonical/minimal edit attribution. A low-coverage
result with explicit ambiguity is preferable to a plausible-looking false move. Accuracy reports
therefore publish coverage and abstention beside precision and recall.

Tree-sitter positions in the report use zero-based rows and UTF-8 byte columns. Universal positions
use zero-based rows and raw-byte columns. Consumers needing Unicode code-point or UTF-16 editor
coordinates must convert them explicitly.
