# Evidence Workbench design

StrataDiff's viewer is a verification surface, not a decorated line diff. It combines familiar
review navigation with two things ordinary code-review products do not expose: the evidence for a
structural relation and explicit uncertainty when snapshots do not determine one.

## Product references

The interaction model was informed by the following public documentation, reviewed on
2026-09-04:

- [GitHub proposed-change review](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request) and [Files changed refresh](https://github.blog/changelog/2025-06-26-improved-pull-request-files-changed-experience-now-in-public-preview): split/unified review, a resizable file tree, and comment/error/warning indicators.
- [GitLab merge-request changes](https://docs.gitlab.com/user/project/merge_requests/changes/): single-file focus, expandable context, and large-diff controls.
- [VS Code diff editor](https://code.visualstudio.com/updates/v1_82#_diff-editor) and [accessible diff viewer](https://code.visualstudio.com/updates/v1_98#_accessibility): adaptive inline layout, moved-code comparison, collapsed-region breadcrumbs, and an `F7` accessible viewer for modified files.
- [Difftastic](https://difftastic.wilfred.me.uk/introduction.html): syntax-aware diffs and formatting-change suppression.
- [SemanticDiff middle bar](https://semanticdiff.com/docs/understand-diff/middle-bar/), [minimap](https://semanticdiff.com/docs/understand-diff/minimap/), and [moved code](https://semanticdiff.com/docs/understand-diff/moved-code/): correspondence geometry, a whole-file change overview, and paired-location navigation.
- [Reviewable files](https://docs.reviewable.io/files): per-revision review state and next-unreviewed navigation.
- [Graphite PR page](https://graphite.com/docs/pr-page-overview) and [pull-request versions](https://graphite.com/docs/pull-request-versions): file navigation, hiding reviewed changes, and explicit version bounds.

The resulting layout uses a change outline on the left, a synchronized evidence canvas in the
center, and a proof inspector on the right. Narrow viewports collapse the sidebars and switch the
code layer to a unified presentation.

The Code layer directly embeds the Apache-2.0 [`@pierre/diffs`](https://diffs.com/) 1.4.0 renderer
rather than reimplementing syntax highlighting, character-level intra-line highlighting,
synchronized split scrolling, or unchanged-context expansion. StrataDiff adds the report-specific
evidence selection and verified claim boundaries around that renderer.

| Reviewed pattern | Status | Decision in the workbench |
|---|---|---|
| Split and unified code layouts | Implemented | Both are available; narrow screens switch to unified without overwriting the desktop preference. |
| Collapsed unchanged context | Implemented | The focused view is the default and has a one-click full-file mode. Relation and ambiguity selection expands the full file before an off-screen selected row is revealed. |
| Long-line handling | Implemented | Reviewers can switch between horizontal scrolling and wrapped lines without recomputing the report. |
| Persistent file/evidence navigation | Implemented | Search, filters, pagination, and `j`/`k` update the shared selection without forcing the reviewer out of Code. |
| Relationship visualization | Implemented | The dedicated Structure layer uses a before/evidence/after middle column and exposes ambiguity instead of drawing speculative edges. |
| Minimap and resizable docked panels | Deferred | These are useful for multi-file reports but omitted from the single-pair viewer until they can preserve exact evidence navigation and keyboard access. |
| Inline editing, merge actions, AI summaries, and review approval | Excluded | This surface verifies evidence; it does not mutate code or infer claims. |

## Claim boundaries

The interface preserves the report contract rather than strengthening it through visual wording:

- Readable line highlighting is a review aid; the patch layer is the certified byte transformation.
- `model_forced` describes selection under the declared model, not historical identity.
- `equivalent_relocation` is labeled as model-equivalent and is not attributed to the author.
- An exact ambiguity shows only its serialized `possible_pairs` and joint ordering constraint.
- A symbolic abstention with `pair_claims: none` shows its covered scopes but no candidate edge.
- Universal mode says “Byte-defined structure, not AST.”
- Verification state is independent from insert/delete colors and remains visible while scrolling.

## Visual and interaction system

Coral marks deletion, teal marks insertion, purple marks model-forced structure, amber hatching
marks ambiguity, and green is reserved for successful verification. Every state also has a text
label or line style so color is never the sole signal.

The primary keyboard path is `j`/`k` for adjacent evidence, `[`/`]` for ambiguity groups,
`1`/`2`/`3` for Code/Structure/Exact bytes, `/` for search, and `?` for the shortcut reference.
Selection is shared across layers, so switching views does not discard the current evidence item.

## Local security model

The CLI verifies the generated report before binding a server. The server listens on IPv4 loopback
only, rejects unexpected Host headers, requires a cryptographically random query token for report
and source bytes, disables caching and referrers, and serves a restrictive Content Security Policy.
Assets are bundled locally; the browser makes no CDN request.
