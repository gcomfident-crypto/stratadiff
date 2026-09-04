# Evidence Workbench design

StrataDiff's viewer is a verification surface, not a decorated line diff. It combines familiar
review navigation with two things ordinary code-review products do not expose: the evidence for a
structural relation and explicit uncertainty when snapshots do not determine one.

## Product references

The interaction model was informed by the following public documentation, reviewed on
2026-09-04:

- [GitHub proposed-change review](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request): split/unified review and low-friction navigation.
- [GitLab merge-request changes](https://docs.gitlab.com/user/project/merge_requests/changes/): single-file focus, expandable context, and large-diff controls.
- [VS Code diff editor](https://code.visualstudio.com/updates/v1_82#_diff-editor): adaptive inline layout, moved-code comparison, and accessible navigation.
- [Difftastic](https://difftastic.wilfred.me.uk/introduction.html): syntax-aware emphasis with low visual noise.
- [SemanticDiff middle bar](https://semanticdiff.com/docs/understand-diff/middle-bar/) and [minimap](https://semanticdiff.com/docs/understand-diff/minimap/): relationship navigation and paired locations.
- [Reviewable reviews](https://docs.reviewable.io/reviews): evidence-oriented progress and next-change navigation.
- [Graphite PR page](https://graphite.com/docs/pr-page-overview): focused review modes and version-aware navigation.

The resulting layout uses a change outline on the left, a synchronized evidence canvas in the
center, and a proof inspector on the right. Narrow viewports collapse the sidebars and switch the
code layer to a unified presentation.

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
