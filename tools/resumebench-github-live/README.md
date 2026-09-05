# ResumeBench-GitHub-Live tooling

`resumebench_github_live.py` owns the reproducible core for the five-case GitHub Live v1
diagnostic. It derives policy oracles from Git objects without reading StrataDiff output, then uses
a clean release binary only for the separate conformance evaluation.

The commands are:

- `self-test`: exercise independent patch construction and strict four-way replay primitives;
- `verify-provenance`: query each exact PR, review, Git commit object, and force-push event ID and
  require every field in that online verification contract to match the manifest;
- `materialize`: fetch only the manifest's exact Q, B, and D commit IDs, hydrate the blobs required
  by the two review ranges, pin Q/A/B/C/D refs, remove every remote, and prove offline replay;
- `generate`: recompute and write the five independent oracle files from supplied bare repositories;
- `verify`: recompute each oracle with lazy fetching disabled and require exact equality;
- `evaluate`: verify the oracles, run a clean release StrataDiff binary, and write a conformance
  evaluation;
- `freeze`: generate the oracles and canonical evaluation, write `SHA256SUMS`, and verify the bundle;
- `verify-bundle`: validate schemas, internal references, frozen totals, and every checksum without
  network or source repositories.

Typical use is:

```console
GITHUB_TOKEN=... python3 tools/resumebench-github-live/resumebench_github_live.py verify-provenance \
  --manifest benchmarks/resumebench-github-live-v1/manifest.json \
  --github-token-env GITHUB_TOKEN
python3 tools/resumebench-github-live/resumebench_github_live.py materialize \
  --manifest benchmarks/resumebench-github-live-v1/manifest.json \
  --output /tmp/resumebench-github-live-v1
python3 tools/resumebench-github-live/resumebench_github_live.py verify \
  --materialization /tmp/resumebench-github-live-v1
python3 tools/resumebench-github-live/resumebench_github_live.py evaluate \
  --materialization /tmp/resumebench-github-live-v1 \
  --stratadiff target/release/stratadiff \
  --output /tmp/resumebench-github-live-evaluation.json
```

Maintainers can replace the last command with `freeze` to rewrite the canonical oracles,
evaluation, and checksums. Normal CI needs only `self-test` and `verify-bundle`; live object
availability and provenance are checked separately by the **ResumeBench GitHub Live Canary**
workflow, not as a blocking correctness prerequisite.

Pass existing repositories either as five repeated
`--repository CASE_ID=/absolute/path/repository.git` arguments or as one `--materialization`
directory created by `materialize`. See the dataset [README](../../benchmarks/resumebench-github-live-v1/README.md)
for complete commands and the claim boundary.

An optional GitHub token is accepted only through the environment variable named by
`--github-token-env`; the token is never placed in an argument, output, artifact, remote URL, or Git
configuration file. Fetches have an explicit timeout and never fall back from a full object ID to a
branch, PR head, mirror, or alternate commit. GitHub may garbage-collect orphaned review commits, so
`materialize` can legitimately fail with upstream unavailability.
