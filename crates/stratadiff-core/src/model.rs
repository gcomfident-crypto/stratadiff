use serde::{Deserialize, Serialize};

use crate::language::Language;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NodeRef {
    pub id: usize,
    pub kind: String,
    pub named: bool,
    pub extra: bool,
    pub missing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub span: Span,
    pub subtree_size: usize,
    pub syntax_hash: String,
    pub shape_hash: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    InputPair,
    ByteEqual,
    SyntaxEqual,
    ShapeEqual,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Correspondence {
    InputPair,
    ModelForced,
    Suggested,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub before: NodeRef,
    pub after: NodeRef,
    pub predicate: Predicate,
    pub correspondence: Correspondence,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// One explicitly supported edge in an exact ambiguity constraint.
pub struct AmbiguityPair {
    pub before_id: usize,
    pub after_id: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityAbstentionCause {
    DuplicateSymmetry,
    ComponentLimit,
    CandidateScanLimit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairClaims {
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AmbiguityConstraint {
    /// A resolution selects exactly `required_matches` listed pairs, uses each endpoint at most
    /// once, and preserves the order of the enclosing `before` and `after` endpoint arrays.
    ExactOrderedAlignment {
        predicate: Predicate,
        required_matches: usize,
        possible_pairs: Vec<AmbiguityPair>,
    },
    /// The endpoint arrays define only the abstention scope and make no pairwise claims.
    SymbolicAbstention {
        cause: AmbiguityAbstentionCause,
        pair_claims: PairClaims,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AmbiguityGroup {
    pub parent_before: usize,
    pub parent_after: usize,
    pub before: Vec<NodeRef>,
    pub after: Vec<NodeRef>,
    pub constraint: AmbiguityConstraint,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Insert,
    Delete,
    EquivalentRelocation,
    ChildOrderChanged,
    ModelForcedUpdate,
    SuggestedUpdate,
    FormattingOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StructuralChange {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<NodeRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<NodeRef>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: String,
    pub byte_len: usize,
    pub blake3: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParserManifest {
    pub engine: String,
    pub runtime_version: String,
    pub language: Language,
    pub grammar_name: String,
    pub grammar_version: String,
    pub grammar_abi: usize,
    pub node_types_blake3: String,
    pub coordinate_unit: String,
    pub root_kind: String,
    pub before_nodes: usize,
    pub after_nodes: usize,
    pub error_free: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ByteEdit {
    pub old_start: usize,
    pub old_end: usize,
    pub replacement_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LosslessPatch {
    pub algorithm: String,
    pub edits: Vec<ByteEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplayCertificate {
    pub before_blake3: String,
    pub after_blake3: String,
    pub reconstructed_blake3: String,
    pub before_len: usize,
    pub after_len: usize,
    pub patch_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub model_forced_relations: usize,
    pub suggested_relations: usize,
    pub ambiguity_groups: usize,
    pub structural_changes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiffReport {
    pub schema: String,
    pub engine_version: String,
    pub before: Artifact,
    pub after: Artifact,
    pub parser: ParserManifest,
    pub relations: Vec<Relation>,
    pub ambiguities: Vec<AmbiguityGroup>,
    pub changes: Vec<StructuralChange>,
    pub patch: LosslessPatch,
    pub certificate: ReplayCertificate,
    pub summary: Summary,
}
