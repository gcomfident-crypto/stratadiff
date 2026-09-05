import type { FileSessionPayload, NodeRef, RepositorySessionPayload, ReviewCoverageSessionPayload, ReviewFile } from '../types'

const digest = 'a'.repeat(64)

function node(id: number, kind: string, startByte: number, endByte: number): NodeRef {
  return {
    id,
    kind,
    named: true,
    extra: false,
    missing: false,
    span: {
      start_byte: startByte,
      end_byte: endByte,
      start: { row: 0, column: startByte },
      end: { row: 0, column: endByte },
    },
    subtree_size: 1,
    syntax_hash: digest,
    shape_hash: digest,
  }
}

export function sessionFixture(): FileSessionPayload {
  const beforeText = 'const before = 1\n'
  const afterText = 'const after = 2\n'
  return {
    kind: 'file_diff',
    report: {
      schema: 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v3.schema.json',
      engine_version: '0.3.0',
      before: { path: 'before.ts', byte_len: beforeText.length, blake3: digest },
      after: { path: 'after.ts', byte_len: afterText.length, blake3: digest },
      parser: {
        engine: 'tree-sitter',
        runtime_version: '0.27.0',
        language: 'typescript',
        grammar_name: 'tree-sitter-typescript',
        grammar_version: '0.23.2',
        grammar_abi: 14,
        node_types_blake3: digest,
        coordinate_unit: 'zero_based_row_utf8_byte_column',
        root_kind: 'program',
        before_nodes: 5,
        after_nodes: 5,
        error_free: true,
      },
      relations: [{
        before: node(0, 'program', 0, beforeText.length),
        after: node(0, 'program', 0, afterText.length),
        predicate: 'input_pair',
        correspondence: 'input_pair',
        evidence: ['caller_supplied_file_pair'],
      }],
      ambiguities: [{
        parent_before: 0,
        parent_after: 0,
        before: [node(1, 'identifier', 6, 12), node(2, 'identifier', 6, 12)],
        after: [node(1, 'identifier', 6, 11), node(2, 'identifier', 6, 11)],
        constraint: { kind: 'symbolic_abstention', cause: 'duplicate_symmetry', pair_claims: 'none' },
        reason: 'Repeated nodes are indistinguishable from the snapshots.',
      }],
      changes: [{
        kind: 'model_forced_update',
        before: node(1, 'identifier', 6, 12),
        after: node(1, 'identifier', 6, 11),
        detail: 'Shape-compatible identifier changed.',
      }],
      patch: {
        algorithm: 'bounded-patience-lines+bounded-byte-refinement-v2',
        edits: [{ old_start: 6, old_end: 16, replacement_base64: btoa('after = 2') }],
      },
      certificate: {
        before_blake3: digest,
        after_blake3: digest,
        reconstructed_blake3: digest,
        before_len: beforeText.length,
        after_len: afterText.length,
        patch_verified: true,
      },
      summary: {
        model_forced_relations: 0,
        suggested_relations: 0,
        ambiguity_groups: 1,
        structural_changes: 1,
      },
    },
    verification: { verified: true, message: 'Diff evidence verified and the rebuilt target matched.' },
  }
}

function reviewFile(path: string, state?: ReviewFile['checkpoint_state']): ReviewFile {
  return {
    status: 'modified',
    before_path: path,
    before_path_encoding: 'utf8',
    after_path: path,
    after_path_encoding: 'utf8',
    before_mode: '100644',
    after_mode: '100644',
    before_blob: digest,
    after_blob: 'b'.repeat(64),
    before_bytes: 17,
    after_bytes: 16,
    line_change_envelope: { additions: 1, deletions: 1 },
    language: 'typescript',
    priority: 'review_first',
    lane: 'review_first',
    checkpoint_state: state,
    checkpoint_match_basis: state === 'unchanged_since_checkpoint' ? 'exact_git_change_identity' : undefined,
    reason: 'the single-file diff patch rebuilt the target bytes exactly; a structural delta remains in the first pass',
    evidence: {
      report_blake3: 'c'.repeat(64),
      replay_check_passed_during_analysis: true,
      model_forced_relations: 2,
      suggested_relations: 0,
      ambiguity_groups: 0,
      byte_edits: 1,
      changes: {
        insertions: 0,
        deletions: 0,
        equivalent_relocations: 0,
        child_order_changes: 0,
        model_forced_updates: 1,
        suggested_updates: 0,
        formatting_only: 0,
      },
    },
  }
}

