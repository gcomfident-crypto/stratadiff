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
    SingleVariableDeclaration,
    MarkerAnnotation,
    NormalAnnotation,
    SingleMemberAnnotation,
    SwitchCase,
    TypeParameter,
    VariableDeclarationFragment,
    MethodInvocation,
    ArrayAccess,
    ArrayCreation,
    CastExpression,
    ClassInstanceCreation,
    ConditionalExpression,
    EmptyStatement,
    InfixExpression,
    InstanceofExpression,
    ParenthesizedExpression,
    PrefixExpression,
    ThisExpression,
    LineComment,
    BlockComment,
    /// JDT distinguishes `QualifiedName` from `FieldAccess`, while tree-sitter may represent the
    /// same exact source range as either `scoped_identifier` or `field_access` by context.
    QualifiedAccess,
    InfixExpressionOperator,
    MethodInvocationArguments,
    MethodInvocationReceiver,
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
    let mut comparable = Vec::new();
    for node in &parsed.nodes {
        let range = OffsetRange {
            start: node.span.start_byte,
            end: node.span.end_byte,
        };
        let roles =
            if node.kind == "method_invocation" && !is_jdt_method_invocation(&parsed, node.id) {
                &[]
            } else {
                tree_sitter_java_node_roles(&node.kind)
            };
        if node.kind == "annotation" {
            comparable.push(ComparableNode {
                role: annotation_role(&parsed, node.id),
                utf8_bytes: range,
            });
        } else if node.kind == "switch_label" {
            comparable.push(ComparableNode {
                role: SharedNodeRole::SwitchCase,
                utf8_bytes: switch_case_range(&parsed, node.id)?,
            });
        } else {
            comparable.extend(roles.iter().map(|role| ComparableNode {
                role: *role,
                utf8_bytes: range,
            }));
        }

        let Some(parent_id) = node.parent else {
            continue;
        };
        let parent = &parsed.nodes[parent_id];
        if parent.kind == "binary_expression"
            && node.field.as_deref() == Some("operator")
            && is_binary_operator(&node.kind)
        {
            comparable.push(ComparableNode {
                role: SharedNodeRole::InfixExpressionOperator,
                utf8_bytes: range,
            });
        }
        if node.kind == ";" && is_empty_statement_context(parent, node.field.as_deref()) {
            comparable.push(ComparableNode {
                role: SharedNodeRole::EmptyStatement,
                utf8_bytes: range,
            });
        }
        if node.kind == "update_expression" && is_prefix_update(&parsed, node.id) {
            comparable.push(ComparableNode {
                role: SharedNodeRole::PrefixExpression,
                utf8_bytes: range,
            });
        }
        if parent.kind == "method_invocation"
            && is_jdt_method_invocation(&parsed, parent_id)
            && node.field.as_deref() == Some("object")
        {
            comparable.push(ComparableNode {
                role: SharedNodeRole::MethodInvocationReceiver,
                utf8_bytes: range,
            });
        }
        if parent.kind == "method_invocation"
            && is_jdt_method_invocation(&parsed, parent_id)
            && node.field.as_deref() == Some("arguments")
            && node.kind == "argument_list"
            && let Some(argument_range) = method_invocation_argument_range(&parsed, node.id)?
        {
            comparable.push(ComparableNode {
                role: SharedNodeRole::MethodInvocationArguments,
                utf8_bytes: argument_range,
            });
        }
    }
    Ok(comparable)
}

fn is_jdt_method_invocation(parsed: &crate::syntax::ParsedSyntax, invocation_id: usize) -> bool {
    !parsed.nodes[invocation_id].children.iter().any(|id| {
        let child = &parsed.nodes[*id];
        child.field.as_deref() == Some("object") && child.kind == "super"
    })
}

fn is_prefix_update(parsed: &crate::syntax::ParsedSyntax, update_id: usize) -> bool {
    let update = &parsed.nodes[update_id];
    update.children.first().is_some_and(|id| {
        let first = &parsed.nodes[*id];
        matches!(first.kind.as_str(), "++" | "--")
            && first.span.start_byte == update.span.start_byte
    })
}

