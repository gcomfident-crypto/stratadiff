# Benchmarks

## ResumeBench-Real v0

The first real-history review-resume diagnostic pins five merged changes from the public Gerrit
project. Each case has an earlier patch set with a public `Code-Review+2` event and a later submitted
patch set. Four cases retain the same merge base. The fifth rebases between the reviewed and current
snapshots and was expected to be rejected by the v0 policy.

An independent Python/Git oracle, separate from the StrataDiff library, derives complete change identities for
the base-to-checkpoint, base-to-head, and checkpoint-to-head ranges. It stores raw-diff digests,
base64 paths, modes, Git object IDs, canonical identity SHA-256 values, and blob content SHA-256
values. The checked-in StrataDiff 0.3.0 run reports:

| Cases | Current identities | Exactly carried | Need review now | Retired | False carry | False invalidation |
|---:|---:|---:|---:|---:|---:|---:|
| 5 (4 partitions + 1 refusal) | 24 | 20 | 4 | 3 | 0 | 0 |

All five cases passed under that historical policy, including the expected base-drift refusal. The
checked-in evaluation was produced before non-interacting four-way byte replay was implemented and
must not be used as evidence for the current base-drift behavior. The observed focus share is
4 / 24 = 16.7% in the deliberately selected comparable cases. It is not a population estimate and
does not establish reviewer-time savings or defect recall. See the
[`ResumeBench-Real v0` dataset card](../benchmarks/resumebench-real-v0/README.md),
[`manifest`](../benchmarks/resumebench-real-v0/manifest.json), and
[`evaluation`](../benchmarks/resumebench-real-v0/evaluation-v0.1.0.json).

## ResumeBench-Real v1

The current policy was evaluated against the pinned fifth history,
`gerrit-612221-ps8-ps10`. Exact identity carried four files. Four-way replay carried
`Documentation/user-search.txt`, where the reviewed 13-line insertion did not interact with an
upstream one-line deletion elsewhere in the file. The resulting current PR partition was 5 carried
files and 2 residue files, matching Gerrit's public submission record. The residue excluded files
changed only between the two base revisions.

ResumeBench-Real v0 remains frozen. The separate
[`ResumeBench-Real v1`](../benchmarks/resumebench-real-v1/README.md) artifact defines the
four-snapshot replay rule, independently checks the 5/2 partition and per-file carry basis, and
binds a clean release evaluation to the engine revision. It remains a single selected correctness
case and cannot support claims about prevalence, reviewer time, or defect recall.

## ResumeBench-GitHub-Live v1

The GitHub-live diagnostic freezes five public pull-request histories with a human `APPROVED` or
`CHANGES_REQUESTED` review followed by a force-push. Each case records the requested base, reviewed
checkpoint, both derived merge bases, captured final head, exact review ID, force-push events,
source-license observation, and the complete current file partition. Its independent Python/Git
oracle recomputes raw identities and four-way byte replay without importing the StrataDiff library;
the product evaluation is a separate comparison against that oracle.

| Cases | Current files | Exact carry | Four-way carry | Need review | Retired |
|---:|---:|---:|---:|---:|---:|
| 5 | 47 | 23 | 6 | 18 | 67 |

A naive direct path diff from the obsolete reviewed commit to the captured head contains 1,838
paths. Of those, 1,815 are not current PR paths, while 24 current PR paths are absent. This is a
path-set diagnostic of base noise, not a reviewer-value metric. The five cases were selected to
exercise policy behavior; they are neither random nor representative and contain no human-priority
ground truth. See the
[`ResumeBench-GitHub-Live v1` dataset card](../benchmarks/resumebench-github-live-v1/README.md).

## Review Delta v1 controlled contract

[`Review Delta v1`](../benchmarks/review-delta-v1/README.md) is the network-free executable
contract for the five-snapshot resume queue. Its thirteen synthetic Git histories cover pure
rebase, an independent author follow-up, a dropped reviewed edit or rename, upstream absorption,
overlap and adjacency rejection, binary content, and conservative add/delete/rename/mode
fallbacks. Every case moves the merge base.

The runner checks more than the serialized summary. An independent Git oracle derives the exact
`C -> D` identities, then the runner queries both Workbench scopes and compares every served source
byte in its regular-blob cases. This catches a class of failures where the gate is conservative but
the reviewer is shown the wrong baseline. It also executes the residue gate and verifies the saved
evaluation independently. Gitlink/submodule source rendering is explicitly outside the v1 claim.

