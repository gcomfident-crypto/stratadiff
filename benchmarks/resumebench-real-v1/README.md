# ResumeBench-Real v1

ResumeBench-Real v1 freezes one real rebase history that v0 intentionally rejected. It tests
StrataDiff's current review-residue contract against Gerrit change
[`612221`](https://gerrit-review.googlesource.com/c/gerrit/+/612221): patch set 8 was the last
`Code-Review+2` checkpoint, patch set 9 rebased it, and patch set 10 was submitted after two files
changed.

This is a narrow provenance case, not a representative sample. Its value is that Gerrit's own
submission record names the two files that changed after the last approved patch set, while the
four Git snapshots remain publicly fetchable.

## Frozen result

```text
A = c84d40e  checkpoint base / PS8 parent
B = ea65335  reviewed checkpoint / PS8
C = 007993f  current base / PS10 parent
D = 251e1f9  submitted head / PS10
```

The current PR-relative delta is `C -> D`, so upstream-only files are outside the result. The
oracle freezes this partition:

| State | Count | Independent basis |
|---|---:|---|
| Exact-identity carry | 4 | The complete raw Git change identity is identical in `A -> B` and `C -> D` |
| Four-way carry | 1 | A checked-in byte-edit witness proves both `A -> B -> D` and `A -> C -> D` |
| Needs review now | 2 | Gerrit's submission message names these as changed since approved PS8 |
| Retired checkpoint changes | 2 | The corresponding PS8 identities no longer occur in `C -> D` |

The two needs-review files are `ChangeQueryBuilder.java` and `RegexOnlyPathsPredicate.java`.
`Documentation/user-search.txt` is the replay-carried file. The remaining four current files are
exactly carried.

## What is independently checked

[`verify.py`](verify.py) uses only the Python standard library and Git plumbing to:

1. bind snapshots `A`–`D` to full commit IDs, parents, ancestry, raw deltas, modes, and blob IDs;
2. recompute the seven checkpoint and seven current PR-relative identities with renames disabled;
3. prove the four exact carries by full identity equality;
4. validate the frozen byte witness for `Documentation/user-search.txt`, including blob SHA-256,
   non-overlap and non-adjacency, and exact replay in both application orders;
5. bind the two needs-review paths to the normalized Gerrit submission evidence in `manifest.json`;
6. verify the two retired PS8 identities; and
7. optionally compare every file state and per-file `checkpoint_match_basis` with a StrataDiff
   release binary.

Oracle verification is offline. It sets `GIT_NO_LAZY_FETCH=1`, so a partial repository missing a
required object fails instead of silently contacting the network. `verify-provenance` is a separate
online check against the public Gerrit API.

## Reproduce

Run the pure-function tests:

```text
python3 benchmarks/resumebench-real-v1/verify.py self-test
```

Materialize the exact public refs (this is the only data-fetching step):

```text
python3 benchmarks/resumebench-real-v1/verify.py materialize \
  --output /absolute/path/to/resumebench-real-v1
```

Then disconnect the network if desired and verify the frozen oracle:

```text
python3 benchmarks/resumebench-real-v1/verify.py verify-oracle \
  --repository /absolute/path/to/resumebench-real-v1/repository.git
```

Evaluate a clean release build and write a provenance-bound result:

```text
cargo build --locked --release --bin stratadiff
python3 benchmarks/resumebench-real-v1/verify.py evaluate \
  --repository /absolute/path/to/resumebench-real-v1/repository.git \
  --stratadiff target/release/stratadiff \
  --output /tmp/resumebench-real-v1-evaluation.json
```

[`evaluation-v1.0.0.json`](evaluation-v1.0.0.json) is the frozen clean-release run for the engine
revision that introduced rebase-aware carry. A new engine should produce a new evaluation artifact
rather than overwrite that historical record.

Refresh the external provenance check independently of the offline oracle:

```text
python3 benchmarks/resumebench-real-v1/verify.py verify-provenance
```

## Claim boundary

Passing v1 establishes that this implementation reproduces Gerrit's recorded 5/2 review-residue
partition on one real rebased change and supplies an exact byte proof for the non-identity carry.
It does not establish reviewer-time savings, defect recall, semantic equivalence, prevalence, or
safe approval. Those require a larger sampling protocol and a reviewer study.

The benchmark metadata and verifier are MIT licensed. Gerrit source is Apache-2.0 and is fetched
only into the caller's materialization directory; no source snapshot or reviewer identity is
vendored here.