fn is_empty_statement_context(parent: &crate::syntax::SyntaxNode, field: Option<&str>) -> bool {
    match parent.kind.as_str() {
        "block" | "switch_block_statement_group" | "labeled_statement" => true,
        "if_statement" => matches!(field, Some("consequence" | "alternative")),
        "while_statement" | "for_statement" | "enhanced_for_statement" | "do_statement" => {
            field == Some("body")
        }
        _ => false,
    }
}

fn switch_case_range(
    parsed: &crate::syntax::ParsedSyntax,
    switch_label_id: usize,
) -> Result<OffsetRange> {
    let label = &parsed.nodes[switch_label_id];
    let parent_id = label
        .parent
        .context("tree-sitter switch_label has no parent")?;
    let siblings = &parsed.nodes[parent_id].children;
    let position = siblings
        .iter()
        .position(|id| *id == switch_label_id)
        .context("tree-sitter switch_label is absent from its parent's children")?;
    let delimiter = siblings
        .get(position + 1)
        .map(|id| &parsed.nodes[*id])
        .context("tree-sitter switch_label has no following delimiter")?;
    if !matches!(delimiter.kind.as_str(), ":" | "->") {
        bail!(
            "tree-sitter switch_label is followed by unexpected node kind {}",
            delimiter.kind
        );
    }
    Ok(OffsetRange {
        start: label.span.start_byte,
        end: delimiter.span.end_byte,
    })
}

fn annotation_role(parsed: &crate::syntax::ParsedSyntax, annotation_id: usize) -> SharedNodeRole {
    let annotation = &parsed.nodes[annotation_id];
    let arguments = annotation
        .children
        .iter()
        .map(|id| &parsed.nodes[*id])
        .find(|child| {
            child.field.as_deref() == Some("arguments") && child.kind == "annotation_argument_list"
        })
        .expect("tree-sitter annotation has a required annotation_argument_list");
    if arguments
        .children
        .iter()
        .any(|id| parsed.nodes[*id].kind == "element_value_pair")
        || !arguments.children.iter().any(|id| parsed.nodes[*id].named)
    {
        SharedNodeRole::NormalAnnotation
    } else {
        SharedNodeRole::SingleMemberAnnotation
    }
}

fn method_invocation_argument_range(
    parsed: &crate::syntax::ParsedSyntax,
    argument_list_id: usize,
) -> Result<Option<OffsetRange>> {
    let argument_list = &parsed.nodes[argument_list_id];
    let bytes = &parsed.source[argument_list.span.start_byte..argument_list.span.end_byte];
    if bytes.len() < 2 || bytes.first() != Some(&b'(') || bytes.last() != Some(&b')') {
        bail!("tree-sitter method invocation argument_list is not parenthesized");
    }

    let named_arguments: Vec<_> = argument_list
        .children
        .iter()
        .map(|id| &parsed.nodes[*id])
        .filter(|child| child.named && !child.extra)
        .collect();
    if let (Some(first), Some(last)) = (named_arguments.first(), named_arguments.last()) {
        Ok(Some(OffsetRange {
            start: first.span.start_byte,
            end: last.span.end_byte,
        }))
    } else {
        Ok(None)
    }
}

fn is_binary_operator(kind: &str) -> bool {
    matches!(
        kind,
        "!=" | "%"
            | "&"
            | "&&"
            | "*"
            | "+"
            | "-"
            | "/"
            | "<"
            | "<<"
            | "<="
            | "=="
            | ">"
            | ">="
            | ">>"
            | ">>>"
            | "^"
            | "|"
            | "||"
    )
}

/// Resolve a JDT oracle endpoint to exactly one comparable tree-sitter node.
///
/// JDT declaration ranges may include a leading Javadoc while tree-sitter keeps that comment as an
/// extra sibling. For declaration roles only, one exact `/** ... */` prefix and the whitespace
/// immediately following it are removed before the exact role-and-range lookup. Ordinary comments,
/// arbitrary whitespace, fuzzy ranges, and labels are never used to make a match.
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
            start: skip_leading_javadoc(source, direct.utf8_bytes.start, direct.utf8_bytes.end)?,
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