The development suite currently passes all 13 cases, and its Rust harness passes both integration
tests. Those results establish behavior only for the enumerated histories. A publication or release
claim additionally requires a pinned clean release build with `git_dirty=false`; the suite does not
measure prevalence, semantic safety, defect recall, or reviewer time.

## Review Inbox v1 public-metadata seed

The first prospective open-PR seed freezes three public reviewer/PR pairs captured on 2026-09-05.
Separate GitHub REST v3 and GraphQL v4 assets retain each target reviewer's metadata history. The
offline verifier selects the latest completed checkpoint independently from each representation,
then compares reviewer identity, review history, checkpoint, head, and classification. Both paths
agreed on two actionable cases and one stable control. One actionable case has 43 later
`COMMENTED` reviews from the same reviewer; none replaces the earlier `APPROVED` checkpoint.

The verifier binds both observation assets by SHA-256, checks the bounded sequential capture window
and privacy exclusions, and exercises cross-API tamper controls. REST and GraphQL share GitHub as
their upstream, so agreement is not independent ground truth. The seed was convenience-selected
during API feasibility work and therefore supports only the action contract, not prevalence, time
savings, retention, issue recall, safety, or product-market fit. See the
[`Review Inbox v1` dataset card](../benchmarks/review-inbox-v1/README.md).

## DiffBenchmark literature-subset result

The latest provenance-complete Java evaluation was run on 2026-09-04 against the 285-case
DiffBenchmark literature subset at commit
`870592abd559d0bd822a27eb5c8ea45aee47015b`. The raw report is
[`diffbenchmark-literature-evaluation-v6.json`](../benchmarks/diffbenchmark-literature-evaluation-v6.json),
its canonical source manifest is
[`diffbenchmark-literature-manifest-v3.json`](../benchmarks/diffbenchmark-literature-manifest-v3.json),
and [`SHA256SUMS`](../benchmarks/SHA256SUMS) authenticates the checked-in artifacts.

The evaluator selected all 285 cases. It evaluated 283, independently verified all 283 generated
reports, and replayed all 283 targets byte for byte. It recorded one digest-pinned malformed Hive
oracle and one digest-pinned Alluxio source with a missing closing parenthesis. There were no
unexpected case errors, and `benchmarkComplete` is `true`.

| Metric | Program elements | Fine mappings |
|---|---:|---:|
| Raw oracle relations | 15,680 | 145,435 |
| Scorable relations | 15,499 (98.846%) | 143,454 (98.638%) |
| TP / FP / FN | 14,507 / 1 / 992 | 132,780 / 69 / 10,674 |
| Micro precision | 99.993% | 99.948% |
| Micro recall | 93.600% | 92.559% |
| Micro F1 | 96.691% | 96.112% |
| Macro precision | 99.995% (273 defined cases) | 99.890% (275) |
| Macro recall | 84.858% (278) | 86.294% (278) |
| Macro F1 | 90.054% (278) | 90.895% (278) |
| Perfect exact-forced, gold-bearing cases | 21 / 278 (7.554%) | 15 / 278 (5.396%) |
| Singleton recall | 93.733% | 94.023% |
| Multi-relation recall | 0 / 22 (0%) | 22 / 2,256 (0.975%) |
| Ambiguity-covered gold relations | 0 / 15,499 | 0 / 143,454 |
| Multi groups touched by forced edges | 0 / 10 | 36 / 317 (11.356%) |
| Unscored forced predictions | 170 | 560,684 |

Precision and recall apply only to the fixed, scorable adapter universe. In particular, the large
unscored count is disclosed rather than being treated as either correct or incorrect. The
multi-group rate is computed over raw-oracle bipartite connected components before parser/taxonomy
exclusions; the 36 touched fine-mapping groups contain 22 forced gold edges and 14 forced
false-positive edges incident to those groups. The engine emitted no ambiguity candidate that
covered a scorable gold relation in this run, so ambiguity coverage is 0%; this is a current recall
limitation, not evidence of certainty.

The adapter's flattened ambiguity list is the edge union of explicit `possible_pairs`, not one
jointly selectable mapping. `symbolic_abstention` scopes make no pair claims and therefore add no
candidates. This run did not exercise a scorable exact ordered constraint, so it does not establish
coverage for that representation.

