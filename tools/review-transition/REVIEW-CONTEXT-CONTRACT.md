# Review context, receipt, and input contract v1

These versioned Draft 2020-12 schemas define the boundary between ReviewTransition and a
downstream reviewer:

- [`review-context-v1.schema.json`](review-context-v1.schema.json) declares every input that may
  influence the reviewer;
- [`review-receipt-v1.schema.json`](review-receipt-v1.schema.json) signs the exact prior input,
  result, coverage, and context; and
- [`review-input-v1.schema.json`](review-input-v1.schema.json) emits one fail-closed routing
  decision: `skip`, `residue`, `full`, or `blocked`;
- [`review-cache-payload-v1`](../../schema/review-cache-payload-v1.schema.json) binds routing
  provenance and immutable source bytes;
- [`review-cache-reviewer-input-v1`](../../schema/review-cache-reviewer-input-v1.schema.json) is
  the exact projection an adapter may expose to the reviewer; and
- [`review-cache-result-v1`](../../schema/review-cache-result-v1.schema.json) records explicit
  completion, per-obligation outcomes, and complete coverage.

Schema validity is necessary, not sufficient. A consumer must also perform the digest, signature,
identity, and set checks below. It must reject an unknown field rather than silently ignore it.

The context, receipt, and routing schemas remain beside ReviewTransition because they define one
versioned protocol. `stratadiff review-cache-context` builds the context and its Git closure,
`stratadiff review-cache` performs preflight routing, and `stratadiff review-cache-receipt` performs
postflight validation before signing.

## Canonical bytes and digests

`stratadiff-canonical-json-v1` means UTF-8 JSON with object keys sorted lexicographically, no
insignificant whitespace, JSON strings escaped deterministically, finite numbers only, and no
trailing newline. SHA-256 values are lowercase hexadecimal over the exact bytes named by the field.

The producer computes `review-context.body_sha256` over the canonical complete `body`. That digest
is an immutable artifact binding; it is expected to change when the PR snapshot changes. The
separate `compatibility_sha256` is SHA-256 over this canonical projection:

```json
{
  "change_identity_schema": "<body.change_identity_schema>",
  "dependency_closure": "<body.dependency_closure>",
  "reuse_policy": "<body.reuse_policy>",
  "review_input_scope": "<body.review_input_scope>",
  "reviewer": "<body.reviewer>"
}
```

The angle-bracket values above mean the complete corresponding JSON values, not strings. The full
body digest therefore prevents context substitution, while the compatibility digest permits a new
Git head only when the reviewer implementation, model, prompt, policy, configuration, runtime,
tools, declared input scope, and external dependency closure remain identical.

A receipt attestation computes `body_sha256` over the canonical receipt `body`; its Ed25519
signature covers that 32-byte SHA-256 value using the key identified by `key_id`. A routing consumer
must recompute both full context and compatibility digests, recompute the receipt body digest,
verify the signature against a key trusted by the receipt body's `trust_domain`, and verify that the
applicable trust policy has the declared digest. Merely setting `trusted_source_verified` to `true`
does not establish trust.

The selected-payload digest covers the canonical provenance envelope, including base/head OIDs.
The separate reviewer-visible projection deliberately excludes those routing OIDs and is emitted as
its own file. Its digest covers the exact composition contract, repository/PR identity, and ordered
items exposed to the reviewer. An adapter must feed that projection—not the provenance envelope—to
the reviewer. The completion manifest binds both digests, so it cannot silently review one input
and sign another.

## Context closure

A context is complete only when it binds all of the following:

- reviewer implementation artifact and entry point;
- exact model revision and inference parameters, or an explicit `none` model;
- canonical prompt, policy, and configuration artifacts;
- runtime, environment, architecture, and every invoked tool;
- stable repository and pull-request identities plus canonical PR metadata;
- the complete declared Git object/path closure; and
- every historical disposition visible to the reviewer.

`reviewer.composition.kind=holistic` treats the complete reviewer-visible projection as one
indivisible input. It may be reused only when that complete projection digest is unchanged; it can
never authorize a partial residue. `itemwise_closed_v1` additionally binds an item-review contract
and a cross-item aggregation contract. Only that explicit mode may carry a strict subset of
successful or advisory item outcomes.

`review_input_scope=selected_payload_only` declares that the downstream reviewer can observe only
the selected immutable payload plus the other inputs already bound by the compatibility digest.
`declared_repository_closure` declares whole-repository access. In that mode a changed
`repository_closure.canonical_closure_sha256` forces `full`; exact per-change identities cannot hide
changes elsewhere in the tree.

`pull_request.canonical_metadata_sha256` covers reviewer-visible PR metadata such as title, body,
labels, and policy-relevant author facts. It must not include the base or head object ID, requested
base reference, or observation time: those transition-specific values are already explicit fields.
Changing reviewer-visible metadata forces `full`; merely advancing the separately bound Git head
does not manufacture a context mismatch.

