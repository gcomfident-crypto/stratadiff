use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::Language;
use crate::syntax::parse;

/// The two mapping categories published in each DiffBenchmark `GOD.json` group.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GodMappingGroup {
    pub matched_elements: Vec<GodMappingRecord>,
    pub mappings: Vec<GodMappingRecord>,
}

/// One human-readable mapping entry from DiffBenchmark.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GodMappingRecord {
    pub left: String,
    pub right: String,
    pub info: String,
}

/// The complete supported shape of one DiffBenchmark `GOD.json` oracle file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GodReport {
    pub intra_file_mappings: GodMappingGroup,
    pub inter_file_mappings: BTreeMap<String, GodMappingGroup>,
}

/// Parse one oracle file strictly. Malformed JSON and unknown fields are errors rather than
/// silently excluded benchmark cases.
pub fn parse_god_report(bytes: &[u8]) -> Result<GodReport> {
    serde_json::from_slice(bytes).context("invalid DiffBenchmark GOD.json")
}

/// Conservative semantic roles shared by Eclipse JDT and tree-sitter-java node types.
///
/// A role is emitted only for parser node types listed explicitly by the two classifier
/// functions. The taxonomy deliberately has no spelling-based fallback.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SharedNodeRole {
    MethodDeclaration,
    FieldDeclaration,
    TypeDeclaration,
    /// The declaration keyword represented by JDT's synthetic `TYPE_DECLARATION_KIND` node.
    TypeDeclarationKind,
    EnumDeclaration,
    EnumConstantDeclaration,
    AnnotationTypeDeclaration,
    RecordDeclaration,
    Block,
    AssertStatement,
    BreakStatement,
    ContinueStatement,
    DoStatement,
    EnhancedForStatement,
    ExplicitConstructorInvocation,
    ExpressionStatement,
    ForStatement,
    IfStatement,
    LabeledStatement,
    LocalVariableDeclaration,
    ReturnStatement,
    SwitchConstruct,
    SynchronizedStatement,
    ThrowStatement,
    TryStatement,
    WhileStatement,
    YieldStatement,
    SimpleName,
    PrimitiveType,
    SimpleType,
    ArrayType,
    ParameterizedType,
    /// One modifier keyword, never tree-sitter's aggregate `modifiers` node.
    Modifier,
    BooleanLiteral,
    CharacterLiteral,
    NumberLiteral,
    StringLiteral,
    NullLiteral,
    TypeLiteral,
}

impl SharedNodeRole {
    fn is_declaration(self) -> bool {
        matches!(
            self,
            Self::MethodDeclaration
                | Self::FieldDeclaration
                | Self::TypeDeclaration
                | Self::EnumDeclaration
                | Self::EnumConstantDeclaration
                | Self::AnnotationTypeDeclaration
                | Self::RecordDeclaration
        )
    }
}

/// A parser-neutral endpoint used for exact benchmark comparison.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct ComparableNode {
    pub role: SharedNodeRole,
    pub utf8_bytes: OffsetRange,
}

/// Return the explicitly supported nodes in one error-free tree-sitter-java parse.
pub fn comparable_tree_sitter_java_nodes(source: &[u8]) -> Result<Vec<ComparableNode>> {
    let parsed = parse(source.to_vec(), Language::Java)?;
    Ok(parsed
        .nodes
        .iter()
        .filter_map(|node| {
            tree_sitter_java_node_role(&node.kind).map(|role| ComparableNode {
                role,
                utf8_bytes: OffsetRange {
                    start: node.span.start_byte,
                    end: node.span.end_byte,
                },
            })
        })
        .collect())
}

/// Resolve a JDT oracle endpoint to exactly one comparable tree-sitter node.
///
/// JDT declaration ranges may include a leading Javadoc while tree-sitter keeps that comment as an
/// extra sibling. For declaration roles only, leading whitespace and Java comments are removed
/// before the exact role-and-range lookup. No fuzzy range or label matching is performed.
pub fn resolve_jdt_node(
    node: &JdtOracleNode,
    source: &str,
    candidates: &[ComparableNode],
) -> Result<Option<ComparableNode>> {
    let normalized = normalize_node(node, source, "oracle")?;
    let direct = ComparableNode {
        role: normalized.role,
        utf8_bytes: normalized.utf8_bytes,
    };
    if count_node(candidates, direct) == 1 {
        return Ok(Some(direct));
    }

    if !direct.role.is_declaration() {
        return Ok(None);
    }
    let adjusted = ComparableNode {
        role: direct.role,
        utf8_bytes: OffsetRange {
            start: skip_leading_java_trivia(
                source,
                direct.utf8_bytes.start,
                direct.utf8_bytes.end,
            )?,
            end: direct.utf8_bytes.end,
        },
    };
    Ok((count_node(candidates, adjusted) == 1).then_some(adjusted))
}

