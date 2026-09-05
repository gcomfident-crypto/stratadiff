# Review Memory Audit regression v1

This bundle is a fast, deterministic regression set for the repository-level Review Memory Audit.
It derives 24 cases from the already observed
[Review Churn Census v1](../review-churn-census-v1/README.md). Outcomes informed both the product
direction and the bucket design, so this is **post-outcome regression coverage**, not a blind set,
confirmatory study, prevalence estimate, or product-value result.

## Coverage design

The selector assigns cases in a frozen priority order and then ranks eligible cases by
`SHA-256(seed || 0x00 || case_id)`. Earlier buckets consume a case before later buckets, making the
24 assignments mutually exclusive.

| Bucket | Cases | Contract exercised |
|---|---:|---|
| `missing_oid` | 2 | A missing latest checkpoint or final-head OID remains unknown and never falls back. |
| `commented_newer` | 1 | A later `COMMENTED` session remains partial attention, not completion. |
| `dismissed` | 3 | Only a linked, temporally valid prior `APPROVED` or `CHANGES_REQUESTED` state restores completion. |
| `rewrite_heavy` | 4 | One post-review force-push case from each high-churn design-partner hypothesis repository; same-reviewer completed re-review is preferred in the deterministic rank. |
| `drift_without_force` | 4 | Latest checkpoint drift is separate from an earlier or unobserved force-push. |
| `bot_only` | 2 | Bot sessions never enter the external human peer denominator. |
| `zero_review` | 3 | A PR with no submitted review objects produces no eligible checkpoint. |
| `commented_only` | 1 | COMMENTED-only human attention is reported but never becomes a completed checkpoint. |
| `stable` | 4 | Comparable completed checkpoints equal to final head remain non-findings. |

The final stable selection first fills any repository not already represented. All ten source-panel
repositories are therefore exercised without hand-picking a visually compelling PR.

## What the oracle checks

Each golden entry pins the Census case ID, public repository and PR number, assigned bucket,
classification booleans, the relevant per-PR counts, and the semantic state of every peer-reviewer
pair. Reviewer identities remain the Census's PR-local opaque keys. No login, PR title/body, review
text, source, patch, diff, email, token, or raw API response is copied into this bundle.

The verifier checks the two source artifact hashes, the manifest-to-capture hash edge, deterministic
selection, repository coverage, every stored oracle field, and canonical JSON. It does not call the
network:

```console
python3 -B benchmarks/review-memory-audit-v1/verify.py verify
python3 -B benchmarks/review-memory-audit-v1/verify.py self-test
```

`freeze` is maintainer-only and deterministically rewrites `golden-v1.json` from the pinned Census:

```console
python3 -B benchmarks/review-memory-audit-v1/verify.py freeze
```

## Missing evidence

None of the 500 source cases required a second GitHub review or filtered-timeline page. Pagination
success and corrupt-page refusal therefore need controlled transport fixtures, plus at least one
prospectively frozen real multi-page case before making a live-coverage claim. The panel also has
individual zero-review PRs, not a repository selected for zero review activity.

A later generalization test must freeze new repositories, a non-overlapping time window, selection
rules, and acceptance thresholds before capture. Passing this regression bundle or the full 500-case
shadow replay cannot establish reviewer time savings, issue recall, safe carry, willingness to pay,
retention, or product-market fit.