fn skip_leading_javadoc(source: &str, start: usize, end: usize) -> Result<usize> {
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        bail!("oracle byte range {start}-{end} is not a valid UTF-8 source range");
    }

    let remaining = &source[start..end];
    if !remaining.starts_with("/**") {
        return Ok(start);
    }
    let close = remaining
        .find("*/")
        .context("unterminated Javadoc in oracle declaration range")?;
    let mut cursor = start + close + 2;
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
    Ok(cursor)
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
        "SingleVariableDeclaration" => SharedNodeRole::SingleVariableDeclaration,
        "MarkerAnnotation" => SharedNodeRole::MarkerAnnotation,
        "NormalAnnotation" => SharedNodeRole::NormalAnnotation,
        "SingleMemberAnnotation" => SharedNodeRole::SingleMemberAnnotation,
        "SwitchCase" => SharedNodeRole::SwitchCase,
        "TypeParameter" => SharedNodeRole::TypeParameter,
        "VariableDeclarationFragment" => SharedNodeRole::VariableDeclarationFragment,
        "MethodInvocation" => SharedNodeRole::MethodInvocation,
        "ArrayAccess" => SharedNodeRole::ArrayAccess,
        "ArrayCreation" => SharedNodeRole::ArrayCreation,
        "CastExpression" => SharedNodeRole::CastExpression,
        "ClassInstanceCreation" => SharedNodeRole::ClassInstanceCreation,
        "ConditionalExpression" => SharedNodeRole::ConditionalExpression,
        "EmptyStatement" => SharedNodeRole::EmptyStatement,
        "InfixExpression" => SharedNodeRole::InfixExpression,
        "InstanceofExpression" => SharedNodeRole::InstanceofExpression,
        "ParenthesizedExpression" => SharedNodeRole::ParenthesizedExpression,
        "PrefixExpression" => SharedNodeRole::PrefixExpression,
        "ThisExpression" => SharedNodeRole::ThisExpression,
        "LineComment" => SharedNodeRole::LineComment,
        "BlockComment" => SharedNodeRole::BlockComment,
        "QualifiedName" | "FieldAccess" => SharedNodeRole::QualifiedAccess,
        "INFIX_EXPRESSION_OPERATOR" => SharedNodeRole::InfixExpressionOperator,
        "METHOD_INVOCATION_ARGUMENTS" => SharedNodeRole::MethodInvocationArguments,
        "METHOD_INVOCATION_RECEIVER" => SharedNodeRole::MethodInvocationReceiver,
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

/// Classify a tree-sitter-java node type that has exactly one context-free shared role.
///
/// Context-dependent synthetic roles and `annotation`, whose JDT role depends on its argument-list
/// shape, are emitted by [`comparable_tree_sitter_java_nodes`] instead.
pub fn tree_sitter_java_node_role(node_type: &str) -> Option<SharedNodeRole> {
    let roles = tree_sitter_java_node_roles(node_type);
    if roles.len() == 1 {
        Some(roles[0])
    } else {
        None
    }
}