fn count_node(candidates: &[ComparableNode], expected: ComparableNode) -> usize {
    candidates
        .iter()
        .filter(|candidate| **candidate == expected)
        .count()
}

fn skip_leading_java_trivia(source: &str, start: usize, end: usize) -> Result<usize> {
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        bail!("oracle byte range {start}-{end} is not a valid UTF-8 source range");
    }

    let mut cursor = start;
    loop {
        while cursor < end {
            let character = source[cursor..end]
                .chars()
                .next()
                .context("expected a character before the range end")?;
            if !character.is_whitespace() {
                break;
            }
            cursor += character.len_utf8();
        }
        let remaining = &source[cursor..end];
        if remaining.starts_with("//") {
            cursor += remaining
                .find('\n')
                .map_or(remaining.len(), |index| index + 1);
        } else if remaining.starts_with("/*") {
            let close = remaining
                .find("*/")
                .context("unterminated Java comment in oracle declaration range")?;
            cursor += close + 2;
        } else {
            return Ok(cursor);
        }
    }
}

/// Classify an Eclipse JDT AST node type into the shared taxonomy.
pub fn jdt_node_role(node_type: &str) -> Option<SharedNodeRole> {
    Some(match node_type {
        "MethodDeclaration" => SharedNodeRole::MethodDeclaration,
        "FieldDeclaration" => SharedNodeRole::FieldDeclaration,
        "TypeDeclaration" => SharedNodeRole::TypeDeclaration,
        "TYPE_DECLARATION_KIND" => SharedNodeRole::TypeDeclarationKind,
        "EnumDeclaration" => SharedNodeRole::EnumDeclaration,
        "EnumConstantDeclaration" => SharedNodeRole::EnumConstantDeclaration,
        "AnnotationTypeDeclaration" => SharedNodeRole::AnnotationTypeDeclaration,
        "RecordDeclaration" => SharedNodeRole::RecordDeclaration,
        "Block" => SharedNodeRole::Block,
        "AssertStatement" => SharedNodeRole::AssertStatement,
        "BreakStatement" => SharedNodeRole::BreakStatement,
        "ContinueStatement" => SharedNodeRole::ContinueStatement,
        "DoStatement" => SharedNodeRole::DoStatement,
        "EnhancedForStatement" => SharedNodeRole::EnhancedForStatement,
        "ConstructorInvocation" | "SuperConstructorInvocation" => {
            SharedNodeRole::ExplicitConstructorInvocation
        }
        "ExpressionStatement" => SharedNodeRole::ExpressionStatement,
        "ForStatement" => SharedNodeRole::ForStatement,
        "IfStatement" => SharedNodeRole::IfStatement,
        "LabeledStatement" => SharedNodeRole::LabeledStatement,
        "VariableDeclarationStatement" => SharedNodeRole::LocalVariableDeclaration,
        "ReturnStatement" => SharedNodeRole::ReturnStatement,
        "SwitchStatement" | "SwitchExpression" => SharedNodeRole::SwitchConstruct,
        "SynchronizedStatement" => SharedNodeRole::SynchronizedStatement,
        "ThrowStatement" => SharedNodeRole::ThrowStatement,
        "TryStatement" => SharedNodeRole::TryStatement,
        "WhileStatement" => SharedNodeRole::WhileStatement,
        "YieldStatement" => SharedNodeRole::YieldStatement,
        "SimpleName" => SharedNodeRole::SimpleName,
        "PrimitiveType" => SharedNodeRole::PrimitiveType,
        "SimpleType" => SharedNodeRole::SimpleType,
        "ArrayType" => SharedNodeRole::ArrayType,
        "ParameterizedType" => SharedNodeRole::ParameterizedType,
        "Modifier" => SharedNodeRole::Modifier,
        "BooleanLiteral" => SharedNodeRole::BooleanLiteral,
        "CharacterLiteral" => SharedNodeRole::CharacterLiteral,
        "NumberLiteral" => SharedNodeRole::NumberLiteral,
        "StringLiteral" | "TextBlock" => SharedNodeRole::StringLiteral,
        "NullLiteral" => SharedNodeRole::NullLiteral,
        "TypeLiteral" => SharedNodeRole::TypeLiteral,
        _ => return None,
    })
}

