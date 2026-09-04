use anyhow::{Context, Result, bail};
use tree_sitter::{Node, Parser};

use crate::language::Language;
use crate::model::{NodeRef, Position, Span};

#[derive(Clone, Debug)]
pub struct SyntaxNode {
    pub id: usize,
    pub kind: String,
    pub named: bool,
    pub extra: bool,
    pub missing: bool,
    pub field: Option<String>,
    pub span: Span,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub subtree_size: usize,
    pub byte_hash: [u8; 32],
    pub syntax_hash: [u8; 32],
    pub shape_hash: [u8; 32],
}

impl SyntaxNode {
    pub fn as_ref(&self) -> NodeRef {
        NodeRef {
            id: self.id,
            kind: self.kind.clone(),
            named: self.named,
            extra: self.extra,
            missing: self.missing,
            field: self.field.clone(),
            span: self.span.clone(),
            subtree_size: self.subtree_size,
            syntax_hash: hash_hex(self.syntax_hash),
            shape_hash: hash_hex(self.shape_hash),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParsedSyntax {
    pub source: Vec<u8>,
    pub nodes: Vec<SyntaxNode>,
    pub root: usize,
    pub root_kind: String,
}

pub fn parse(source: Vec<u8>, language: Language) -> Result<ParsedSyntax> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.parser_language())
        .context("failed to initialize the tree-sitter grammar")?;
    let tree = parser
        .parse(&source, None)
        .context("tree-sitter returned no syntax tree")?;
    let root = tree.root_node();
    if root.has_error() {
        let invalid = first_invalid_node(root)
            .context("tree-sitter reported a syntax error without an invalid descendant")?;
        let start = invalid.start_position();
        let end = invalid.end_position();
        bail!(
            "{} parser produced {} bytes {}-{} at {}:{}-{}:{}; refusing to present a partial parse as an exact structural diff",
            format!("{language:?}").to_ascii_lowercase(),
            if invalid.is_missing() {
                "a missing node"
            } else {
                "an ERROR node"
            },
            invalid.start_byte(),
            invalid.end_byte(),
            start.row,
            start.column,
            end.row,
            end.column,
        );
    }

    let mut nodes = Vec::new();
    let root_id = collect(root, None, None, 0, &source, &mut nodes)?;
    Ok(ParsedSyntax {
        source,
        root: root_id,
        root_kind: root.kind().to_owned(),
        nodes,
    })
}

fn first_invalid_node(root: Node<'_>) -> Option<Node<'_>> {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        let mut cursor = node.walk();
        let mut children: Vec<_> = node.children(&mut cursor).collect();
        children.reverse();
        pending.extend(children);
    }
    None
}

fn collect(
    node: Node<'_>,
    parent: Option<usize>,
    field: Option<String>,
    depth: usize,
    source: &[u8],
    nodes: &mut Vec<SyntaxNode>,
) -> Result<usize> {
    const MAX_SYNTAX_DEPTH: usize = 512;
    if depth > MAX_SYNTAX_DEPTH {
        bail!(
            "syntax tree exceeds the supported depth of {MAX_SYNTAX_DEPTH}; refusing recursive analysis"
        );
    }
    let id = nodes.len();
    nodes.push(SyntaxNode {
        id,
        kind: node.kind().to_owned(),
        named: node.is_named(),
        extra: node.is_extra(),
        missing: node.is_missing(),
        field,
        span: span(node),
        parent,
        children: Vec::new(),
        subtree_size: 1,
        byte_hash: [0; 32],
        syntax_hash: [0; 32],
        shape_hash: [0; 32],
    });

    let mut children = Vec::with_capacity(node.child_count() as usize);
    for index in 0..node.child_count() {
        let child = node
            .child(index)
            .expect("child index is bounded by child_count");
        let child_field = node.field_name_for_child(index).map(str::to_owned);
        children.push(collect(
            child,
            Some(id),
            child_field,
            depth + 1,
            source,
            nodes,
        )?);
    }

    let subtree_size = 1 + children
        .iter()
        .map(|child| nodes[*child].subtree_size)
        .sum::<usize>();
    let byte_hash = blake3_hex(b"stratadiff.byte.v1", &source[node.byte_range()]);
    let syntax_hash = syntax_hash(node, &children, source, nodes);
    let shape_hash = shape_hash(node, &children, nodes);

    nodes[id].children = children;
    nodes[id].subtree_size = subtree_size;
    nodes[id].byte_hash = byte_hash;
    nodes[id].syntax_hash = syntax_hash;
    nodes[id].shape_hash = shape_hash;
    Ok(id)
}

fn span(node: Node<'_>) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start: Position {
            row: start.row,
            column: start.column,
        },
        end: Position {
            row: end.row,
            column: end.column,
        },
    }
}

