# Merge Readiness Audit v1

This bundle freezes the first 13 live GitHub reports produced by
`stratadiff readiness-audit` at revision
`b318e730a1a2599151460694b3e96cca7ad13ae5`. Its purpose is narrow: preserve
the permission boundary and output shape discovered during product feasibility work.

Twelve public repositories were observed with `viewerPermission=READ`. Every one of those
captures is `partial` and `inconclusive` because GitHub marked the default branch protected while
classic branch protection was absent or unreadable. The owned StrataDiff control was observed with
`viewerPermission=ADMIN`; it is complete and reports that the default branch lacks an observed
pull-request or required-check gate.

This result deliberately weakens the product claim. A public-repository token with read access is
not enough for a complete effective-policy audit. The audit command is therefore an admin
onboarding/preflight surface, while exact-PR diagnosis can be a lower-permission product entry.

## Frozen result

| Observation | Count |
|---|---:|
| Reports | 13 |
| READ permission, partial/inconclusive | 12 |
| ADMIN permission, complete/action required | 1 |
| Required checks summarized | 37 |
| Pull requests sampled | 2 |
| API calls | 97 |
| Response bytes observed | 4,762,769 |

`github/docs` accounts for 34 required checks. `dcoapp/app` accounts for three and also exposes one
`required_check_source_unresolved` unknown for a Vercel context. These are diagnostic observations,
not independently labelled defects.

## Bundle layout

- `reports/` contains the 13 unmodified JSON outputs.
- `manifest.json` binds every report to repository, capture permission, expected status, verdict,
  SHA-256, binary revision, and aggregate result.
- `verify.py` is network-free and independent of the Rust implementation. It checks hashes, closed
  report shapes, summary arithmetic, claim boundaries, and the fail-closed invariant.
- `SHA256SUMS` binds the complete human-readable and executable bundle. It is an integrity check,
  not a publisher signature.

## Offline verification

```text
python3 -B benchmarks/merge-readiness-audit-v1/verify.py verify
python3 -B benchmarks/merge-readiness-audit-v1/verify.py summary
(cd benchmarks/merge-readiness-audit-v1 && sha256sum -c SHA256SUMS)
```

The verifier never calls GitHub. Permission labels are capture metadata and cannot be reconstructed
from the report payload alone.

## Claim boundary

The repositories were convenience-selected for feasibility work, not randomly sampled. The reports
are first-party outputs from the same implementation being exercised, not independent accuracy
ground truth. This bundle does not estimate how often merge queues fail, validate historical merge
safety, reproduce workflow runtime semantics, establish cost savings, or demonstrate product-market
fit. A later controlled doctor benchmark must provide labelled failure and clean-control cases.
