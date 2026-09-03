# Local performance baseline

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
