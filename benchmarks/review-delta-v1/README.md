# Review Delta v1 controlled benchmark

This suite is the executable contract for StrataDiff's rebase-aware review queue. It constructs thirteen
small Git histories locally, with no network access, and evaluates the five snapshots involved in a
review continuation:

```text
A = old merge base
B = reviewed checkpoint
C = current merge base
D = current head
S = A->B and A->C replayed in both orders to one byte-identical baseline
```

Resume may compare `S -> D`, use an explicit conservative fallback, or be empty after a pure
rebase. Within this suite's regular-blob scope (`100644` and `100755`), Full PR scope must always
remain the exact raw Git identity set and source bytes for `C -> D`. The benchmark checks both
scopes through the real Workbench HTTP endpoints, not only the serialized CLI summaries.

## Cases and frozen outcomes

| Case | Resume outcome | Full `C -> D` |
|---|---|---|
| `pure-rebase-carries-reviewed-edit` | Empty; gate passes | One carried modification |
| `noninteracting-author-followup` | One-line exact `S -> D` | Two changed lines |
| `upstream-absorbed-reviewed-addition` | Empty; retired change is proven absorbed and gate passes | Empty |
| `new-current-file-after-checkpoint` | One unrelated new file | Carried reviewed file plus new file |
| `dropped-reviewed-edit` | One-line removal from `S`; gate fails | Empty |
| `overlapping-edits-fail-closed` | `current_base_fallback / overlap_or_adjacent` | One modification |
| `adjacent-edits-fail-closed` | `current_base_fallback / overlap_or_adjacent` | One modification |
| `binary-nul-fail-closed` | `current_base_fallback / binary_nul` | One binary modification |
| `added-file-fallback` | `current_base_fallback / unsupported_change` | One addition |
| `deleted-file-fallback` | `current_base_fallback / unsupported_change` | One deletion |
| `renamed-file-fallback` | `current_base_fallback / unsupported_change` | One exact relocation |
| `dropped-rename-displays-both-paths` | Two `checkpoint_head_fallback` entries; gate fails | Empty |
| `mode-change-fallback` | `current_base_fallback / unsupported_change` | One mode change |

Every expected status, path, line envelope, baseline basis, fallback reason, source kind, gate
result, and reconstructed baseline byte string is in [`manifest.json`](manifest.json). Every case
has a real base drift (`tree(A) != tree(C)`) and the deterministic topology `A->B`, `A->C->D`.

## Run

Validate the manifest and prove that materialization is deterministic:

```text
python3 tools/review-delta-v1/review_delta_v1.py validate
python3 tools/review-delta-v1/review_delta_v1.py self-test
```

Evaluate a development binary. The runner materializes temporary repositories, executes the CLI
artifact path and residue gate, starts one loopback Workbench per case, and verifies all Full and
Resume source bytes:

```text
cargo build --bin stratadiff
python3 tools/review-delta-v1/run.py \
  --stratadiff target/debug/stratadiff \
  --output /tmp/review-delta-v1-evaluation.json
python3 tools/review-delta-v1/verify.py \
  --evaluation /tmp/review-delta-v1-evaluation.json
```

Retain the generated repositories for diagnosis with `--workdir /new/absolute/path`. That path
must not already exist; the runner never overwrites an existing materialization.

For a result intended for publication or release evidence, build from a clean checkout and require
the embedded provenance to report both `git_dirty=false` and `build_profile=release`:

```text
scripts/build-release.sh --bin stratadiff
python3 tools/review-delta-v1/run.py \
  --stratadiff target/release/stratadiff \
  --require-clean \
  --output /tmp/review-delta-v1-release.json
```

## What is independently checked

The standard-library runner and Git plumbing verify:

1. exact tree bytes, modes, parentage, merge bases, and deterministic commit IDs for A-D;
2. manifest coverage and internal count/gate consistency;
3. every Full report identity against an independent `git diff --raw --no-renames C D`, including
   StrataDiff's unique exact-relocation normalization;
4. every regular-blob Full Workbench source byte-for-byte against the recorded C and D blobs;
5. Resume entries, exact reconstructed baseline bytes, fallback reasons, source commits/objects,
   line envelopes, and Workbench source bytes;
6. the CLI residue gate exit status; and
7. a saved evaluation's manifest digest, build provenance, complete case set, and normalized
   outcomes.

## Claim boundary

Passing this benchmark establishes deterministic behavior for these controlled histories and
guards fail-closed edge cases. It does not show how common these histories are, prove that a human
reviewed B, establish semantic safety, measure defect recall, or demonstrate reviewer-time savings.
Those claims require real-history sampling and a human study. Version 1 also excludes symlinks and
gitlinks/submodules: a gitlink has commit-pointer identity but no blob source bytes, and the current
Workbench source endpoint returns 422 for it. A passing v1 result therefore makes no claim about
source rendering for those modes. Saved evaluation JSON is unsigned; verification proves internal
consistency with the frozen manifest, not artifact authorship or CI execution provenance.
