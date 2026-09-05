# GitHub review-coverage check

StrataDiff's first host integration answers one operational question:

> Since a named reviewer last finished a review, which current PR changes still lack carry evidence?

It is designed to complement GitHub's native approval rule. It does not create, restore, or replace
an approval.

## Check lifecycle

| Event | Resolved checkpoint | Expected coverage result |
|---|---|---|
| No completed review by the configured reviewer | None | Gate fails; the complete PR remains reviewable |
| Reviewer submits `APPROVED` or `CHANGES_REQUESTED` at head `R` | `R` | Zero residue at that exact head |
| Author pushes new content at `H` | `R` | Gate fails on every current PR file not carried from `R` |
| Stack tool rebases or restacks without interacting with reviewed edits | `R` | Exact identities and eligible four-way replays carry; upstream-only files are excluded |
| Reviewer finishes another review at `H` | `H` | The new completed review closes the residue |

`CHANGES_REQUESTED` is accepted as review coverage because GitHub documents it as a decision made
when a reviewer finishes a review. GitHub's native merge policy still blocks on that decision. A
`COMMENTED` review does not establish that the reviewer completed the PR, and a `DISMISSED` review
is excluded because the API does not reliably expose why it was dismissed. Bots, pending reviews,
and deleted users are also excluded.

The reviewer login is explicit configuration. The current resolver does not infer CODEOWNER,
organization membership, or permission to approve. Treat the configured login as policy, not as a
fact established by StrataDiff.

## Required-check workflow

Run the same job when the head changes and when the reviewer submits or dismisses a review. Pin the
Action to an immutable commit before using it as a protected-branch requirement.

```yaml
name: Review coverage

on:
  pull_request:
    types: [opened, synchronize, reopened]
  pull_request_review:
    types: [submitted, dismissed]

permissions:
  contents: read
  pull-requests: read

jobs:
  review-coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          ref: ${{ github.event.pull_request.head.sha }}
      - id: coverage
        uses: gcomfident-crypto/stratadiff@main
        with:
          base: ${{ github.event.pull_request.base.sha }}
          head: ${{ github.event.pull_request.head.sha }}
          reviewer: alice
          github-token: ${{ github.token }}
          fail-on-review-residue: true
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: stratadiff-review-coverage
          path: |
            ${{ steps.coverage.outputs.report }}
            ${{ steps.coverage.outputs.checkpoint_record }}
          if-no-files-found: error
```

The mutable action references in this example are readable previews, not supply-chain guidance.
Pin `actions/checkout`, `actions/upload-artifact`, and StrataDiff to audited full commit IDs in a
production workflow.

With `fail-on-review-residue: true`, a failed gate creates file-scoped GitHub error annotations for
the first 20 current PR files that need review. If the queue is larger, one notice reports the
omitted count while the step summary and JSON artifact retain every file. A missing checkpoint
produces one check-level error because the complete PR, rather than a bounded file subset, remains
in review. Paths and workflow-command data are escaped before emission; non-UTF-8 paths remain in
the report but are represented by a check-level annotation instead of an invalid file link.

The resolver requests at most 100 reviews from GitHub. It fails closed if pagination indicates a
larger history. It verifies the selected review SHA through GitHub's Git commit-object endpoint. If
that commit is no longer in the checkout after a force-push, the Action requests that exact SHA,
and no branch or tag, from the HTTPS repository URL derived from `github.server_url` and
`github.repository`. The
authenticated fetch runs in an isolated temporary bare repository that ignores system and user Git
configuration; the verified objects are then imported locally without the token. It never falls
back to `origin`, the current PR head, or another commit. A provider that no longer serves the exact
review commit therefore leaves the check red with an actionable error.

All API calls disable user curl configuration before loading a mode-restricted temporary request
configuration. Temporary API bodies, credentials, provider objects, and refs are removed by the
Action's exit trap. The resulting code analysis stays inside the runner.

When `reviewer` is configured, the Action also emits `checkpoint_record`: a deterministic JSON
selection record containing the chosen review ID, reviewer, state, commit ID, submission time, URL,
and author association. Upload it beside the review report if the workflow needs durable audit
history. This matters because GitHub may later retain a dismissed review while returning no commit
for it. The record is producer-attested workflow output, not a GitHub signature or a self-contained
proof of the API response; retain the workflow provenance or an artifact attestation with it.

## Local inspection

Save the response from GitHub's list pull request reviews endpoint and inspect the deterministic
selection record:

```text
stratadiff github-checkpoint reviews.json --reviewer alice --format json
```

The default output is only the selected commit ID, which can be passed directly to `review`:

```text
checkpoint="$(stratadiff github-checkpoint reviews.json --reviewer alice)"
stratadiff review BASE HEAD --checkpoint "$checkpoint" --format json --output review.json \
  --github-annotations --fail-on-review-residue
```

An empty resolver result means no eligible checkpoint. In gate mode this must remain a failed
check, not an implicit full-review approval.

## Evidence still needed

The implementation establishes deterministic checkpoint selection and residue classification. It
does not yet prove that teams save time. A design-partner pilot must record eligible PR rate,
residue size, false carries, reviewer overrides, time to decision, and issue recall. Until then the
honest claim is “reproducible review coverage,” not “faster or safer review.”
