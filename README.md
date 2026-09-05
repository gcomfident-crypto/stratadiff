# StrataDiff

**Resume the review. Verify what changed.**

StrataDiff is a local, proof-carrying memory layer for human review of large refactors, codemods,
and AI-written changes. Give it the last complete PR snapshot you reviewed and the current head. It
first carries exact Git change identities. If the merge base changed, a unique same-path regular
file modification may also carry when a strict four-way byte replay proves that the reviewed edit
and the upstream edit do not interact. Everything else becomes `needs_review_now`. This is a
review gate with inspectable evidence, not another generic "changes since last review" view.

The checkpoint policy is built on an evidence-carrying single-file differ whose report separates
three questions that traditional AST diff tools often mix together:

1. What byte transformation turns the old file into the new file?
2. Which structural predicates can be checked directly?
3. Which node correspondences are forced by the declared model, merely suggested, or ambiguous?

The first question is answered losslessly. The second is re-derived by the matcher-free verifier
crate used by `stratadiff verify`. The third never silently turns a heuristic score into a
historical fact.

> **Project status:** research alpha. Exact Review Resume, strict base-drift replay, the repository
> review-focus command, patch reconstruction certificates, conservative syntax anchors, bounded all-optima child
> alignment, duplicate ambiguity handling, deterministic JSON reports, a resource-bounded
> matcher-free verifier crate, native Tree-sitter adapters, and an explicit Universal byte mode work
> now. The alpha Action can resolve an explicitly named reviewer's latest completed GitHub review
> and optionally fail a required check while any current PR file remains in the residue. A
> provenance-complete DiffBenchmark literature-subset evaluation is published below.
> Binding-aware rename proofs remain on the roadmap.

## Why another code diff?

Line diff is exact but structurally coarse. GumTree-style matching is useful but must choose a
single mapping even when multiple histories explain the same two snapshots. That creates false
moves, false updates, and unstable output around repeated code.

StrataDiff makes uncertainty part of the data model:

- `predicate` says what is observable: `byte_equal`, `syntax_equal`, or `shape_equal`.
- `correspondence` says how a pair was selected. The current engine emits `model_forced` pairs;
  `suggested` remains reserved in the v3 data model for future explicitly evidenced rules.
- `ambiguities` encode coupled ordered choices as explicit pair constraints. Repeated or oversized
  regions carry `pair_claims: none`, so endpoint sets can never be mistaken for a Cartesian product.
- `patch` and `certificate` rebuild and hash-check the target byte for byte.

Two snapshots cannot reveal whether identical blocks were swapped, deleted and pasted, or left
untouched. No snapshot-only algorithm can be 100% correct about that hidden history. StrataDiff
therefore makes a narrower contract for each report accepted by its verifier: every serialized
predicate is rechecked, and applying the patch reproduces the supplied target bytes exactly. This
is not a claim of perfect historical identity, semantic equivalence, complete correspondence, or a
canonical/minimal edit script. The matcher abstains where identity is not observable.

## Evaluated result

The checked-in v6 evaluation was produced by StrataDiff 0.2.0 over all 285 cases in
DiffBenchmark's pinned Java literature subset. It evaluated, matcher-free verified, and
byte-for-byte replayed all 283 well-formed cases. The two remaining inputs are digest-pinned
upstream data defects: one malformed oracle and one malformed Java source. There were no
unexpected case errors. These measurements are retained as historical evidence; changes after
0.2.0 require a fresh run before they can claim the same scores.

| Fixed scorable adapter universe | Precision | Recall | F1 | Oracle coverage |
|---|---:|---:|---:|---:|
| Program elements | 99.993% | 93.600% | 96.691% | 98.846% |
| Fine mappings | 99.948% | 92.559% | 96.112% | 98.638% |

These correspondence scores apply only inside the declared scoring universe; they are not a claim
of 100% historical identity accuracy. In particular, ambiguity-covered gold relations were 0 in
this run, multi-relation recall remains weak, and predictions outside the scoring universe are
reported but not counted as true or false positives: 170 forced program-element predictions and
560,684 forced fine-mapping predictions were unscored. This subset and protocol are not directly
comparable with published full-corpus GumTree or RefactoringMiner figures.

In the v6 evaluation, the adapter flattens only explicit `possible_pairs` into an edge-union
coverage view. That union is not a jointly selectable mapping, and symbolic abstentions contribute
no pair candidates.