export function repositorySessionFixture(): RepositorySessionPayload {
  const checkpoint = '1'.repeat(40)
  const head = '2'.repeat(40)
  const base = '0'.repeat(40)
  const currentFiles = [
    reviewFile('src/changed.ts', 'needs_review_now'),
    reviewFile('src/new-risk.ts', 'needs_review_now'),
    reviewFile('src/carried.ts', 'unchanged_since_checkpoint'),
  ]
  const resumeFiles = [
    reviewFile('src/changed.ts'),
    { ...reviewFile('src/retired.ts'), status: 'deleted' as const, after_path: undefined, after_path_encoding: undefined, after_blob: undefined, after_mode: undefined, after_bytes: undefined, evidence: undefined },
  ]
  const resumeEntries = resumeFiles.map((file) => ({
    file,
    baseline_basis: 'checkpoint_snapshot' as const,
    before_source: file.before_blob === undefined
      ? { kind: 'empty' as const }
      : { kind: 'git_object' as const, commit: checkpoint, object_id: file.before_blob, byte_len: file.before_bytes },
    after_source: file.after_blob === undefined
      ? { kind: 'empty' as const }
      : { kind: 'git_object' as const, commit: head, object_id: file.after_blob, byte_len: file.after_bytes },
  }))
  return {
    kind: 'repository_review',
    review: {
      schema: 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-v1.schema.json',
      engine_version: '0.3.0',
      requested_base: 'main',
      requested_head: 'HEAD',
      base_commit: base,
      head_commit: head,
      comparison: 'merge_base_to_head',
      checkpoint: {
        requested_revision: 'reviewed',
        commit: checkpoint,
        base_commit: base,
        match_basis: 'exact_git_change_identity',
      },
      summary: {
        changed_files: 3,
        first_pass_files: 3,
        review_first_files: 3,
        syntax_preserved_files: 0,
        content_preserved_files: 0,
        unverified_files: 0,
        replay_check_passed_files: 3,
        replay_check_not_run_files: 0,
        line_envelope_complete: true,
        changed_line_envelope: 6,
        first_pass_line_envelope: 6,
        checkpoint: {
          needs_review_now_files: 2,
          unchanged_since_checkpoint_files: 1,
          retired_change_count: 1,
        },
      },
      files: currentFiles,
    },
    resume_delta: {
      schema: 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json',
      engine_version: '0.3.0',
      comparison: 'checkpoint_to_head',
      old_base_commit: base,
      checkpoint_commit: checkpoint,
      current_base_commit: base,
      head_commit: head,
      summary: {
        displayable_files: 2,
        unresolved_retired_changes: 0,
        needs_review_files: 2,
        gate_passed: false,
      },
      entries: resumeEntries,
      unresolved_retired_changes: [],
    },
    base_drift: {
      status: 'not_applicable',
      message: 'The checkpoint and current review use the same merge base.',
    },
    assessment: {
      status: 'producer_attested',
      basis: 'exact_git_change_identity',
      message: 'Checkpoint carry-forward is exact but does not establish semantic safety.',
    },
  }
}

export function repositoryBaseDriftSessionFixture(): RepositorySessionPayload {
  const payload = repositorySessionFixture()
  const oldBase = '3'.repeat(40)
  payload.review.checkpoint!.base_commit = oldBase
  payload.review.checkpoint!.match_basis = 'exact_git_change_identity_or_noninteracting_four_way_byte_replay'
  payload.resume_delta.comparison = 'per_file_review_baseline_to_head'
  payload.resume_delta.old_base_commit = oldBase
  payload.assessment.basis = 'exact_git_change_identity_or_noninteracting_four_way_byte_replay'
  const baseFile = reviewFile('src/upstream-context.ts')
  payload.base_drift = {
    status: 'available',
    message: 'Exact old-base to current-base context. These upstream changes are context only.',
    delta: {
      schema: payload.resume_delta.schema,
      engine_version: payload.resume_delta.engine_version,
      comparison: 'checkpoint_to_head',
      old_base_commit: oldBase,
      checkpoint_commit: oldBase,
      current_base_commit: oldBase,
      head_commit: payload.review.base_commit,
      summary: {
        displayable_files: 1,
        unresolved_retired_changes: 0,
        needs_review_files: 1,
        gate_passed: false,
      },
      entries: [{
        file: baseFile,
        baseline_basis: 'checkpoint_snapshot',
        before_source: { kind: 'git_object', commit: oldBase, object_id: baseFile.before_blob!, byte_len: baseFile.before_bytes },
        after_source: { kind: 'git_object', commit: payload.review.base_commit, object_id: baseFile.after_blob!, byte_len: baseFile.after_bytes },
      }],
      unresolved_retired_changes: [],
    },
  }
  return payload
}