/// Classify a tree-sitter-java node type into the shared taxonomy.
pub fn tree_sitter_java_node_role(node_type: &str) -> Option<SharedNodeRole> {
    Some(match node_type {
        "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" => {
            SharedNodeRole::MethodDeclaration
        }
        "field_declaration" | "constant_declaration" => SharedNodeRole::FieldDeclaration,
        "class_declaration" | "interface_declaration" => SharedNodeRole::TypeDeclaration,
        "class" | "interface" => SharedNodeRole::TypeDeclarationKind,
        "enum_declaration" => SharedNodeRole::EnumDeclaration,
        "enum_constant" => SharedNodeRole::EnumConstantDeclaration,
        "annotation_type_declaration" => SharedNodeRole::AnnotationTypeDeclaration,
        "record_declaration" => SharedNodeRole::RecordDeclaration,
        "block" | "constructor_body" => SharedNodeRole::Block,
        "assert_statement" => SharedNodeRole::AssertStatement,
        "break_statement" => SharedNodeRole::BreakStatement,
        "continue_statement" => SharedNodeRole::ContinueStatement,
        "do_statement" => SharedNodeRole::DoStatement,
        "enhanced_for_statement" => SharedNodeRole::EnhancedForStatement,
        "explicit_constructor_invocation" => SharedNodeRole::ExplicitConstructorInvocation,
        "expression_statement" => SharedNodeRole::ExpressionStatement,
        "for_statement" => SharedNodeRole::ForStatement,
        "if_statement" => SharedNodeRole::IfStatement,
        "labeled_statement" => SharedNodeRole::LabeledStatement,
        "local_variable_declaration" => SharedNodeRole::LocalVariableDeclaration,
        "return_statement" => SharedNodeRole::ReturnStatement,
        "switch_expression" => SharedNodeRole::SwitchConstruct,
        "synchronized_statement" => SharedNodeRole::SynchronizedStatement,
        "throw_statement" => SharedNodeRole::ThrowStatement,
        "try_statement" | "try_with_resources_statement" => SharedNodeRole::TryStatement,
        "while_statement" => SharedNodeRole::WhileStatement,
        "yield_statement" => SharedNodeRole::YieldStatement,
        "identifier" => SharedNodeRole::SimpleName,
        "integral_type" | "floating_point_type" | "boolean_type" | "void_type" => {
            SharedNodeRole::PrimitiveType
        }
        "type_identifier" => SharedNodeRole::SimpleType,
        "array_type" => SharedNodeRole::ArrayType,
        "generic_type" => SharedNodeRole::ParameterizedType,
        "public" | "private" | "protected" | "abstract" | "static" | "final" | "strictfp"
        | "default" | "synchronized" | "native" | "transient" | "volatile" | "sealed"
        | "non-sealed" => SharedNodeRole::Modifier,
        "true" | "false" => SharedNodeRole::BooleanLiteral,
        "character_literal" => SharedNodeRole::CharacterLiteral,
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "decimal_floating_point_literal"
        | "hex_floating_point_literal" => SharedNodeRole::NumberLiteral,
        "string_literal" => SharedNodeRole::StringLiteral,
        "null_literal" => SharedNodeRole::NullLiteral,
        "class_literal" => SharedNodeRole::TypeLiteral,
        _ => return None,
    })
}

/// A half-open offset range. JDT ranges count UTF-16 code units; normalized ranges count UTF-8
/// bytes in the original source.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct OffsetRange {
    pub start: usize,
    pub end: usize,
}

/// One endpoint parsed from DiffBenchmark's `Type[start-end]` notation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JdtOracleNode {
    pub node_type: String,
    pub utf16_code_units: OffsetRange,
}

/// A raw mapping from a `GOD.json` `info` field.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JdtOracleMapping {
    pub before: JdtOracleNode,
    pub after: JdtOracleNode,
}

/// A DiffBenchmark endpoint normalized to Tree-sitter's UTF-8 byte coordinate system.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OracleNode {
    pub node_type: String,
    pub role: SharedNodeRole,
    pub jdt_utf16_code_units: OffsetRange,
    pub utf8_bytes: OffsetRange,
}

/// A serializable mapping suitable for comparison with Tree-sitter node spans.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OracleMapping {
    pub before: OracleNode,
    pub after: OracleNode,
}

/// Parse one DiffBenchmark `GOD.json` `info` value.
///
/// Both JDT ranges are interpreted as half-open UTF-16 code-unit ranges.
pub fn parse_god_info(info: &str) -> Result<JdtOracleMapping> {
    let mut endpoints = info.split(':');
    let before = endpoints
        .next()
        .context("DiffBenchmark info is missing the before endpoint")?;
    let after = endpoints
        .next()
        .context("DiffBenchmark info is missing the after endpoint")?;
    if endpoints.next().is_some() {
        bail!("DiffBenchmark info must contain exactly one ':' separator");
    }

    Ok(JdtOracleMapping {
        before: parse_endpoint(before, "before")?,
        after: parse_endpoint(after, "after")?,
    })
}

