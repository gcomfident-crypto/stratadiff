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

export interface SessionPayload {
  report: DiffReport
  verification: VerificationResult
}

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

export interface LoadedSession extends SessionPayload {
  decodedBefore: DecodedArtifact
  decodedAfter: DecodedArtifact
}
