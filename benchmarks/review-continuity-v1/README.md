# Review Continuity v1

Review Continuity v1 is a deterministic, network-free comparison of StrataDiff and three cheap
substitutes people may reasonably try before adopting a dedicated review-memory tool. It asks one
narrow question: after a reviewed checkpoint is rewritten onto a new base, which repository paths
still contain byte-level review residue?

The suite materializes six tiny Git repositories from the frozen
[`manifest.json`](manifest.json). The separately frozen [`oracle.json`](oracle.json) was written
from the A/B/C/D byte histories, not from StrataDiff output:

```text
A = old merge base
B = caller-attested reviewed checkpoint
C = current merge base
D = current head
```

Every B and C is a direct child of A; every D is a direct child of C. The runner verifies every
tree byte, parent edge, and merge base before evaluating a method.

## Cases

| Case | Risk exercised | Required byte-level attention |
|---|---|---|
| `pure-rebase` | Reviewed patch replayed on an unrelated base | none |
| `author-followup` | One post-review edit beside a carried edit | `feature.py`, 2 changed lines |
| `dropped-reviewed-edit` | Reviewed edit vanished while C→D is empty | `feature.py`, 2 changed lines |
| `whitespace-patch-id-collision` | Stable patch-id normalizes meaningful indentation | `auth.py`, 2 changed lines |
| `stack-squash-parent-hazard` | B and D trees match, but C→D reverses a restacked-parent policy edit | `stack.py`, 2 changed lines |
| `rename-and-edit` | Reviewed file renamed and edited after base drift | `old.py` + `new.py`, 4 changed lines |

The whitespace fixture is intentionally invalid Python after the rewrite: it demonstrates that
byte-distinct changes can share a stable patch-id. The benchmark does not infer whether arbitrary
formatting changes are semantically important.

## Compared methods

- `git_patch_id`: compare stable patch IDs for A→B and C→D. Equal means carry all; unequal exposes
  only C→D.
- `checkpoint_to_head`: expose the no-rename B→D path diff.
- `git_range_diff`: use `git range-diff A..B C..D`; an all-`=` series carries all, otherwise expose
  the union of A→B and C→D. This union is a deliberately conservative adapter, not a claim that
  range-diff itself is a review queue.
- `stratadiff`: run the real review-delta path and its fail-closed residue gate.

On the frozen six-case oracle, the expected and locally reproduced result is:

| Method | Synthetic false-carry cases | Exact path + line cases | Attention line changes | Avoidable lines on non-miss cases |
|---|---:|---:|---:|---:|
| StrataDiff | 0 | 6/6 | 12 | 0 |
| range-diff union adapter | 0 | 2/6 | 22 | 10 |
| stable patch-id adapter | 2 | 3/6 | 10 | 2 |
| checkpoint→head | 1 | 1/6 | 18 | 8 |

Here, “false carry” means only that a method omitted a frozen required path or exposed fewer line
changes than the exact controlled residue. It is not a production safety rate. A method can also
show many unnecessary lines without missing a required path.

## Run and verify

Only Python's standard library, Git, and a StrataDiff binary are required. No network call occurs.

```text
python3 -B tools/review-continuity-v1/run.py validate
python3 -B tools/review-continuity-v1/run.py self-test

cargo build --bin stratadiff
python3 -B tools/review-continuity-v1/run.py run \
  --stratadiff target/debug/stratadiff \
  --output /tmp/review-continuity-v1.json

python3 -B tools/review-continuity-v1/verify.py verify \
  --evaluation /tmp/review-continuity-v1.json
python3 -B tools/review-continuity-v1/verify.py verify-bundle
python3 -B tools/review-continuity-v1/verify.py self-test
```

For publication-grade build provenance, first build from a clean checkout and add
`--require-clean`. That option rejects a dirty build or a non-release binary:

```text
scripts/build-release.sh --bin stratadiff
python3 -B tools/review-continuity-v1/run.py run \
  --stratadiff target/release/stratadiff \
  --require-clean \
  --output /tmp/review-continuity-v1-release.json
```

`--workdir /new/absolute/path` retains all six repositories and raw StrataDiff artifacts for
inspection. The path must not exist; the runner never overwrites a materialization.

## Independent and tamper checks

The runner and verifier share no Python imports. The runner independently checks Git topology,
tree bytes, patch IDs, range-diff markers, delta commit bindings, and gate exit status. From the
manifest bytes, the verifier separately rebuilds every A→B, B→D, and C→D path/line scope, derives
each adapter result from its evidence, derives StrataDiff's result from normalized delta entries,
and recomputes every score and aggregate. It also checks hashes, provenance shape, exact case
membership, duplicate JSON keys, and closed fields.

The verifier self-test invokes the real verification paths and rejects seven tamper classes:
oracle byte mutation, duplicate JSON keys, a forged result with internally recomputed scores and
aggregate, an omitted case, contradictory range-diff evidence, an added evaluation field, and a
raw checksum mutation. `SHA256SUMS` binds the README, manifest, oracle, runner, and independent
verifier. The checksums detect accidental or local tampering; they are not signatures and do not
authenticate the publisher.

## Claim boundary

This is a controlled regression suite, not a benchmark of real reviewer behavior. Passing it does
not prove that a human reviewed B, that carried code is semantically equivalent or correct, that
StrataDiff detects defects, or that developers save time. It provides reproducible evidence that,
for these six byte histories, StrataDiff preserves the frozen residue while the named baseline
adapters either miss a hazard or expose more review material. Real-history generalization and the
preregistered human study remain separate gates.