See the [complete results and limitations](docs/benchmarks.md), the
[raw evaluation report](benchmarks/diffbenchmark-literature-evaluation-v6.json), and the
[artifact checksums](benchmarks/SHA256SUMS).

The checked-in [ResumeBench-Real v0](benchmarks/resumebench-real-v0/README.md) diagnostic is
historical evidence for StrataDiff 0.3.0's earlier exact-identity policy. It pins five public Gerrit
review histories: four exact partitions totaling 20 carries and 4 identities needing review, plus
one expected refusal after the merge base changed. Its evaluation predates four-way replay and must
not be presented as validation of the current base-drift behavior.

The checked-in [ResumeBench-Real v1](benchmarks/resumebench-real-v1/README.md) freezes that Gerrit
base-drift history under the current policy. An independent four-snapshot oracle and clean release
evaluation agree on 5 carried files (4 exact identities plus 1 four-way replay), the same 2
needs-review files named by Gerrit's public submission record, and 2 retired checkpoint changes.
This is one deliberately selected correctness case; neither v0 nor v1 estimates reviewer time or
defect recall.

The checked-in [ResumeBench-GitHub-Live v1](benchmarks/resumebench-github-live-v1/README.md) extends
that diagnostic to five public GitHub PR histories whose reviewed commits were later force-pushed
away. Across 47 current PR files, the pinned policy carries 23 by exact Git identity and 6 by strict
four-way replay, leaving 18 in the review residue. A naive obsolete-checkpoint-to-head path diff
contains 1,838 paths—1,815 outside the current PR—and still omits 24 current paths. These are
purposefully selected correctness cases with no human-priority ground truth, not prevalence,
time-saving, or safety evidence.

## Quick start

Rust 1.90 or newer is required. The repository includes the compiled Evidence Workbench in
`web/dist`, so an ordinary Cargo build does not require Node.js. Rebuilding or verifying the web
frontend requires Node.js 24 and npm 11.

```console
cargo build --release
target/release/stratadiff build-info
target/release/stratadiff review origin/main HEAD
target/release/stratadiff review origin/main HEAD --checkpoint LAST_REVIEWED_SHA
target/release/stratadiff review origin/main HEAD --checkpoint LAST_REVIEWED_SHA \
  --fail-on-review-residue
target/release/stratadiff github-checkpoint reviews.json --reviewer REVIEWER_LOGIN
target/release/stratadiff diff examples/demo/before.py examples/demo/after.py \
  --output change.axd
target/release/stratadiff verify change.axd \
  examples/demo/before.py examples/demo/after.py
target/release/stratadiff apply change.axd examples/demo/before.py \
  --output rebuilt.py
cmp rebuilt.py examples/demo/after.py
```

### See the review-coverage gate on a real rebase

From a clean checkout, one command builds StrataDiff, materializes a pinned Gerrit review history,
checks the independent oracle, and proves that the required check blocks on exactly the two files
Gerrit recorded as changed after approval. The first Cargo build may download Rust dependencies,
and fixture materialization fetches the pinned Git objects:

```console
python3 scripts/demo_review_coverage.py --open
```

The first run writes all artifacts under `target/review-coverage-demo/`. Later runs can be
reproduced without network access when that fixture and a clean release binary already exist:

```console
python3 scripts/demo_review_coverage.py --offline --open
```

The demo deliberately exits successfully after verifying that the inner required check exits 1;
that red check is the expected product result, not a failed benchmark. With `--open`, the local
Workbench keeps running until you press Ctrl+C.

### Repository review focus

`review` compares the merge base of two Git revisions with the requested head and emits Markdown
that can be written directly to a GitHub Actions step summary. The current `review-v1` JSON is a
producer-attested focus summary: it records commit/blob provenance and a digest of each analyzed
single-file report, but does not include those reports and cannot yet be replay-verified by itself.

```console
target/release/stratadiff review origin/main HEAD > review-focus.md
target/release/stratadiff review origin/main HEAD --format json --output review-focus.json
target/release/stratadiff review origin/main HEAD \
  --checkpoint LAST_REVIEWED_SHA > review-resume.md
```

`--checkpoint` is an explicit caller attestation that the complete PR change set at that commit was
reviewed. StrataDiff does not infer or prove the human action. Each range must resolve to one unique
merge base. With the same merge base, carry requires the same complete Git change identity: status,
similarity, before and after paths and encodings, modes, and object IDs.

