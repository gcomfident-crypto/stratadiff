# ReviewBench seed v1

ReviewBench seed v1 is a small, checked-in safety regression corpus for the
StrataDiff review evidence and priority classifier. It asks one deliberately narrow question:

> Does every behavior-sensitive mutation keep `review_first` attention priority, even when the
> parser-model evidence class reports an unchanged non-trivia CST?

The source of truth is
[`../reviewbench-seed-v1.json`](../reviewbench-seed-v1.json). Each case contains
an exact before/after byte pair, a native StrataDiff language, a mutation
operator, a manually justified category, and the expected evidence class. The
paired design covers Python, JavaScript, TypeScript, Rust, Java, Go, and JSON.
Adversarial Python debug-f-string, Rust/C stringification, and HTML rendering
cases guard against treating a Tree-sitter `formatting_only` result as a
behavior guarantee: each can observe spacing that the current CST model does
not retain.

## Label policy

- `syntax_preserved`: the current parser/model sees identical non-trivia
  syntax. This is an evidence class, not a general claim of semantic
  equivalence or lower review priority.
- `behavior_sensitive`: the mutation can change observable behavior. These
  cases must retain `review_first` attention priority even when their evidence
  class is `syntax_preserved`.

The checked-in integration test parses every source pair with the selected
native grammar, produces and matcher-free verifies a StrataDiff report, then
checks both its evidence class and attention priority. It also validates unique
case IDs, non-empty rationales, paired language coverage, and the presence of
multiple mutation families.

Run it with:

```text
cargo test --test reviewbench
```

## What this does not establish

This is a seed set, not evidence of production recall. It is hand-authored,
small, and intentionally isolates single-file mutations. It does not represent
the frequency or complexity distribution of real pull requests; execute code;
or broadly cover cross-file behavior, type resolution, preprocessors, generated
sources, dependency changes, build files, concurrency, or framework conventions. Passing
it proves only that these named cases obey the current conservative priority policy.

Before publishing a broad defect-detection or reviewer-time claim, expand the
benchmark with independently labeled real-world pull-request commits, hidden
holdout cases, multiple annotators and agreement reporting, mutation-project
tests, and false-safe-rate confidence intervals.