export function reviewCoverageSessionFixture(): ReviewCoverageSessionPayload {
  const base = '0'.repeat(40)
  const head = '1'.repeat(40)
  return {
    kind: 'review_coverage_passport',
    verification: {
      verified: true,
      message: 'The signature and coverage decision were recomputed from exact offline Git objects.',
    },
    passport: {
      schema: 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-coverage-v1.schema.json',
      body: {
        engine_version: '0.3.0',
        protected_base_commit: base,
        merge_base_commit: base,
        head_commit: head,
        codeowners_source: {
          base_commit: base,
          path: '.github/CODEOWNERS',
          blob_oid: '2'.repeat(40),
          byte_len: 72,
          blake3: digest,
        },
        ledger: {
          provider_url: 'https://github.com',
          repository: { id: 1, node_id: 'R_1', full_name: 'acme/payments' },
          pull_request: { id: 2, node_id: 'PR_2', number: 42 },
          receiver: { algorithm: 'ed25519', key_id: 'receiver-1', public_key: digest },
          review_receipts: [{ review_id: 101 }, { review_id: 102 }],
          dismissals: [],
        },
        ownership: {
          provider_url: 'https://github.com',
          repository_id: 1,
          base_commit: base,
          observed_at: '2026-09-05T12:00:00Z',
        },
        checkpoint_proofs: [{ checkpoint_commit: '3'.repeat(40) }],
        files: [
          {
            scope: 'current_change',
            change: { status: 'modified', before_path: 'payments/charge.ts', after_path: 'payments/charge.ts' },
            path: 'payments/charge.ts',
            path_encoding: 'utf8',
            matching_rule: {
              line: 2,
              pattern: '/payments/',
              owner_alternatives: [{ kind: 'team', organization: 'acme', slug: 'payments' }],
            },
            owner_alternatives: [{
              owner: { kind: 'team', organization: 'acme', slug: 'payments' },
              eligible_reviewer_ids: [11],
              active_review_ids: [101],
              covering_review_ids: [],
              blockers: [],
            }],
            state: 'needs_review',
            reason: 'The current Payments change differs from its reviewed checkpoint.',
          },
          {
            scope: 'current_change',
            change: { status: 'modified', before_path: 'security/policy.ts', after_path: 'security/policy.ts' },
            path: 'security/policy.ts',
            path_encoding: 'utf8',
            matching_rule: {
              line: 3,
              pattern: '/security/',
              owner_alternatives: [{ kind: 'team', organization: 'acme', slug: 'security' }],
            },
            owner_alternatives: [{
              owner: { kind: 'team', organization: 'acme', slug: 'security' },
              eligible_reviewer_ids: [12],
              active_review_ids: [102],
              covering_review_ids: [102],
              blockers: [],
            }],
            state: 'covered',
            reason: 'The complete Git change identity is covered by review 102.',
          },
          {
            scope: 'retired_residue',
            change: { status: 'modified', before_path: 'legacy/removed.ts', after_path: 'legacy/removed.ts' },
            path: 'legacy/removed.ts',
            path_encoding: 'utf8',
            matching_rule: {
              line: 4,
              pattern: '/legacy/',
              owner_alternatives: [{ kind: 'user', login: 'maintainer' }],
            },
            owner_alternatives: [{
              owner: { kind: 'user', login: 'maintainer' },
              eligible_reviewer_ids: [],
              active_review_ids: [],
              covering_review_ids: [],
              blockers: ['CODEOWNER @maintainer does not have write permission at the protected base.'],
            }],
            state: 'blocked',
            reason: 'A reviewed change disappeared, but its required owner identity cannot be authorized.',
          },
        ],
        unresolved_residue: [{
          checkpoint_commit: '4'.repeat(40),
          path: 'git-bytes:%FF.ts',
          path_encoding: 'git_bytes_percent_encoded',
          reason: 'non_utf8_git_path',
        }],
        summary: {
          current_files: 2,
          retired_residue_files: 1,
          unresolved_residue: 1,
          total_requirements: 4,
          covered_files: 1,
          needs_review_files: 1,
          blocked_files: 2,
          active_review_receipts: 2,
          unique_checkpoint_proofs: 1,
          gate_passed: false,
        },
        non_claims: [
          'This passport does not create or restore a GitHub approval.',
          'Coverage is not a claim of semantic safety or absence of bugs.',
        ],
      },
      attestation: {
        algorithm: 'ed25519',
        key_id: 'receiver-1',
        body_sha256: '5'.repeat(64),
        signature: '6'.repeat(128),
      },
    },
  }
}
