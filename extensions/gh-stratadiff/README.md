# `gh stratadiff`

This directory contains the first personal, repository-admin-free entry point for StrataDiff.
It is a GitHub CLI script extension with three commands:

```console
gh stratadiff audit -R OWNER/REPOSITORY
gh stratadiff resume <PR>
gh stratadiff ownership-snapshot <BASE> --output ownership.json
```

`audit` examines a repository's recent merged pull requests and reports whether completed reviewer
checkpoints drifted from the final head. It is the no-checkout discovery path: with `-R`, it runs
from any directory and does not invoke Git, materialize commits, or create temporary refs. The
report distinguishes `no_eligible_reviews`, `insufficient_evidence`, `no_observed_drift`, and
`affected` instead of treating incomplete review data as a clean result.

`resume` finds the authenticated user's latest eligible completed review, binds it to the pull
request's exact base and head commits, and opens the existing local Review Resume Workbench. If an
exact commit is absent locally, the extension verifies it through GitHub and imports only that SHA
as an exact ref over authenticated HTTPS. Git transfers the reachable object closure needed to
materialize that commit; source analysis remains local.

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
gh stratadiff resume 123
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

Run the command from a checkout of the pull request's repository. The checkout does not need to be
on the pull request branch. Missing PR base, current head, and review checkpoint commits are fetched
by exact object ID without switching branches or changing worktree files. Use `--repo-dir PATH` when
invoking it elsewhere. `-R OWNER/REPO` selects a different GitHub repository, and `--reviewer LOGIN`
selects another reviewer when policy permits access to that review history.

Repository inference is performed from the canonical `--repo-dir` checkout rather than the shell's
current directory. Subsequent pull-request requests use the host-qualified repository identity, so
the default path and GitHub Enterprise hosts do not silently fall back to an unrelated checkout or
to `github.com`.

For a terminal-only session:

```console
gh stratadiff resume 123 --no-open
```

## Resume trust and failure boundary

The extension deliberately performs the following sequence:

1. Read the PR's exact base and head SHAs. For either missing object, verify the exact SHA through
   GitHub's commit API before fetching it from that repository's canonical HTTPS URL.
2. Fetch one response containing at most the first 100 reviews. The status, headers, and JSON body
   are parsed together; any pagination `Link` header fails closed before checkpoint selection.
3. Resolve one checkpoint with `stratadiff github-checkpoint`.
4. Ask GitHub for that exact checkpoint object and validate it with
   `stratadiff github-commit-object`; fetch the same exact SHA when it is absent locally.
5. Verify every imported ref resolves to the requested commit, reread the PR base and head to detect
   a concurrent push, then open `review --checkpoint --workbench`.

Provider fetches run in an isolated bare repository with inherited Git credentials, URL rewrites,
proxies, and trace settings removed. The credential header is scoped to the canonical HTTPS fetch;
the tokenless local import uses `git fetch-pack`, then atomically creates a temporary
`refs/stratadiff/resume/...` ref. It never reads or writes the caller's `FETCH_HEAD` and removes its
temporary refs on exit. Imported objects may remain in the object database, but no branch or
worktree file is changed.

If GitHub's API or HTTPS transport no longer serves an exact object, the command stops with that
SHA. It never substitutes the current head, a moving branch, or another review checkpoint.
The preliminary provider-commit checks and exact Git fetches currently rely on the configured `gh`
and Git transport timeouts; the ownership collector's per-call, response-size, and total-window
limits begin after that materialization step.

The chosen checkpoint is the latest eligible review in the single GitHub response snapshot. A
review submitted or dismissed after that response can make the snapshot stale; the command does
not claim that review history remains unchanged while the local Workbench is open.

The Workbench is review assistance, not a native approval: this command does not submit, restore,
dismiss, or otherwise modify a GitHub review.

The long-running Workbench starts with a fresh allowlisted environment. GitHub tokens, extension
state, arbitrary caller secrets, inherited Git configuration, credential helpers, proxies, and
trace settings do not reach the server or browser process. The allowlist retains only `PATH`,
`HOME`, display/session variables needed to open a browser, locale variables, and fixed defensive
Git settings.

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

Resume:

```text
--reviewer LOGIN   Reviewer login; defaults to the authenticated gh user
-R, --repo REPO    GitHub repository in [HOST/]OWNER/REPO form
--repo-dir PATH    Local Git repository; defaults to the current repository
--port PORT        Loopback viewer port; 0 chooses an available port
--no-open          Print the viewer URL without opening a browser
```

`resume` and `ownership-snapshot` require Bash, GitHub CLI, Git, `env`, `base64`, and a built
`stratadiff` executable. GitHub Enterprise hosts without a port in their HTTPS origin are supported
through the repository URL returned by `gh`; the current prototype does not accept origins with
embedded credentials or explicit ports.

## Test

The test suite uses only stubbed GitHub, Git, StrataDiff, and audit executables; it needs no token
and makes no network request. It covers all three commands, including audit from a non-Git
directory, host-qualified GHES operation, provider verification of a locally present base,
exact-object recovery, output mode, and cleanup on failure:

```console
bash tests/resume_test.sh
```

It covers successful command binding, exact-object recovery for missing current base/head and a
historical checkpoint, provider/API disappearance, temporary-ref cleanup, `FETCH_HEAD` immutability,
fetch-pack output and ref-collision failures, a PR head changing during resolution, absence of an
eligible review, bounded single-response review retrieval, and the final Workbench environment
allowlist. Audit-specific cases verify option forwarding, repository inference, backend status
propagation, input validation, and the absence of Git or temporary-ref activity.
