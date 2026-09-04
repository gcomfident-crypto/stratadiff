import type { NodeRef, SessionPayload } from '../types'

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

export function sessionFixture(): SessionPayload {
  const beforeText = 'const before = 1\n'
  const afterText = 'const after = 2\n'
  return {
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
    verification: { verified: true, message: 'Report verified and replay matched the target.' },
  }
}