/// Return every context-free shared role compatible with one tree-sitter-java node type.
/// Context narrows `annotation` to one role when comparable nodes are collected.
pub fn tree_sitter_java_node_roles(node_type: &str) -> &'static [SharedNodeRole] {
    match node_type {
        "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" => {
            &[SharedNodeRole::MethodDeclaration]
        }
        "field_declaration" | "constant_declaration" => &[SharedNodeRole::FieldDeclaration],
        "class_declaration" | "interface_declaration" => &[SharedNodeRole::TypeDeclaration],
        "class" | "interface" => &[SharedNodeRole::TypeDeclarationKind],
        "enum_declaration" => &[SharedNodeRole::EnumDeclaration],
        "enum_constant" => &[SharedNodeRole::EnumConstantDeclaration],
        "annotation_type_declaration" => &[SharedNodeRole::AnnotationTypeDeclaration],
        "record_declaration" => &[SharedNodeRole::RecordDeclaration],
        "block" | "constructor_body" => &[SharedNodeRole::Block],
        "assert_statement" => &[SharedNodeRole::AssertStatement],
        "break_statement" => &[SharedNodeRole::BreakStatement],
        "continue_statement" => &[SharedNodeRole::ContinueStatement],
        "do_statement" => &[SharedNodeRole::DoStatement],
        "enhanced_for_statement" => &[SharedNodeRole::EnhancedForStatement],
        "explicit_constructor_invocation" => &[SharedNodeRole::ExplicitConstructorInvocation],
        "expression_statement" => &[SharedNodeRole::ExpressionStatement],
        "for_statement" => &[SharedNodeRole::ForStatement],
        "if_statement" => &[SharedNodeRole::IfStatement],
        "labeled_statement" => &[SharedNodeRole::LabeledStatement],
        "local_variable_declaration" => &[SharedNodeRole::LocalVariableDeclaration],
        "return_statement" => &[SharedNodeRole::ReturnStatement],
        "switch_expression" => &[SharedNodeRole::SwitchConstruct],
        "synchronized_statement" => &[SharedNodeRole::SynchronizedStatement],
        "throw_statement" => &[SharedNodeRole::ThrowStatement],
        "try_statement" | "try_with_resources_statement" => &[SharedNodeRole::TryStatement],
        "while_statement" => &[SharedNodeRole::WhileStatement],
        "yield_statement" => &[SharedNodeRole::YieldStatement],
        "formal_parameter"
        | "catch_formal_parameter"
        | "spread_parameter"
        | "receiver_parameter" => &[SharedNodeRole::SingleVariableDeclaration],
        "marker_annotation" => &[SharedNodeRole::MarkerAnnotation],
        "annotation" => &[
            SharedNodeRole::NormalAnnotation,
            SharedNodeRole::SingleMemberAnnotation,
        ],
        "switch_label" => &[SharedNodeRole::SwitchCase],
        "type_parameter" => &[SharedNodeRole::TypeParameter],
        "variable_declarator" => &[SharedNodeRole::VariableDeclarationFragment],
        "method_invocation" => &[SharedNodeRole::MethodInvocation],
        "array_access" => &[SharedNodeRole::ArrayAccess],
        "array_creation_expression" => &[SharedNodeRole::ArrayCreation],
        "cast_expression" => &[SharedNodeRole::CastExpression],
        "object_creation_expression" => &[SharedNodeRole::ClassInstanceCreation],
        "ternary_expression" => &[SharedNodeRole::ConditionalExpression],
        "binary_expression" => &[SharedNodeRole::InfixExpression],
        "instanceof_expression" => &[SharedNodeRole::InstanceofExpression],
        "parenthesized_expression" => &[SharedNodeRole::ParenthesizedExpression],
        "unary_expression" => &[SharedNodeRole::PrefixExpression],
        "this" => &[SharedNodeRole::ThisExpression],
        "line_comment" => &[SharedNodeRole::LineComment],
        "block_comment" => &[SharedNodeRole::BlockComment],
        "scoped_identifier" | "field_access" => &[SharedNodeRole::QualifiedAccess],
        "identifier" => &[SharedNodeRole::SimpleName],
        "integral_type" | "floating_point_type" | "boolean_type" | "void_type" => {
            &[SharedNodeRole::PrimitiveType]
        }
        "type_identifier" => &[SharedNodeRole::SimpleType],
        "array_type" => &[SharedNodeRole::ArrayType],
        "generic_type" => &[SharedNodeRole::ParameterizedType],
        "public" | "private" | "protected" | "abstract" | "static" | "final" | "strictfp"
        | "default" | "synchronized" | "native" | "transient" | "volatile" | "sealed"
        | "non-sealed" => &[SharedNodeRole::Modifier],
        "true" | "false" => &[SharedNodeRole::BooleanLiteral],
        "character_literal" => &[SharedNodeRole::CharacterLiteral],
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "decimal_floating_point_literal"
        | "hex_floating_point_literal" => &[SharedNodeRole::NumberLiteral],
        "string_literal" => &[SharedNodeRole::StringLiteral],
        "null_literal" => &[SharedNodeRole::NullLiteral],
        "class_literal" => &[SharedNodeRole::TypeLiteral],
        _ => &[],
    }
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
