# StrataDiff

**A structural diff that knows when it does not know.**

StrataDiff is a fast, proof-carrying source-code differ. It separates three questions that
traditional AST differencers often mix together:

1. What byte transformation turns the old file into the new file?
2. Which structural predicates can be checked directly?
3. Which node correspondences are forced by the declared model, merely suggested, or ambiguous?

The first question is answered losslessly. The second is independently rechecked by
`stratadiff verify`. The third never silently turns a heuristic score into a historical fact.

> **Project status:** research alpha. The replay certificate, conservative syntax anchors, bounded
> all-optima child alignment, duplicate ambiguity handling, deterministic JSON report, independent
> resource-bounded verifier crate, native Tree-sitter adapters, and an explicit Universal byte mode
> work now. A provenance-complete DiffBenchmark literature-subset evaluation is published below.
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
DiffBenchmark's pinned Java literature subset. It evaluated, independently verified, and
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

## Quick start

Rust 1.90 or newer is required. The repository includes the compiled Evidence Workbench in
`web/dist`, so an ordinary Cargo build does not require Node.js. Rebuilding or verifying the web
frontend requires Node.js 24 and npm 11.

```console
cargo build --release
target/release/stratadiff diff examples/demo/before.py examples/demo/after.py \
  --output change.axd
target/release/stratadiff verify change.axd \
  examples/demo/before.py examples/demo/after.py
target/release/stratadiff apply change.axd examples/demo/before.py \
  --output rebuilt.py
cmp rebuilt.py examples/demo/after.py
```

### Evidence Workbench

Open the same proof-carrying analysis as an interactive local review surface:

```console
target/release/stratadiff view examples/demo/before.py examples/demo/after.py
```

The viewer keeps the readable code diff, structural relations, ambiguity constraints, and exact
byte edits as separate synchronized layers. Selecting an item opens its observable facts, model
selection rule, non-claims, and verification trace. Invalid UTF-8 is rendered losslessly as bytes
rather than decoded with replacement characters, and a symbolic abstention with
`pair_claims: none` never becomes a set of speculative correspondence lines.

`view` performs the same bounded analysis and independent verification before starting the UI. It
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
| Verification work | 128 Mi units | Deterministic semantic-work budget |

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
survey that motivated the design, and [docs/benchmarks.md](docs/benchmarks.md) for reproducible
evaluation results and the local performance baseline.

The JSON serialization and structural constraints are published as
[schema/report-v3.schema.json](schema/report-v3.schema.json). Historical
[v1](schema/report-v1.schema.json) and [v2](schema/report-v2.schema.json) schemas remain available
for inspection. Old reports are not relabeled or silently upgraded; rerun the original snapshots
to produce a v3 report. Report-model and claim validity are stricter than the schema alone and are
established by `stratadiff verify`, which independently rebuilds the selected parser representation
and re-derives the report's claims.

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