The measured `analyze_bytes` latency was 7.486 ms p50, 59.408 ms p95, and 230.951 ms maximum.
Serialized per-case diff reports were 1,900,674 bytes p50, 13,628,735 bytes p95, and 49,257,509
bytes maximum. Independent verification consumed 291,198 deterministic work units p50, 2,586,914
p95, and 104,372,509 maximum, below the default 134,217,728-unit budget in every case. The evaluator
process reached 340,148 KiB `VmHWM`. Latency excludes JDT enumeration, adaptation, verification, and
scoring; `VmHWM` covers the Rust parent process, not the JDT JVM.

Compared with v5, the forced and scorable TP, FP, and FN counts and every reported precision,
recall, and F1 value are unchanged. v6 runs every generated report through the standalone
resource-bounded verifier and records its actual charged work. The adapter continues to project
only explicit `possible_pairs`; symbolic scopes make no pair claims. The run produced no projected
ambiguity candidate, so the benchmark does not yet measure exact-constraint coverage.

Provenance:

- StrataDiff engine commit: `a1dfe8317d742cc975064f62c23aa86ee0dacaeb`
- clean release build: `true`
- evaluator SHA-256: `0703c32e3f08b9a7e8f73e2db609aacb11b1efc9cec32e775703f1be5a20be90`
- `Cargo.lock` SHA-256: `012fa40d4372edbcb8f1c9b3f393ea0307becc56d9b0a00a9b7bccabfed3e211`
- canonical manifest BLAKE3: `0012eecb59360ef45e9ccc2ecaa9c11ca1387bfa6c391238d0301a84ee44d9d3`
- JDT profile: `gumtree-3.0.0-jdt-core-3.35.0-ecj-3.35.0-helper-v3`
- Java: Temurin 17.0.20.1+1 (caller-selected local trust boundary)
- CPU: Intel Core i7-14700; Linux 5.15 x86-64; Rust 1.98.1

This run uses JDT only to enumerate exact oracle-compatible node identities; it is not a GumTree
matcher baseline. It covers the intra-file literature subset, not all DiffBenchmark or Defects4J,
and excludes cross-file cases. The subset contains 22 program-element and 2,256 fine-mapping
multi-relations, but report v3 has only one-to-one verified relations; this is not a dedicated
one-to-many evaluation. Published RefactoringMiner and GumTree figures use different corpora and
protocols and are not directly comparable to this table.

Formal command (no `--limit`):

```console
stratadiff-evaluate /absolute/path/to/DiffBenchmark /absolute/path/to/materialized \
  --jdt-cache /absolute/path/to/jdt-cache \
  --java-executable /absolute/path/to/jdk-17/bin/java \
  --require-complete \
  --output /absolute/path/to/evaluation.json
```

## Local performance baseline

This is an engineering baseline, not a cross-tool accuracy comparison.

- Date: 2026-09-04
- CPU: Intel Core i7-14700
- OS: Linux 5.15 x86-64
- Rust: 1.98.1
Command:

```console
cargo bench --bench structural -- --sample-size 10 --measurement-time 2 --warm-up-time 1
```

The ordinary case generates Python functions containing an assignment and return statement, with
every fiftieth function changed between snapshots. The duplicate-sibling case repeats an identical
function in both snapshots and exercises symbolic ambiguity handling without constructing a
quadratic candidate graph. Timings include two fresh Tree-sitter parses, Merkle hashing, matching,
structural event derivation, patch construction, replay, and certificate hashing. Criterion prepares
owned input buffers outside the timed routine.

| Functions | Combined bytes | Median estimate | Throughput |
|---:|---:|---:|---:|
| 100 | 14.61 KiB | 5.495 ms | 2.597 MiB/s |
| 1,000 | 149.96 KiB | 69.743 ms | 2.100 MiB/s |
| 5,000 | 767.15 KiB | 421.83 ms | 1.776 MiB/s |

| Duplicate functions | Combined bytes | Median estimate | Throughput |
|---:|---:|---:|---:|
| 100 | 5.86 KiB | 2.291 ms | 2.498 MiB/s |
| 1,000 | 58.59 KiB | 24.919 ms | 2.296 MiB/s |
| 5,000 | 292.97 KiB | 137.46 ms | 2.081 MiB/s |

These numbers establish a reproducible v0.2 starting point. The next optimization pass will profile
allocations, intern kinds and fields, and add deeply nested syntax and large unrelated byte-region
benchmarks. The 10,000-duplicate acceptance case remains a resource-safety test rather than a short
Criterion sample.
