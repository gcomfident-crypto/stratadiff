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

Each generated Python function contains an assignment and return statement. Every fiftieth function
changes between snapshots. Timings include two fresh Tree-sitter parses, Merkle hashing, matching,
structural event derivation, patch construction, replay, and certificate hashing. Criterion prepares
owned input buffers outside the timed routine.

| Functions | Combined bytes | Median estimate | Throughput |
|---:|---:|---:|---:|
| 100 | about 14 KiB | 4.91 ms | 2.91 MiB/s |
| 1,000 | about 149 KiB | 71.31 ms | 2.05 MiB/s |
| 5,000 | about 766 KiB | 391.28 ms | 1.91 MiB/s |

These numbers establish a reproducible starting point. The next optimization pass will profile
allocations, intern kinds and fields, and add three pathological cases: deeply nested syntax, 10,000
duplicate siblings, and large unrelated byte regions.
