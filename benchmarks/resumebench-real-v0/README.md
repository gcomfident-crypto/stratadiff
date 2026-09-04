# ResumeBench-Real v0

ResumeBench-Real v0 is a pinned diagnostic benchmark for StrataDiff's exact review-resume
contract. It contains five public, merged Gerrit changes where an earlier patch set had a
`Code-Review+2` event and a later patch set was submitted.

This is the first benchmark in the repository based on real review histories. It complements the
synthetic [`resumebench-seed-v1`](../resumebench/README.md): the seed stresses adversarial identity
rules, while this corpus checks those rules against fetchable Git objects and public review-state
events.

## Frozen cases

| Case | Real transition | Expected behavior |
|---|---|---|
| `gerrit-623361-ps1-ps2` | reviewed PS1 → PS2 | 1 exact carry, 2 current identities need review, 1 retired |
| `gerrit-617901-ps3-ps4` | reviewed PS3 → PS4 | 9 exact carries, 1 needs review, 1 retired |
| `gerrit-438704-ps1-ps3` | last-approved PS1 → submitted PS3 | 8 exact carries, 1 needs review, 1 retired |
| `gerrit-620081-ps1-ps2` | reviewed PS1 → commit-message-only PS2 | 2 exact carries, no source identity needs review |
| `gerrit-612221-ps8-ps10` | reviewed PS8 → rebased PS10 | fail closed because the checkpoint and current merge bases differ |

The four comparable histories contain 24 current PR change identities. The independent oracle
labels 20 as exact carries and 4 as needing review. This observed 16.7% focus share is a property of
this deliberately selected sample, not an estimate for typical pull requests.

## Independent oracle

[`resumebench_real.py`](../../tools/resumebench-real/resumebench_real.py) does not import or invoke
StrataDiff while producing ground truth. It reads NUL-delimited `git diff-tree --raw --no-renames`
records and independently constructs these sets:

```text
C = delta(requested base, checkpoint)
H = delta(requested base, current head)
D = delta(checkpoint, current head)

carried          = H intersection C
needs review now = H minus C
retired          = C minus H
```

Every identity includes status, similarity, raw paths, modes, and before/after object IDs. Raw
paths are stored as base64, each identity has a canonical SHA-256, and each referenced blob has a
byte length and content SHA-256. `D` is retained separately because the Workbench's visible
checkpoint delta is not generally the same set as `needs review now`.

The oracle applies the same public exact-relocation policy without using production code: one
deletion and one addition become `R100` only when object ID and mode both match and the candidate
pair is unique. Ambiguous candidates make oracle generation fail.

## Reproduce

Verify that the frozen commits, parents, patch-set numbers, `Code-Review+2` event, and
merge/rebase evidence still match Gerrit's public API:

```text
python3 tools/resumebench-real/resumebench_real.py verify-provenance \
  --manifest benchmarks/resumebench-real-v0/manifest.json
```

Materialize the pinned Gerrit revisions into a local, non-shallow thin repository:

```text
python3 tools/resumebench-real/resumebench_real.py materialize \
  --manifest benchmarks/resumebench-real-v0/manifest.json \
  --output /absolute/path/to/resumebench-real-v0
```

The materializer downloads full objects for the selected patch sets plus a tree-filtered complete
commit graph. It retains Git's promisor metadata for intentionally omitted historical trees, while
oracle and product commands disable lazy fetching and fail if any object needed by a selected case
is absent. It does not vendor Gerrit source into this repository. It verifies all pinned refs,
parents, merge bases, required source objects, and the upstream Apache-2.0 `COPYING` blob before
atomically installing the output.

Recompute the checked-in oracles, or verify them without writing:

```text
python3 tools/resumebench-real/resumebench_real.py generate \
  --manifest benchmarks/resumebench-real-v0/manifest.json \
  --repository /absolute/path/to/resumebench-real-v0/repository.git

python3 tools/resumebench-real/resumebench_real.py verify \
  --manifest benchmarks/resumebench-real-v0/manifest.json \
  --repository /absolute/path/to/resumebench-real-v0/repository.git
```

Run StrataDiff against the frozen oracle from a clean checkout so the evaluator can bind the
result to an exact source revision:

```text
cargo build --locked --release --bin stratadiff
python3 tools/resumebench-real/resumebench_real.py evaluate \
  --manifest benchmarks/resumebench-real-v0/manifest.json \
  --repository /absolute/path/to/resumebench-real-v0/repository.git \
  --stratadiff target/release/stratadiff \
  --output /tmp/resumebench-real-evaluation.json
```

The evaluator reports false carry, false invalidation, identity omissions/extras, retired-count
mismatches, and fail-closed behavior. It also records the exact binary digest plus the embedded Git
revision, dirty state, Cargo.lock digest, build profile, and Rust compiler version from
`stratadiff build-info`. `benchmark_complete=true` requires every frozen case to run and pass using
a clean release build with complete engine provenance.

## Claim boundary and licensing

The manifest, oracle, and evaluation metadata are released under the repository's MIT license.
Gerrit source remains Apache-2.0 and is fetched only into the user's materialization directory.
The dataset records stable public URLs, Git object IDs, review-state message IDs, and timestamps;
it intentionally excludes mutable raw API responses, reviewer names, email addresses, review
bodies, and source snapshots. The online provenance check validates the frozen semantic fields
rather than treating an append-only response body as immutable.

Passing this benchmark establishes exact-identity classification on these five histories. It does
not show how common the pattern is, how much reviewer time is saved, whether developers understand
the focused view, or whether defect recall is preserved. Those claims require a larger sampled
corpus and a counterbalanced reviewer study.