When the merge base changed, exact identity remains the fast path. A second path is available only
for one uniquely matched, same-path `Modified` regular file with the same mode. StrataDiff creates
the reviewed byte patch and the upstream byte patch from the old base, rejects any touching or
overlapping edits, translates each patch across the other, and requires both replay orders to
produce the current blob exactly. NUL-containing content, unsupported modes, missing or oversized
blobs, ambiguous candidates, conflicts, and failed replay stay in `needs_review_now`. Additions,
deletions, copies, renames, and type changes do not use this fallback. Upstream-only files are not
part of the current PR residue. Checkpoint changes that match by neither path are counted as retired.
In JSON, the checkpoint policy is
`exact_git_change_identity_or_noninteracting_four_way_byte_replay`; each carried file records either
`exact_git_change_identity` or `exact_noninteracting_four_way_byte_replay` in
`checkpoint_match_basis`.

The Markdown output puts `needs_review_now` first and folds carried changes into a details section.
This is file-level review memory: it does not preserve partial-file comments, prove semantic safety,
account for effects from a newly changed file elsewhere, or grant approval. Rebase-aware review
already exists in products such as Reviewable and Graphite. StrataDiff's narrower goal is a
deterministic, host-neutral gate whose evidence can become part of a portable Change Passport.

Every changed file is retained and placed in one of four lanes:

- `review first`: new, deleted, or structurally changed code;
- `unverified`: unsupported, invalid, or resource-limited content, which stays in the human-review
  queue instead of disappearing;
- `same Git object`: Git reports the same object ID; path, copy, type, and file-mode effects stay in
  the first-pass queue (for gitlinks the object is a target commit, not a blob);
- `parser model matched (non-semantic)`: the pinned CST predicate matched, while textual, comment,
  build, and semantic effects remain explicit non-claims.

Evidence class and attention priority are separate. The conservative alpha policy keeps every file
in the first pass, including same-object metadata changes and parser-model matches. This is
intentional: Rust `stringify!`, Python debug f-strings, C preprocessing, and HTML rendering all show
that discarded source trivia can be observable. A future policy may lower intrinsic priority only
after context-specific adversarial evaluation; `review-v1` does not do so. Explicit checkpoint
comparison is a separate axis. It carries complete Git change identities and, across base drift,
the narrow class of same-file changes that pass non-interacting four-way byte replay. The
`github-checkpoint` command resolves an explicitly named reviewer's latest non-dismissed human
`APPROVED` or `CHANGES_REQUESTED` review from GitHub's list-reviews JSON. It ignores comments, bots,
pending reviews, deleted users, and dismissed reviews. This resolves a historical commit; it does
not prove reviewer authority, preserve partial-file state, or restore a GitHub approval. A GitHub
App and CODEOWNERS-aware multi-reviewer policy remain future work.

Repository discovery disables Git's heuristic rename/copy prepass so oversized or adversarial blobs
cannot consume unbounded work before StrataDiff's limits apply. A unique delete/add pair with the
same object ID is reported as an exact relocation; rename-plus-edit and ambiguous duplicate cases
remain separate changes. Per-file line counts are a linear-time common-prefix/suffix envelope, not
a minimal Git diffstat, and may conservatively include unchanged lines between distant edits.

In GitHub Actions, check out enough history for the merge base and append the Markdown output:

```yaml
- uses: actions/checkout@v5
  with:
    fetch-depth: 0
- run: stratadiff review "${{ github.event.pull_request.base.sha }}" "${{ github.event.pull_request.head.sha }}" >> "$GITHUB_STEP_SUMMARY"
```

The repository also ships an alpha composite action. Analysis runs inside the caller's GitHub
runner and StrataDiff itself has no upload step. If `reviewer` is configured, the Action downloads
up to 100 review records from GitHub's API using the caller-provided token; it fails closed above
that bound. The selected review SHA is verified against GitHub's commit-object API. When a
force-push has removed it from the checkout, the Action fetches that exact object through an
isolated provider-bound repository and imports it locally without the token; it never substitutes
`origin`, the current PR head, or another checkpoint. When consumed from a separately pinned remote
ref, the Action builds from its own directory so a checkout-level `.cargo/config.toml` cannot
redirect that build. A local `uses: ./` invocation has no such boundary because the Action and
checkout are the same tree. The workflow still uses GitHub-hosted or self-hosted runner
infrastructure plus third-party checkout, toolchain, cache, and optional artifact actions.
`fail-on-review-residue` makes the Action suitable as an experimental required check, but it still
does not grant or restore approval, prove semantic safety, or establish reviewer authorization.
When that gate fails, the Action adds file-scoped GitHub error annotations for up to 20 current PR
files that still need review. Larger residues stay bounded in the log and report their remaining
count; the step summary and JSON artifact retain the complete queue.
Audit and pin every action to an immutable full commit before using it in a protected production
workflow; the mutable `main` reference below is only a preview:

