# StrataDiff Final-Head Review Governor (hosted App MVP)

This directory is an independent Node 24 + TypeScript control plane for a **dedicated StrataDiff
GitHub App**. It receives GitHub webhooks, immediately revokes the desired final-head gate in
PostgreSQL, dispatches one CodeRabbit full review, correlates public provider evidence, and creates
or updates the fixed `StrataDiff Final Head` Check Run with an installation token.

It is intentionally separate from the Action pilot. Do not give an Actions workflow or another App
the same gate identity: the ruleset must pin this dedicated App as the expected source.

## GitHub App configuration

Grant the App these repository permissions:

- Checks: read and write
- Issues: read and write (the CodeRabbit command is a top-level PR conversation comment)
- Pull requests: read
- Commit statuses: read
- Metadata: read

Subscribe to `pull_request`, `pull_request_review`, `status`, `issue_comment`, `push`, and
`merge_group`. Point the webhook URL to `POST /webhooks/github`; `/healthz` is the only other HTTP
endpoint. Use a strong webhook secret and keep the App private to the intended installations while
validating the MVP.

Configure a repository ruleset with `StrataDiff Final Head` as a required status check. Set the
ruleset's `integration_id` to the numeric **GitHub App ID** for this dedicated App (not an
installation ID and not GitHub Actions). The ID can be confirmed from the App settings or the
`app.id` on a Check Run returned by GitHub's Checks API.

For direct PR merging, also require branches to be up to date before merging (strict status
checks). A merge queue is the preferred alternative. With a merge queue, the `merge_group`
`head_sha` is a separate and final gate subject: this service writes a new Check Run on that SHA and
never treats a green PR-head check as the merge-group check. The current MVP deliberately leaves a
new merge-group gate in progress because a merge-group-native review-evidence adapter is not yet
implemented; it fails closed instead of borrowing PR evidence.

## Database and delivery model

`migrations/001_initial.sql` creates:

- `webhook_delivery`, keyed by `X-GitHub-Delivery`, for HMAC-verified idempotency;
- `pr_pair`, uniquely binding repository, PR, base, and head with monotonic epoch and fence fields;
- `dispatch` and `evidence`, both bound to the pair epoch;
- `gate_subject`, separating PR and merge-group Check Runs; and
- `outbox`, so webhook projection and work scheduling commit atomically.

Each relevant delivery first changes the desired gate to `revoked` in the same transaction that
records the event. A revoked gate publishes as `in_progress`, which cannot satisfy a required
check. Workers acquire expiring leases with monotonically increasing fencing tokens. Final commits
compare pair/subject ID, epoch, fence, owner, and expiry; an expired worker cannot commit after a
new event or worker advances the fence. Evidence updates are source-time ordered, and equal-time
deletion/dismissal tombstones cannot be replaced by delayed positive deliveries.

Before posting the provider command, the worker durably moves a dispatch from `planned` to
`attempting`. Once that transition commits, retries only search for and adopt the exact-body App
comment; they never issue another paid command. This makes an uncertain POST at-most-once. A crash
after the transition but before the request leaves the process intentionally remains fail-closed
and needs operator redrive.

Push handling revokes every currently open subject and queues a repository reconciliation. The
GitHub client requests open PRs in 100-item pages until an actually short page is returned; there is
no 256-PR matrix ceiling.

## Run

All configuration is required; the process has no implicit backend or credential fallbacks.

| Variable | Meaning |
| --- | --- |
| `PORT` | HTTP listen port |
| `DATABASE_URL` | PostgreSQL connection URL |
| `GITHUB_WEBHOOK_SECRET` | GitHub App webhook secret |
| `GITHUB_APP_ID` | Dedicated App's numeric ID |
| `GITHUB_PRIVATE_KEY` | PEM key; literal `\n` sequences are accepted |
| `GITHUB_API_URL` | HTTPS API root, normally `https://api.github.com` |
| `GITHUB_REQUEST_TIMEOUT_MS` | Hard timeout for every GitHub API call |
| `WORKER_ID` | Unique process/worker identity |
| `WORKER_POLL_MS` | Empty-outbox polling interval |
| `LEASE_SECONDS` | Pair, gate, and outbox lease duration |
| `OUTBOX_RETRY_SECONDS` | Delay after a failed outbox attempt |
| `EVIDENCE_TIMEOUT_SECONDS` | Deadline for complete post-dispatch provider evidence |

```sh
npm ci
npm run build
DATABASE_URL=postgresql://... npm run migrate
set -a; . ./.env; set +a
npm start
```

`npm start` also applies pending migrations before listening. Run multiple identical processes for
availability; outbox and pair fencing coordinate them through PostgreSQL.

## Verification and current boundary

```sh
npm test
npm run build
```

The tests use an in-memory PostgreSQL-compatible adapter and injected GitHub transport. They make
no calls to GitHub and cover HMAC verification, delivery deduplication, stale delivery ordering,
same-SHA PR isolation, immediate revocation, lease fencing, merge-group head binding, Checks API
identity, and pagination beyond 300 open PRs.

Still not done: a live GitHub App installation E2E, a live PostgreSQL concurrency soak, delivery
redrive/dead-letter operations, an operator UI, and merge-group-native provider
evidence. Those are deployment gates beyond this runnable development MVP; do not claim production
validation until the live installation/ruleset/merge-queue paths have been exercised.

Two cross-system boundaries are also intentionally unresolved. A correctly signed body that is
not valid JSON, or whose repository identity cannot be decoded, cannot yet be routed to a specific
repository and therefore returns `400` without globally revoking existing gates. Repository
reconciliation also lacks a generation compare-and-swap between its GitHub list snapshot and its
database write. Finally, GitHub Checks offers no transaction shared with PostgreSQL, so a stale
publisher can only be detected and compensated after an external write; the service reuses the
canonical Check Run for compensation, but this is not proof of a zero-duration green race. These
are explicit blockers for deployment as a required merge control.

The unit suite's PostgreSQL adapter does not implement `FOR UPDATE SKIP LOCKED`. Before deploying
multiple replicas, run a real-PostgreSQL integration gate that races outbox claims, lease expiry,
epoch changes, and process death around the comment POST. The SQL uses row locking and fenced
updates in production, but the unit adapter must not be presented as proof of those lock semantics.