/// Parse and normalize one DiffBenchmark mapping without reading files or accessing the dataset.
pub fn parse_oracle_mapping(
    info: &str,
    before_source: &str,
    after_source: &str,
) -> Result<OracleMapping> {
    normalize_oracle_mapping(&parse_god_info(info)?, before_source, after_source)
}

/// Normalize an already parsed JDT mapping to UTF-8 byte offsets.
pub fn normalize_oracle_mapping(
    mapping: &JdtOracleMapping,
    before_source: &str,
    after_source: &str,
) -> Result<OracleMapping> {
    Ok(OracleMapping {
        before: normalize_node(&mapping.before, before_source, "before")?,
        after: normalize_node(&mapping.after, after_source, "after")?,
    })
}

/// Convert a UTF-16 code-unit offset into a UTF-8 byte offset in the same source.
///
/// Offsets at the start or end of a Unicode scalar value are accepted. An offset between the two
/// UTF-16 code units of a non-BMP scalar value, or beyond the source, is rejected.
pub fn utf16_offset_to_byte_offset(source: &str, offset: usize) -> Result<usize> {
    let mut utf16_offset = 0;
    for (byte_offset, character) in source.char_indices() {
        if utf16_offset == offset {
            return Ok(byte_offset);
        }
        let next_utf16_offset = utf16_offset + character.len_utf16();
        if offset < next_utf16_offset {
            bail!("UTF-16 offset {offset} splits a surrogate pair");
        }
        utf16_offset = next_utf16_offset;
    }

    if utf16_offset == offset {
        Ok(source.len())
    } else {
        bail!("UTF-16 offset {offset} exceeds source length of {utf16_offset} code units")
    }
}

fn parse_endpoint(endpoint: &str, side: &str) -> Result<JdtOracleNode> {
    if endpoint.trim() != endpoint {
        bail!("DiffBenchmark {side} endpoint contains surrounding whitespace");
    }
    let (node_type, range) = endpoint
        .split_once('[')
        .with_context(|| format!("DiffBenchmark {side} endpoint is missing '['"))?;
    if node_type.is_empty()
        || node_type.chars().any(char::is_whitespace)
        || node_type.contains(['[', ']', ':'])
    {
        bail!("DiffBenchmark {side} endpoint has an invalid node type");
    }
    let range = range
        .strip_suffix(']')
        .with_context(|| format!("DiffBenchmark {side} endpoint is missing closing ']'"))?;
    if range.contains(['[', ']']) {
        bail!("DiffBenchmark {side} endpoint has malformed brackets");
    }
    let (start, end) = range
        .split_once('-')
        .with_context(|| format!("DiffBenchmark {side} endpoint is missing '-'"))?;
    let start = parse_offset(start, side, "start")?;
    let end = parse_offset(end, side, "end")?;
    if start > end {
        bail!("DiffBenchmark {side} range starts after it ends: {start}-{end}");
    }

    Ok(JdtOracleNode {
        node_type: node_type.to_owned(),
        utf16_code_units: OffsetRange { start, end },
    })
}

fn parse_offset(value: &str, side: &str, boundary: &str) -> Result<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("DiffBenchmark {side} {boundary} offset is not an unsigned decimal integer");
    }
    value
        .parse()
        .with_context(|| format!("DiffBenchmark {side} {boundary} offset is too large"))
}

fn normalize_node(node: &JdtOracleNode, source: &str, side: &str) -> Result<OracleNode> {
    let range = &node.utf16_code_units;
    if range.start > range.end {
        bail!(
            "DiffBenchmark {side} range starts after it ends: {}-{}",
            range.start,
            range.end
        );
    }
    let start = utf16_offset_to_byte_offset(source, range.start)
        .with_context(|| format!("invalid DiffBenchmark {side} start offset"))?;
    let end = utf16_offset_to_byte_offset(source, range.end)
        .with_context(|| format!("invalid DiffBenchmark {side} end offset"))?;
    let role = jdt_node_role(&node.node_type).with_context(|| {
        format!(
            "unsupported DiffBenchmark {side} JDT node type {}",
            node.node_type
        )
    })?;

    Ok(OracleNode {
        node_type: node.node_type.clone(),
        role,
        jdt_utf16_code_units: *range,
        utf8_bytes: OffsetRange { start, end },
    })
}