```yaml
permissions:
  contents: read
  pull-requests: read

steps:
  - uses: actions/checkout@v5
    with:
      fetch-depth: 0
  - id: review-focus
    uses: gcomfident-crypto/stratadiff@main
    with:
      base: ${{ github.event.pull_request.base.sha }}
      head: ${{ github.event.pull_request.head.sha }}
      reviewer: alice
      github-token: ${{ github.token }}
      fail-on-review-residue: true
  - uses: actions/upload-artifact@v4
    with:
      name: stratadiff-review-focus
      path: ${{ steps.review-focus.outputs.report }}
```

An explicit `checkpoint` overrides API discovery. With `fail-on-review-residue: true`, the report is
still written before the step exits unsuccessfully. A required-check workflow must run both when the
PR head changes and when the configured reviewer submits a new review; otherwise a completed review
cannot turn the check green. The current alpha resolves one explicitly configured reviewer and does
not infer CODEOWNER or branch-protection authority. See the
[review-coverage integration guide](docs/github-review-coverage.md) for the full event lifecycle and
security boundary.

### Evidence Workbench

Open the same proof-carrying analysis as an interactive local review surface:

```console
target/release/stratadiff view examples/demo/before.py examples/demo/after.py
target/release/stratadiff review origin/main HEAD \
  --checkpoint LAST_REVIEWED_SHA --workbench
```

When both snapshots have the same merge base, Repository Review Resume opens on the checkpoint to
head snapshot delta. When the base changed, that direct snapshot delta contains upstream noise, so
the Workbench instead shows current-base-to-head files that were not carried by exact identity or
four-way replay. Upstream-only files are excluded. Switch to full PR context to inspect the complete
current merge-base-to-head range. A reverted checkpoint change can still appear in retired
accounting without remaining in the current PR diff. File sources come from the object IDs recorded
in the report, never from the mutable worktree.

Files with a structural evidence digest can open the original single-file Workbench. That viewer
keeps the readable code diff, structural relations, ambiguity constraints, and exact byte edits as
separate synchronized layers. Selecting an item opens its observable facts, model selection rule,
non-claims, and verification trace. Invalid UTF-8 is rendered losslessly as bytes rather than
decoded with replacement characters, and a symbolic abstention with `pair_claims: none` never
becomes a set of speculative correspondence lines.

`view` performs the same bounded analysis and matcher-free verification before starting the UI.
The repository-level summary is explicitly marked `producer_attested`; only an opened per-file
report whose recorded digest is regenerated and checked receives the verified evidence treatment.
The local server
binds only to `127.0.0.1`, chooses an ephemeral port by default, protects the session endpoint with
a random token, and embeds all UI assets in the release binary. No source or report data is sent to
an external service. Pass `--no-open` to print the URL without launching a browser, or `--port PORT`
to choose a loopback port. On a shared multi-user host, prefer `--no-open`: the automatic browser
launcher receives the token-bearing URL as a command-line argument, which may be briefly visible
to other local users through process inspection. Treat the printed URL as a session secret. Press
Ctrl+C to stop the server.

Run the complete local CI gate with `scripts/ci.sh`.

`diff --output` and `diff --json` emit compact JSON so reports produced within the default 64 MiB
report boundary can be consumed by `verify` and `apply` without a formatting-size mismatch.

Print the full machine-readable result with `--json`:

```console
stratadiff diff old.ts new.ts --json
```

For a file type without a native grammar, select the conservative Universal byte mode explicitly:

```console
stratadiff diff old.unknown new.unknown --language universal --output change.axd
```

