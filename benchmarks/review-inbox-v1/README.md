# Review Inbox v1 dual-API public-metadata seed

This bundle is a small, prospective correctness check for the personal Review Inbox contract. It
freezes three open pull-request/reviewer pairs from `PostHog/posthog` using two separately acquired
GitHub metadata views:

- `rest-observation-v1.json` records GitHub REST v3 pull-request metadata and the target
  reviewer's records after full REST review pagination and local identity filtering.
- `graphql-observation-v1.json` records the GraphQL v4 fields and author-filtered review
  connection used by the product path.
- `oracle-v1.json` binds both observation files by SHA-256 and records the verifier's
  independently derived comparison. It is not itself an API observation.

Both observations retain the complete target-reviewer history needed to select the latest completed
checkpoint. They contain no source code, diff, patch, PR text, review text, commit message, email, or
credential.

## Captured evidence

REST was captured from `2026-09-05T14:56:48Z` through `14:56:59Z`; GraphQL was captured from
`14:56:59Z` through `14:57:04Z`. The 16-second combined window is below the protocol's
120-second limit.

| PR | Reviewer | Latest completed checkpoint | Head at dual capture | Result |
|---|---|---|---|---|
| `PostHog/posthog#95462` | `andrewm4894` | `3c2ad3361a98cd34bcea07a8acba71144b9c86ca` | `c209bc8884d9921f417b294e8e2b358aefc7f505` | actionable |
| `PostHog/posthog#93146` | `andrewm4894` | `2d9ca8a970604c110936ee10b790649db5f52f04` | `a082ff535976df52c41cf4a11bb4b88cd3fa2a94` | actionable |
| `PostHog/posthog#95295` | `sakce` | `370a6649f35c5e7f25a2bdd83f4dc19516895a4e` | same | up to date |

For `#95462`, each API returned 44 reviews by the target reviewer. The selected approval is followed
by 43 `COMMENTED` reviews. Selecting the latest review object would therefore lose the formal
checkpoint; selecting the latest `APPROVED` or `CHANGES_REQUESTED` record by
`(submitted_at, database_id)` preserves it.

The earlier one-file pilot capture at `2026-09-05T14:24:00Z` observed `#95462` at
`3e39bb0b87a36b7e000393f446c3042690c943c7`. The pull request advanced before this dual capture,
and both new observations now record `c209bc8884d9921f417b294e8e2b358aefc7f505`. The prior head
was not silently carried forward.

## What verification proves

`verify.py` parses the REST and GraphQL representations through separate validators. For each API
it reconstructs reviewer identity, normalizes the full target-reviewer history, chooses the latest
completed checkpoint, and classifies checkpoint versus head. It then compares:

- immutable reviewer identity;
- normalized review history;
- selected checkpoint, including review ID and commit OID;
- current head OID;
- resulting `actionable`, `up_to_date`, or `unobservable` classification.

The verifier also checks canonical JSON, exact schemas, protocol and observation hashes, pagination
completion, the capture-span limit, privacy exclusions, duplicate review IDs, summary recomputation,
and tamper cases. Run it offline:

```console
python3 -B benchmarks/review-inbox-v1/verify.py verify
python3 -B benchmarks/review-inbox-v1/verify.py self-test
(cd benchmarks/review-inbox-v1 && sha256sum -c SHA256SUMS)
```

## Limits on the claim

This is a convenience-selected three-case seed found during API feasibility work. It verifies that
the two API representations agree for these frozen records; it does not estimate how often stale
reviews occur or demonstrate time savings, defect recall, safety, retention, willingness to pay, or
product-market fit.

REST v3 and GraphQL v4 are separate API representations backed by the same GitHub system, not
independent ground-truth providers. The GraphQL capture reproduces the product fields and reviewer
filter, but is not an authenticated-viewer CLI run because the public reviewers are not the capture
account. The captures are sequential, not atomic. Timestamps and the bounded capture span expose
that limitation but cannot eliminate concurrent changes.

Before broader adoption claims, freeze a new protocol and collect at least 30 cases across multiple
repositories, including real `CHANGES_REQUESTED`, missing-OID, outer-pagination, and
review-pagination cases. Product value additionally requires observed reviewer sessions measuring
Inbox-to-Resume conversion, repeat weekly use, review time, and issue recall.