`dependency_closure.status=closed` is required for `skip` or `residue`. Dynamic network lookups,
undeclared tool access, or any other external dependency that cannot be frozen is represented as
`status=open`; it is never hidden behind a placeholder digest. An open context can produce `full`
when the complete current review input is still materializable, and otherwise produces `blocked`.
An unpinned hosted model must use the `hosted_model_unpinned` model and closure branches and is never
automatically reusable.

Tools and historical dispositions are canonically ordered by their complete canonical JSON bytes.
The closure manifests are canonically ordered byte records; duplicate object IDs or paths are
forbidden by the manifest producer. A changed field produces a different context body digest.

A historical disposition matched by `exact_noninteracting_four_way_byte_replay` is evidence only.
Its `automatically_reusable` value is structurally fixed to `false`. An exact-identity disposition
may still set that value to `false`; exact identity is necessary, not sufficient, for reuse.

`stratadiff-exact-git-change-identity-v1` is the canonical object containing `status`, nullable
`similarity_percent`, nullable before/after path bytes encoded as canonical Base64, nullable
before/after six-digit Git modes, and nullable before/after full object IDs. Its identity is the
SHA-256 of that canonical object. Paths are bytes rather than display strings, and object IDs are
not replaced by content, patch, or semantic fingerprints.

## Receipt requirements

A valid receipt binds one repository and PR, the exact prior context body, its compatibility
projection, composition contract, repository-closure and historical-disposition digests,
canonical prior input, selected payload, reviewer-visible projection, canonical completion
manifest, and complete coverage. The verifier must check:

1. the receipt signature and trusted issuer policy;
2. `coverage.covered_input_sha256 == prior_input.canonical_input_sha256`;
3. the covered identities and per-obligation outcomes are exactly the identities in the prior
   selected payload;
4. both omission and unresolved-obligation arrays are empty; and
5. the receipt repository and PR identities match the current routing request;
6. the signed compatibility digest equals the recomputed current compatibility digest;
7. historical-disposition digests match; and
8. for `declared_repository_closure`, repository-closure digests also match; and
9. the adapter-observed live head still equals the routed head at receipt issuance.

A receipt does not grant GitHub approval, establish semantic safety, or authorize reuse by itself.

## Routing invariants

Every transition contains the commit and tree OID for Q, A, B, C, and D. The verifier independently
resolves those objects and checks the declared route. For `provider_attested_same_base`, A and C
must both equal Q. For `full_history_verified`, A and C must each be the sole result of the
corresponding `git merge-base --all` computation.

The four decisions are:

- `skip`: complete accounting, a trusted complete receipt, an exactly equal compatibility digest,
  an empty selected payload, no unresolved obligation, and either an unchanged holistic
  reviewer-visible projection or itemwise-safe exact carries;
- `residue`: the same receipt, compatibility, input-scope, and history gates, at least one exact
  non-blocking carry, a non-empty strict review subset, and an explicitly pinned
  `itemwise_closed_v1` composition contract;
- `full`: complete accounting with no carry; the complete current input and all obligations remain;
- `blocked`: accounting or evidence is incomplete, no carry is allowed, and no downstream input is
  declared safe.

For every `exact_carry`, the verifier checks that `current_identity_sha256` equals
`receipt_identity_sha256`, that it is covered by the trusted receipt, and that it is absent from the
selected payload. It also checks these set equations:

```text
full current identities = exact carried identities ∪ selected current identities
exact carried identities ∩ selected current identities = ∅
selected payload identities = selected current identities ∪ unresolved obligations
```

A prior `failed` or `changes_requested` item is never carried. It is selected for review again. If
an unchanged holistic input is skipped, its authenticated blocking outcome is preserved and the
CLI exits unsuccessfully instead of turning the cache hit green.

When the merge base changes, the base-drift obligation contains the complete ordered A-to-C Git
change identities and their immutable before/after blob bytes. Four commit/tree OIDs alone are not
a reviewable payload.

JSON Schema cannot express digest recomputation, signature verification, OID equality, arbitrary
set equality, or subset relationships. A producer must not label an artifact `skip` or `residue`
until those checks pass.

Four-way replay is deliberately absent from `exact_carry`. If replay evidence exists, its only
valid representation in a routing input is a `review_required` item whose reason is
`four_way_replay_evidence_only`; it remains in the selected payload. It can never authorize verdict
reuse, input omission, or `skip`.

## Validation

Run the offline schema contract tests from this directory:

```console
python3 -B -m unittest test_review_contract_schemas.py
```

The tests validate positive examples for all four routing states, reject unknown or missing fields,
and exercise the exact-identity and four-way fail-closed boundaries. When the optional Python
`jsonschema` package is unavailable, the suite uses its included subset validator for the standard
Draft 2020-12 keywords used by these schemas.