Universal builds a deterministic `file → line → byte-token-run` tree and works on arbitrary byte
content within the declared resource limits, including NULs, invalid UTF-8, mixed line endings, and
files without an extension. It is not a language grammar, AST, semantic analysis, or automatic
fallback. Unknown and ambiguous extensions fail explicitly unless the caller chooses a parser
mode; native parser error nodes also fail.

The current binary ships 29 native grammar modes: Bash, C, C++, C#, CSS, Elixir, Go, Haskell,
HTML, Java, JavaScript/JSX, JSON, Kotlin, Lua, Markdown block structure, OCaml implementation and
interface files, PHP with embedded markup, Python, R, Ruby, Rust, Scala, Swift, TOML, TypeScript,
TSX, YAML, and Zig. These provide concrete-syntax structure, not compiler-level semantics. `.h`
and `.m` are deliberately not guessed because their extensions are ambiguous.

The coverage contract has three distinct layers:

| Layer | Coverage now | Verified claim |
|---|---|---|
| Byte transformation | Arbitrary byte content when Universal is explicitly selected, and valid native-mode input, within the same declared limits | Applying an accepted patch reproduces the supplied target bytes exactly |
| CST structure | Only the native grammars compiled into this build | Serialized syntax/shape predicates hold under the pinned grammar and runtime |
| Language semantics | None in report v3; the JDT bridge is evaluation-only | No binding, type, control-flow, or refactoring-semantic claim |

The default terminal summary renders every terminal byte edit only after replay proves that the
edits reconstruct the target. It prints before/after byte ranges, JSON-quoted UTF-8 with terminal
control characters escaped, and Base64 for non-UTF-8 payloads. This is exact patch-hunk rendering,
not a claim that the structural view is a complete AST or semantic explanation.

## Resource-bounded verification

The `stratadiff-verifier` crate has no dependency on the producer matcher, `similar`, CLI parsing,
CSV tooling, or temporary-file support. For untrusted input, use `verify_report_bytes` or
`verify_and_replay_report_bytes`; these scan collection lengths before constructing the typed
report. The older `verify_report` and `apply_patch` entry points remain source-compatible and use
the defaults below. Typed callers that need different bounds can use `verify_report_with_limits`
and `replay_patch_with_limits`.

| Limit | Default | Scope |
|---|---:|---|
| Raw or compact-serialized report | 64 MiB | One report |
| Source or replayed output | 16 MiB | Each byte array |
| Relations | 250,000 | Total |
| Ambiguity groups | 50,000 | Total |
| Ambiguity endpoints | 500,000 | Both sides combined |
| Exact ambiguity pairs | 1,000,000 | Total |
| Structural changes | 250,000 | Total |
| Patch edits | 250,000 | Total |
| Decoded replacement bytes | 32 MiB | All edits combined |
| Syntax nodes | 1,000,000 | Both fresh parses combined |
| Syntax depth | 512 | Each parse |
| Tree-sitter progress callbacks | 4,000,000 | Each parse |
| Verification work | 128 Mi units | Deterministic verification-work budget |

The CLI uses these defaults and does not currently expose limit flags. It reads files through a
`limit + 1` bounded reader, validates canonical Base64 and checked size arithmetic, and never writes
an `apply` output until replay and the full structural verification have succeeded. Each relation
may carry at most four evidence items, matching the largest evidence recipe in report v3.

These controls bound attacker-selected input, collection growth, parser progress, recursive
comparison, candidate scanning, sorting, and alignment DP. They are not a process sandbox, a wall
clock deadline, or an allocator-level limit; the selected Tree-sitter grammars and the local runtime
remain part of the trusted computing base. Work units are conservative deterministic charges, not
CPU instructions or milliseconds. Callers that deserialize an untrusted report themselves before
using the typed API give up the decode-time collection protection.

## Current algorithm

The alpha implements the first useful slice of Proof-Carrying Structural Diff (PCSD):

1. Build both trees with the selected native Tree-sitter grammar or the explicit Universal byte
   parser.
2. Compute domain-separated byte, syntax, and shape Merkle fingerprints bottom-up.
3. Verify hash hits recursively, so correctness does not depend on collision resistance alone.
4. Map globally unique identical subtrees and unique identical children under mapped parents.
5. Split unmatched direct children at non-crossing exact anchors, partition each region into
   bounded compatibility-graph components, and align at most 64 active children per side in each
   order-interaction component. Map a singleton candidate-group pair only when it is present in every
   maximum-cardinality ordered alignment.