fn syntax_hash(
    node: Node<'_>,
    children: &[usize],
    source: &[u8],
    nodes: &[SyntaxNode],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    put(&mut hasher, b"stratadiff.syntax.v1");
    put(&mut hasher, node.kind().as_bytes());
    put_flags(&mut hasher, node);
    if children.is_empty() {
        put(&mut hasher, &source[node.byte_range()]);
    } else {
        for child in children {
            put_optional(&mut hasher, nodes[*child].field.as_deref());
            put(&mut hasher, &nodes[*child].syntax_hash);
        }
    }
    *hasher.finalize().as_bytes()
}

fn shape_hash(node: Node<'_>, children: &[usize], nodes: &[SyntaxNode]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    put(&mut hasher, b"stratadiff.shape.v1");
    put(&mut hasher, normalized_kind(node.kind()).as_bytes());
    put_flags(&mut hasher, node);
    for child in children {
        put_optional(&mut hasher, nodes[*child].field.as_deref());
        put(&mut hasher, &nodes[*child].shape_hash);
    }
    *hasher.finalize().as_bytes()
}

fn normalized_kind(kind: &str) -> &str {
    if kind.contains("identifier") {
        "<identifier>"
    } else if kind.contains("string") {
        "<string>"
    } else if kind.contains("integer") || kind.contains("float") || kind.contains("number") {
        "<number>"
    } else if kind.contains("comment") {
        "<comment>"
    } else {
        kind
    }
}

fn blake3_hex(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    put(&mut hasher, domain);
    put(&mut hasher, bytes);
    *hasher.finalize().as_bytes()
}

fn put(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn put_optional(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            put(hasher, value.as_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn put_flags(hasher: &mut blake3::Hasher, node: Node<'_>) {
    hasher.update(&[
        u8::from(node.is_named()),
        u8::from(node.is_extra()),
        u8::from(node.is_missing()),
    ]);
}

fn hash_hex(hash: [u8; 32]) -> String {
    blake3::Hash::from_bytes(hash).to_hex().to_string()
}

pub fn syntax_equal(
    left: &ParsedSyntax,
    left_id: usize,
    right: &ParsedSyntax,
    right_id: usize,
) -> bool {
    let left_node = &left.nodes[left_id];
    let right_node = &right.nodes[right_id];
    if left_node.kind != right_node.kind
        || left_node.named != right_node.named
        || left_node.extra != right_node.extra
        || left_node.missing != right_node.missing
        || left_node.children.len() != right_node.children.len()
    {
        return false;
    }
    if left_node.children.is_empty() {
        return left.source[left_node.span.start_byte..left_node.span.end_byte]
            == right.source[right_node.span.start_byte..right_node.span.end_byte];
    }
    left_node
        .children
        .iter()
        .zip(&right_node.children)
        .all(|(left_child, right_child)| {
            left.nodes[*left_child].field == right.nodes[*right_child].field
                && syntax_equal(left, *left_child, right, *right_child)
        })
}

pub fn shape_equal(
    left: &ParsedSyntax,
    left_id: usize,
    right: &ParsedSyntax,
    right_id: usize,
) -> bool {
    let left_node = &left.nodes[left_id];
    let right_node = &right.nodes[right_id];
    if normalized_kind(&left_node.kind) != normalized_kind(&right_node.kind)
        || left_node.named != right_node.named
        || left_node.extra != right_node.extra
        || left_node.missing != right_node.missing
        || left_node.children.len() != right_node.children.len()
    {
        return false;
    }
    left_node
        .children
        .iter()
        .zip(&right_node.children)
        .all(|(left_child, right_child)| {
            left.nodes[*left_child].field == right.nodes[*right_child].field
                && shape_equal(left, *left_child, right, *right_child)
        })
}
