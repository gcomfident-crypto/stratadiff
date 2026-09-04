export type ViewMode = 'code' | 'structure' | 'bytes'
export type DiffStyle = 'split' | 'unified'

export interface Position {
  row: number
  column: number
}

export interface Span {
  start_byte: number
  end_byte: number
  start: Position
  end: Position
}

export interface NodeRef {
  id: number
  kind: string
  named: boolean
  extra: boolean
  missing: boolean
  field?: string
  span: Span
  subtree_size: number
  syntax_hash: string
  shape_hash: string
}

export type Predicate = 'input_pair' | 'byte_equal' | 'syntax_equal' | 'shape_equal'
export type Correspondence = 'input_pair' | 'model_forced' | 'suggested'

export interface Relation {
  before: NodeRef
  after: NodeRef
  predicate: Predicate
  correspondence: Correspondence
  evidence: string[]
}

export interface AmbiguityPair {
  before_id: number
  after_id: number
}

export interface ExactOrderedAlignment {
  kind: 'exact_ordered_alignment'
  predicate: Predicate
  required_matches: number
  possible_pairs: AmbiguityPair[]
}

export interface SymbolicAbstention {
  kind: 'symbolic_abstention'
  cause: 'duplicate_symmetry' | 'component_limit' | 'candidate_scan_limit'
  pair_claims: 'none'
}

export interface AmbiguityGroup {
  parent_before: number
  parent_after: number
  before: NodeRef[]
  after: NodeRef[]
  constraint: ExactOrderedAlignment | SymbolicAbstention
  reason: string
}

export type ChangeKind =
  | 'insert'
  | 'delete'
  | 'equivalent_relocation'
  | 'child_order_changed'
  | 'model_forced_update'
  | 'suggested_update'
  | 'formatting_only'

export interface StructuralChange {
  kind: ChangeKind
  before?: NodeRef
  after?: NodeRef
  detail: string
}

export interface Artifact {
  path: string
  byte_len: number
  blake3: string
}

export interface ParserManifest {
  engine: 'tree-sitter' | 'stratadiff-universal'
  runtime_version: string
  language: string
  grammar_name: string
  grammar_version: string
  grammar_abi: number
  node_types_blake3: string
  coordinate_unit: 'zero_based_row_utf8_byte_column' | 'zero_based_row_byte_column'
  root_kind: string
  before_nodes: number
  after_nodes: number
  error_free: true
}

export interface ByteEdit {
  old_start: number
  old_end: number
  replacement_base64: string
}

export interface ReplayCertificate {
  before_blake3: string
  after_blake3: string
  reconstructed_blake3: string
  before_len: number
  after_len: number
  patch_verified: boolean
}

export interface DiffReport {
  schema: string
  engine_version: string
  before: Artifact
  after: Artifact
  parser: ParserManifest
  relations: Relation[]
  ambiguities: AmbiguityGroup[]
  changes: StructuralChange[]
  patch: {
    algorithm: string
    edits: ByteEdit[]
  }
  certificate: ReplayCertificate
  summary: {
    model_forced_relations: number
    suggested_relations: number
    ambiguity_groups: number
    structural_changes: number
  }
}

export interface VerificationResult {
  verified: true
  message: string
}

export interface FileSessionPayload {
  kind: 'file_diff'
  report: DiffReport
  verification: VerificationResult
  repository_context?: {
    file_index: number
    scope: 'resume' | 'full'
    checkpoint_state?: CheckpointState
    checkpoint_match_basis?: 'exact_git_change_identity' | 'exact_noninteracting_four_way_byte_replay'
  }
}

export type FileStatus = 'added' | 'copied' | 'deleted' | 'modified' | 'renamed' | 'type_changed'
export type ReviewLane = 'review_first' | 'syntax_preserved' | 'content_preserved' | 'unverified'
export type ReviewPriority = 'review_first' | 'evidence_follow_up'
export type CheckpointState = 'needs_review_now' | 'unchanged_since_checkpoint'

export interface ReviewFile {
  status: FileStatus
  similarity_percent?: number
  before_path?: string
  before_path_encoding?: 'utf8' | 'git_bytes_percent_encoded'
  after_path?: string
  after_path_encoding?: 'utf8' | 'git_bytes_percent_encoded'
  before_mode?: string
  after_mode?: string
  before_blob?: string
  after_blob?: string
  before_bytes?: number
  after_bytes?: number
  line_change_envelope?: {
    additions: number
    deletions: number
  }
  language?: string
  priority: ReviewPriority
  lane: ReviewLane
  checkpoint_state?: CheckpointState
  checkpoint_match_basis?: 'exact_git_change_identity' | 'exact_noninteracting_four_way_byte_replay'
  reason: string
  evidence?: {
    report_blake3: string
    replay_check_passed_during_analysis: boolean
    model_forced_relations: number
    suggested_relations: number
    ambiguity_groups: number
    byte_edits: number
    changes: {
      insertions: number
      deletions: number
      equivalent_relocations: number
      child_order_changes: number
      model_forced_updates: number
      suggested_updates: number
      formatting_only: number
    }
  }
}

export interface RepositoryReview {
  schema: string
  engine_version: string
  requested_base: string
  requested_head: string
  base_commit: string
  head_commit: string
  comparison: string
  checkpoint?: {
    requested_revision: string
    commit: string
    base_commit: string
    match_basis: 'exact_git_change_identity' | 'exact_git_change_identity_or_noninteracting_four_way_byte_replay'
  }
  summary: {
    changed_files: number
    first_pass_files: number
    review_first_files: number
    syntax_preserved_files: number
    content_preserved_files: number
    unverified_files: number
    replay_check_passed_files: number
    replay_check_not_run_files: number
    line_envelope_complete: boolean
    changed_line_envelope?: number
    first_pass_line_envelope?: number
    checkpoint?: {
      needs_review_now_files: number
      unchanged_since_checkpoint_files: number
      retired_change_count: number
    }
  }
  files: ReviewFile[]
}

export interface ReviewDelta {
  comparison: 'snapshot_to_snapshot' | 'current_pr_unmatched_identities'
  from_commit: string
  source_base_commit: string
  to_commit: string
  summary: RepositoryReview['summary']
  files: ReviewFile[]
}

export interface RepositorySessionPayload {
  kind: 'repository_review'
  review: RepositoryReview
  resume_delta: ReviewDelta
  assessment: {
    status: 'producer_attested'
    basis: 'exact_git_change_identity' | 'exact_git_change_identity_or_noninteracting_four_way_byte_replay'
    message: string
  }
}

export type SessionPayload = FileSessionPayload | RepositorySessionPayload

export type EvidenceSelection =
  | { type: 'change'; index: number }
  | { type: 'ambiguity'; index: number }
  | { type: 'edit'; index: number }
  | { type: 'relation'; index: number }

export interface DecodedArtifact {
  path: string
  bytes: Uint8Array
  text: string | null
}

export interface LoadedFileSession extends FileSessionPayload {
  decodedBefore: DecodedArtifact
  decodedAfter: DecodedArtifact
}

export type LoadedSession = LoadedFileSession | RepositorySessionPayload
