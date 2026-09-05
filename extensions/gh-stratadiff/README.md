# `gh stratadiff`

This directory contains the personal, repository-admin-free GitHub CLI entry point for StrataDiff.
The launcher exposes five commands; `resume` is implemented by the native Rust binary and the
extension passes its arguments and exit status through unchanged:

```console
gh stratadiff audit -R OWNER/REPOSITORY
gh stratadiff demo
gh stratadiff inbox -R OWNER/REPOSITORY
gh stratadiff resume <PR>
gh stratadiff ownership-snapshot <BASE> --output ownership.json
```

`audit` examines a repository's recent merged pull requests and reports whether completed reviewer
checkpoints drifted from the final head. It is the no-checkout discovery path: with `-R`, it runs
from any directory and does not invoke Git, materialize commits, or create temporary refs. The
report distinguishes `no_eligible_reviews`, `insufficient_evidence`, `no_observed_drift`, and
`affected` instead of treating incomplete review data as a clean result. Audit v2 findings bind
each actionable reviewer login to an immutable GitHub user node ID, and identity gaps or conflicts
fail closed.

`demo` creates a deterministic A/B/C/D Git history entirely in a temporary directory and opens the
Review Resume Workbench without contacting GitHub. An upstream base edit and a previously reviewed
author edit are reconstructed, leaving exactly one post-review line in the queue. The temporary
history is removed when the Workbench stops.

`inbox` is the daily, reviewer-specific path. It finds open pull requests where the authenticated
GitHub user completed a review and the current head differs from that exact checkpoint. Each
actionable item carries the exact `gh stratadiff resume` invocation needed to continue. With `-R`,
it also runs outside a Git checkout and does not invoke Git, materialize commits, or require a local
`stratadiff` binary. It requests no source, diff, title, body, review text, comment text, or commit
message; incomplete pagination and missing or inconsistent identities fail closed.
The viewer and review authors are bound by immutable GitHub node ID as well as login. Every
eligible candidate is fetched again before output, and total API calls, captured nodes, response
bytes, and wall time are bounded. GitHub does not provide one atomic repository-wide snapshot, so
changes to otherwise ineligible PRs during the recorded observation window may appear on the next
run. The unfiltered PR review count is checked separately, so Inbox never emits a command that
would exceed Resume's shared 10,000-review limit.

Native Rust `resume` finds the authenticated user's latest eligible completed review, binds it to
the pull request's exact base and head commits, and opens the local Review Resume Workbench. If an
exact commit is absent locally, it verifies the commit through GitHub and imports only that SHA as
an exact ref over authenticated HTTPS. Git transfers the reachable object closure needed to
materialize that commit; source analysis remains local. With `-R` and no `--repo-dir`, the target
object store is an isolated temporary bare repository, so the first run needs no checkout.

`ownership-snapshot` loads CODEOWNERS from one exact local base commit, verifies that commit against
the selected GitHub host, and delegates live identity, team-membership, and repository-permission
collection to StrataDiff. The resulting private snapshot is ready for `review-coverage`.

## Install from this checkout

Build StrataDiff, then install the extension as a local development symlink:

```console
scripts/build-release.sh --bin stratadiff
cd extensions/gh-stratadiff
gh extension install .
```

Point the extension at the binary from this checkout:

```console
export STRATADIFF_BIN="$(git rev-parse --show-toplevel)/target/release/stratadiff"
gh stratadiff demo
gh stratadiff resume 123 -R OWNER/REPOSITORY
```

To audit the most recent 50 merged pull requests in a 90-day window:

```console
gh stratadiff audit -R OWNER/REPOSITORY
gh stratadiff audit -R HOST/OWNER/REPOSITORY \
  --limit 100 --days 180 --format json --output review-memory-audit.json
```

Omit `-R` to let `gh repo view` infer the repository from the current directory. Use
`--end-exclusive RFC3339` to make the half-open audit window reproducible. Audit requires only
Bash, Python 3, GitHub CLI, and authenticated read access to the selected repository.

To inspect your current review-resume queue:

```console
gh stratadiff inbox -R OWNER/REPOSITORY
gh stratadiff inbox -R HOST/OWNER/REPOSITORY \
  --format json --output review-inbox.json
```

