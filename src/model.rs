use serde::{Deserialize, Serialize};

use crate::language::Language;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
pub struct Relation {
    pub before: NodeRef,
    pub after: NodeRef,
    pub predicate: Predicate,
    pub correspondence: Correspondence,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AmbiguityGroup {
    pub parent_before: usize,
    pub parent_after: usize,
    pub predicate: Predicate,
    pub before: Vec<NodeRef>,
    pub after: Vec<NodeRef>,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Insert,
    Delete,
    EquivalentRelocation,
    ChildOrderChanged,
    SuggestedUpdate,
    FormattingOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralChange {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<NodeRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<NodeRef>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub path: String,
    pub byte_len: usize,
    pub blake3: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
pub struct ByteEdit {
    pub old_start: usize,
    pub old_end: usize,
    pub replacement_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LosslessPatch {
    pub algorithm: String,
    pub edits: Vec<ByteEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayCertificate {
    pub before_blake3: String,
    pub after_blake3: String,
    pub reconstructed_blake3: String,
    pub before_len: usize,
    pub after_len: usize,
    pub patch_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    pub model_forced_relations: usize,
    pub suggested_relations: usize,
    pub ambiguity_groups: usize,
    pub structural_changes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
