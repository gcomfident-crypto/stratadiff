# ResumeBench seed v1

ResumeBench seed v1 is a controlled regression corpus for StrataDiff's Exact Review Resume
policy. It models a base commit, a caller-selected checkpoint whose complete PR change set was
reviewed, and a current head created from the same base with independently rewritten history.

The source of truth is
[`../resumebench-seed-v1.json`](../resumebench-seed-v1.json). Every current change must be placed
in exactly one state:

- `needs_review_now`: its complete Git change identity differs from every checkpoint change;
- `unchanged_since_checkpoint`: status, similarity, paths and path encodings, modes, and object IDs
  are all identical to one checkpoint change.

Checkpoint identities absent from the current identity set—including identities superseded by a
new mutation—are counted as retired and do not enter the current file denominator. The
implementation must also reject checkpoint comparisons whose ranges do not share one exact merge
base.

The seed covers rewritten sibling history, a one-byte mutation, new and retired changes, changed
rename destinations, unsupported content, exact deletions, and a target-mode mismatch. It is
designed to make unsafe carry-forward difficult, not to estimate real-world workload reduction.

## Metrics

The automated gate reports and checks:

- **mutation invalidation recall:** every seeded current change labeled `needs_review_now` is
  invalidated;
- **exact carry precision:** every carried item has byte-for-byte identical serialized Git change
  identity in the checkpoint and current ranges;
- **current accounting:** needs-review plus unchanged equals every current PR change exactly once;
- **retired accounting:** checkpoint-only changes are explicit and never inflate the current
  denominator.

Run it with:

```text
cargo test --test resumebench
```

## What this does not establish

The checkpoint is an assertion by the caller; StrataDiff does not prove that a person reviewed or
approved it. Exact file-level change identity does not prove semantic safety or rule out effects
from newly changed files elsewhere in the repository. The seed has no partial-file review state,
real PR frequency estimate, reviewer timing, or issue-recall measurement. Those require a pinned
multi-revision PR corpus and a counterbalanced human study before making a productivity claim.