Omit `-R` to let `gh repo view` infer the repository. Like `audit`, `inbox` can run from a non-Git
directory when the repository is explicit. The authenticated `gh` user defines whose completed
reviews are examined.

To collect the ownership input for a coverage Passport:

```console
gh stratadiff ownership-snapshot "$BASE_SHA" \
  --repo-dir "$(git rev-parse --show-toplevel)" \
  -R github.example.com/OWNER/REPOSITORY \
  --output ownership.json
```

`-R` accepts either `OWNER/REPO` or the host-qualified `HOST/OWNER/REPO` form. Every provider API
request remains bound to the canonical host returned for that repository. The base must be a full
lowercase object ID; a locally present object is still checked against GitHub, while an absent one
is fetched by exact object ID through the same isolated transport used by `resume`.

For unattended collection, provide a GitHub App installation token through `GH_TOKEN` (GitHub.com)
or `GH_ENTERPRISE_TOKEN` (GHES). The minimum App permissions are repository `Contents: read` and
`Metadata: read`, plus organization `Members: read` when CODEOWNERS contains teams. Do not rely on
the built-in Actions `GITHUB_TOKEN` for team ownership because it cannot generally read organization
membership. The collector queries effective permission only for referenced principals; it does not
download the repository's complete collaborator list.

Resume selects its repository mode only from the presence of `-R` and `--repo-dir`:

| `-R` | `--repo-dir` | Repository behavior |
| --- | --- | --- |
| omitted | omitted | Use the current Git worktree or bare repository and let `gh repo view` infer GitHub coordinates. |
| omitted | set | Use that local Git worktree or bare repository and infer GitHub coordinates from it. |
| set | omitted | Create an isolated temporary bare repository and use the explicit GitHub coordinates. |
| set | set | Use that local Git worktree or bare repository and the explicit GitHub coordinates. |

There is no fallback between modes: an invalid explicit `--repo-dir` fails, and a non-Git current
directory fails unless `-R` is supplied. `--repo-dir` accepts either a worktree or a bare Git
repository. The local modes do not need to be on the pull request branch. Missing PR base, current
head, and review checkpoint commits are fetched by exact object ID without switching branches or
changing worktree files. `--reviewer LOGIN` selects another reviewer when policy permits access to
that review history. Resume currently selects review history by GitHub login; unlike Inbox's
discovery record, it does not bind that selection to a stored immutable user node ID. Subsequent
pull-request requests use the host-qualified repository identity, so GitHub Enterprise hosts do not
silently fall back to `github.com`.

For a terminal-only session:

```console
gh stratadiff resume 123 --no-open
```

## Resume trust and failure boundary

The native Rust command deliberately performs the following sequence:

1. Read the PR's exact base and head SHAs. For either missing object, verify the exact SHA through
   GitHub's commit API before fetching it from that repository's canonical HTTPS URL.
2. Fetch every review page with `gh api --paginate --slurp`. The resolver accepts at most 10,000
   reviews and 32 MiB of page JSON; either limit fails closed before checkpoint selection.
3. Resolve one checkpoint with the same bounded policy exposed by `stratadiff github-checkpoint`.
4. Ask GitHub for that exact checkpoint object and validate the response against the selected SHA;
   fetch the same exact SHA when it is absent locally.
5. Verify every imported ref resolves to the requested commit, reread the PR base and head to detect
   a concurrent push, then launch an isolated native Workbench child.

Provider fetches run in an isolated bare repository with inherited Git credentials, URL rewrites,
proxies, and trace settings removed. The credential header is scoped to the canonical HTTPS fetch;
the tokenless local import uses `git fetch-pack`, then atomically creates a temporary
`refs/stratadiff/resume/...` ref. It never reads or writes the caller's `FETCH_HEAD` and removes its
temporary refs and every fetch-pack keep file that still carries this process's ownership marker.
If another process rewrites or takes ownership of a keep file, Resume preserves it instead of
risking deletion of foreign Git state. In a local mode, imported objects may remain in the selected
object database, but no branch or worktree file is changed. In the `-R`-only mode, the target object
database is also temporary and is removed in full when the command exits.

