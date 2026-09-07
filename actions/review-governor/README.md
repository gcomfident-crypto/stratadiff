# Final-Head Review Governor for CodeRabbit (alpha)

This composite Action waits for a pull request to stop moving, asks CodeRabbit for one full review,
and fails closed until three public GitHub objects appear strictly after that dispatch:

1. a top-level PR-conversation acknowledgement from CodeRabbit containing its command-invocation
   UUID and `Full review finished.` marker;
2. a `Review completed` status from CodeRabbit's GitHub App identity; and
3. a substantive submitted review from the fixed CodeRabbit bot identity whose `commit_id` is the
   expected head SHA and whose body contains CodeRabbit's review marker.

The latter two objects must be within five minutes, and the latest authenticated object in each
stream wins: a newer skipped/failed status, dismissed review, unfinished acknowledgement, or
`CHANGES_REQUESTED` result cannot fall back to older green evidence. CodeRabbit does not expose a
shared run identifier on all three objects, so this remains a conservative correlation rather than
a cryptographic receipt.

The submitted-review condition is intentional. A public CodeRabbit status can be successful even
when its PR comment says that review was automatically paused. Status-only gating would therefore
mark an unreviewed head as covered.

## Required workflow shape

Use the Action from a `pull_request_target` workflow that never checks out or executes pull-request
code. Give every PR one concurrency group so a newer input cancels the older waiting run. The
Action writes the `StrataDiff Final Head` commit status directly to the expected head SHA and
records the expected base SHA in the status description. Configure that exact context as a required
branch-protection check; the `pull_request_target` job itself is not the head-bound gate.

The example also listens for authenticated CodeRabbit review revocation and status regression. It
re-reads the latest provider object before replacing a prior Governor success with `failure`,
`error`, or `pending`; delayed webhook delivery therefore cannot revive an older negative event.

GitHub commit statuses are head-scoped, not `(base, head)`-scoped. Therefore the alpha has two
non-optional repository prerequisites: require branches to be up to date before merging (or use a
merge queue), and install the example's base-push reconciliation job. The first rule is the safety
boundary; the push job promptly overwrites stale green states and re-runs review, but is only a
best-effort backstop. Without strict freshness, a target-branch update can leave a same-head PR
mergeable during workflow queueing or API failure.

This Action is a transparent pilot control, not an exclusive production trust root. Every workflow
with `statuses: write` shares the `github-actions[bot]` source and can forge the same context. Do not
use the alpha as a security boundary in a repository with untrusted write-capable workflows. The
production gate requires a dedicated StrataDiff GitHub App and a ruleset that pins that App as the
expected status source.

Start from [`examples/review-governor-coderabbit.yml`](../../examples/review-governor-coderabbit.yml)
and replace the placeholder with an immutable StrataDiff commit SHA. CodeRabbit's own automatic
per-push reviews must be disabled for the selected repository. Otherwise the Action adds a governed
review but cannot remove the original per-push cost.

The only accepted command is the publicly verified top-level PR-conversation route
`@coderabbitai full review`. The incremental command and arbitrary handles are rejected because
they do not establish a fresh review of a changed base. StrataDiff does not claim to reconstruct or
cache CodeRabbit's private repository context, model state, or prior verdict.

## Outcomes

| Outcome | Action result | Meaning |
| --- | --- | --- |
| `dispatched-covered` | success | The governed command produced all three post-dispatch evidence objects for a still-live base/head pair. |
| `superseded` | failure on the obsolete SHA | The PR moved; no assumption is made that a newer workflow exists. |
| `deferred-draft` | pending gate | Drafts are not dispatched. `ready_for_review` must trigger another run. |
| `observed-provider-evidence` | success in `observe` mode | Head-bound provider evidence exists, but no base-bound gate is asserted. |
| `observed-uncovered` | success in `observe` mode | No write occurred and coverage is incomplete. |
| `reviewed-blocking` | failure | The exact-head bot review requested changes. |
| `timed-out-uncovered` | failure | Complete evidence did not arrive before timeout. |
| `merged-uncovered` | failure | Merge occurred without the required evidence; branch protection was not effective. |

Every invocation writes a bounded, runner-local JSON observation and exposes its path as the
`evidence` output. The artifact explicitly records that it is not signed. Upload it only when the
team needs durable diagnostics.

## Alpha limitations

- The debounce occupies a GitHub-hosted runner. It is appropriate for pilots; the hosted GitHub App
  scheduler must move this wait off-runner.
- A pending `Reviewing base …; command …` status acts only as a short-lived duplicate-dispatch
  lease. It is accepted only when its exact top-level command comment belongs to the current PR.
  A prior success is never reused because `github-actions[bot]` is not a Governor-exclusive trust
  identity. Durable receipts require the dedicated App.
- GitHub has no atomic “comment only if this SHA is still current” operation. The Action checks the
  live base and head before and after every evidence read and before and after success publication,
  but a narrow race can still start one redundant provider run. GitHub also suppresses new workflow
  events for pushes made with `GITHUB_TOKEN`; strict freshness or a merge queue remains mandatory.
- The invalidation listeners reduce stale-success time after provider revocation, but they are still
  ordinary Actions workflows and share the repository-wide `github-actions[bot]` identity. They do
  not replace a dedicated App webhook ledger or an expected-source-pinned ruleset.
- GitHub Actions matrices are capped at 256 jobs. The example first marks every affected head
  pending, then reconciles at most 256 in one push run; larger queues stay fail-closed and require
  the hosted App or operator intervention.
- A CodeRabbit command can be rate-limited or ignored. Missing evidence times out and fails closed.
- This adapter governs CodeRabbit dispatch only. Review Cache receipts for open, fully declared
  reviewers use a separate cryptographic contract.
