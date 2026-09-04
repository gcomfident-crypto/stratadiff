import type { FileSessionPayload, NodeRef, RepositorySessionPayload, ReviewFile } from '../types'

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
      comparison: 'snapshot_to_snapshot',
      from_commit: checkpoint,
      source_base_commit: checkpoint,
      to_commit: head,
      summary: {
        changed_files: 2,
        first_pass_files: 2,
        review_first_files: 2,
        syntax_preserved_files: 0,
        content_preserved_files: 0,
        unverified_files: 0,
        replay_check_passed_files: 1,
        replay_check_not_run_files: 1,
        line_envelope_complete: true,
        changed_line_envelope: 4,
        first_pass_line_envelope: 4,
      },
      files: resumeFiles,
    },
    assessment: {
      status: 'producer_attested',
      basis: 'exact_git_change_identity',
      message: 'Checkpoint carry-forward is exact but does not establish semantic safety.',
    },
  }
}