6. Encode singleton-group ties as exact coupled ordered constraints. Preserve duplicate symmetry
   and oversized interaction components as symbolic abstentions that make no pair claims.
7. Derive insertions, deletions, child-order changes, and model-forced shape updates without
   conflating their evidence levels. The report model also retains `equivalent_relocation`, emitted
   only when an exact pair's before parent has a mapped counterpart different from the pair's
   actual after parent. Its current recall has not been established; it describes the snapshots
   under this mapping model, not the author's historical edit, and the matcher keeps the safer
   delete/insert or ambiguity result when exact anchors conflict.
8. Build an exact patch under the
   `bounded-patience-lines+bounded-byte-refinement-v2` contract using budgeted line-level Patience
   anchors, bounded byte-level Myers refinement, and linear aligned-byte or replacement paths for
   large unmatched regions.
9. Replay the patch immediately and emit a BLAKE3 certificate only if the output is byte-identical.

Typical matching and hashing are linear in syntax-tree size. Ordered dynamic programming is
restricted to independent interaction components of at most 64 active nodes per side; larger
components remain symbolic and never allocate a quadratic candidate matrix. Candidate compatibility
scanning is capped at 16,384 pairs per verified shape class. Patience anchoring is capped at 65,536
lines across both inputs, Myers refinement when the two trimmed sides total at most 64 KiB, aligned large-region
output at 4,096 edits per region, and total patch output at 65,536 edits. The aligned path does not
infer resynchronization for a length-neutral insertion plus deletion, so that case may be displayed
as a larger exact replacement; replay correctness is unaffected.

Native-grammar source positions follow Tree-sitter: zero-based rows and UTF-8 byte columns.
Universal positions use zero-based rows and raw-byte columns.

See [DESIGN.md](DESIGN.md) for invariants, [docs/research.md](docs/research.md) for the tool and paper
survey that motivated the engine, [docs/evidence-workbench.md](docs/evidence-workbench.md) for the
review-UI survey and interaction decisions, and [docs/benchmarks.md](docs/benchmarks.md) for
reproducible evaluation results and the local performance baseline.

The JSON serialization and structural constraints are published as
[schema/report-v3.schema.json](schema/report-v3.schema.json). Historical
[v1](schema/report-v1.schema.json) and [v2](schema/report-v2.schema.json) schemas remain available
for inspection. Old reports are not relabeled or silently upgraded; rerun the original snapshots
to produce a v3 report. Report-model and claim validity are stricter than the schema alone and are
established by `stratadiff verify`, whose matcher-free verifier crate rebuilds the selected parser
representation and re-derives the report's claims.

## Report excerpt

```json
{
  "relations": [
    {
      "predicate": "syntax_equal",
      "correspondence": "model_forced",
      "evidence": ["globally_unique_identical_syntax_subtree", "recursive_syntax_equality_check"]
    },
    {
      "predicate": "shape_equal",
      "correspondence": "model_forced",
      "evidence": [
        "bounded_ordered_child_alignment_v1",
        "pair_present_in_every_optimal_alignment",
        "recursive_shape_equality_check",
        "not_a_historical_identity_claim"
      ]
    }
  ],
  "ambiguities": [
    {
      "constraint": {
        "kind": "symbolic_abstention",
        "cause": "duplicate_symmetry",
        "pair_claims": "none"
      },
      "reason": "repeated shape-equivalent children are intentionally unresolved; endpoint sets make no pair claims"
    }
  ],
  "certificate": {
    "patch_verified": true
  }
}
```

## Near-term roadmap

- Binding-aware alpha equivalence and no-capture rename certificates.
- More compact correspondence proof objects for cheaper independent verification.
- Repository mode with conservative file pairing and parallel parsing.
- Java and C/C++ semantic adapters using compiler-grade front ends.
- A dedicated relocation-evidence phase that can recover more moves without relaxing exact-anchor
  compatibility.
- IDE provenance mode, which can observe actual node lineage instead of inferring history.
- Accuracy and abstention benchmarks against GumTree, RefactoringMiner, and curated adversarial
  edits.

## License

StrataDiff is licensed under the MIT License. The bundled Evidence Workbench includes
`@pierre/diffs`, `@pierre/theme`, and `@pierre/theming` under Apache-2.0; their shared license and
the original `@pierre/theme` notice are preserved in [third_party/pierre](third_party/pierre).