If GitHub's API or HTTPS transport no longer serves an exact object, the command stops with that
SHA. It never substitutes the current head, a moving branch, or another review checkpoint.
Native Resume bounds captured child-process output and applies explicit command timeouts: local
metadata and verification commands use a 30-second limit, while review retrieval and exact Git
transport use a two-minute limit. These are per-process bounds rather than one end-to-end deadline.
Because exact Git fetches deliberately ignore inherited proxy and custom-CA configuration, a GHES
host must currently be reachable directly and trusted by the operating system certificate store.

The chosen checkpoint is the latest eligible review in the complete bounded, paginated GitHub
response (up to 10,000 reviews and 32 MiB). A review submitted or dismissed after that response can
make the snapshot stale; the command does not claim that review history remains unchanged while the
local Workbench is open.

The Workbench is review assistance, not a native approval: this command does not submit, restore,
dismiss, or otherwise modify a GitHub review.

The long-running Workbench starts with a fresh allowlisted environment. GitHub tokens, extension
state, arbitrary caller secrets, inherited Git configuration, credential helpers, proxies, and
trace settings do not reach the server or browser process. The allowlist retains only `PATH`,
`HOME`, display/session variables needed to open a browser, locale variables, and fixed defensive
Git settings.

On Unix, native Resume listens for SIGINT, SIGTERM, and SIGHUP, forwards the signal to the active
child process group, waits for bounded graceful shutdown, and then cleans temporary refs, pack keep
files, credentials, and scratch state. It exits with `128 + signal`; a child that does not stop in
the grace period is killed and reaped.

## Options

Audit:

```text
-R, --repo REPO          GitHub repository in [HOST/]OWNER/REPO form
--limit N                Maximum merged pull requests to inspect; defaults to 50
--days D                 Lookback window in days; defaults to 90
--format markdown|json   Report format; defaults to markdown
--output PATH            Write the report to PATH instead of stdout
--end-exclusive RFC3339  Fixed UTC end of the half-open audit window
```

Inbox:

```text
-R, --repo REPO          GitHub repository in [HOST/]OWNER/REPO form
--format markdown|json   Report format; defaults to markdown
--output PATH            Write the report to PATH instead of stdout
```

Demo:

```text
--port PORT        Loopback viewer port; 0 chooses an available port
--no-open          Print the viewer URL without opening a browser
```

Resume:

```text
--reviewer LOGIN   Reviewer login; defaults to the authenticated gh user
-R, --repo REPO    GitHub repository in [HOST/]OWNER/REPO form
--repo-dir PATH    Use an existing local Git worktree or bare repository
--port PORT        Loopback viewer port; 0 chooses an available port
--no-open          Print the viewer URL without opening a browser
```

`demo` requires Bash, Git, `env`, and a built `stratadiff` executable, but it does not require a
checkout, GitHub authentication, or network access. Native `resume` additionally requires Git and
GitHub CLI; credential encoding is built into the binary. The Bash `ownership-snapshot` path also
requires the external `base64` utility. GitHub Enterprise hosts without a port in their HTTPS origin
are supported through the repository URL returned by `gh`; origins with embedded credentials or
explicit ports are not accepted.

## Test

The shell contract suite uses stubbed GitHub, Git, StrataDiff, and Python executables; it needs no
token and makes no network request. Resume's shell coverage is intentionally limited to proving
that the extension delegates argv and the native exit status unchanged. The native Rust integration
suite owns Resume behavior:

```console
bash tests/resume_test.sh
cargo test --test resume_cli
cargo test --test gh_extension
```

`resume_cli` uses real local Git object stores and a stubbed GitHub CLI without network access. It
covers all four option-selected repository modes (including an existing bare repository),
host-qualified GHES operation, default and explicit reviewers, exact-object recovery, canonical PR
URL binding, double-snapshot drift rejection, `FETCH_HEAD` immutability, temporary ref and pack-keep
cleanup, Workbench environment isolation, and signals delivered both during Git mutations and after
the Workbench is ready.
`gh_extension` retains an independent real-Git check of fetch-pack keep records, compare-and-swap
refs, and `FETCH_HEAD`. Audit-specific shell cases verify option forwarding, repository inference,
backend status propagation, input validation, and the absence of Git or temporary-ref activity.
Inbox-specific cases verify explicit and inferred repositories, backend output and status
propagation, strict argument validation, and the same no-Git/no-StrataDiff boundary.
