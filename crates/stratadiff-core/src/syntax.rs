use std::ops::ControlFlow;

use anyhow::{Context, Result, bail};
use tree_sitter::{Node, ParseOptions, ParseState, Parser};

use crate::language::Language;
use crate::model::{NodeRef, Position, Span};

const MAX_SYNTAX_DEPTH: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_parse_callbacks: usize,
}

impl ParseLimits {
    pub const COMPATIBILITY: Self = Self {
        max_nodes: usize::MAX,
        max_depth: MAX_SYNTAX_DEPTH,
        max_parse_callbacks: usize::MAX,
    };
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self::COMPATIBILITY
    }
}

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
    pub language: Language,
}

pub fn parse(source: Vec<u8>, language: Language) -> Result<ParsedSyntax> {
    parse_with_limits(source, language, &ParseLimits::COMPATIBILITY)
}

pub fn parse_with_limits(
    source: Vec<u8>,
    language: Language,
    limits: &ParseLimits,
) -> Result<ParsedSyntax> {
    if language == Language::Universal {
        return parse_universal(source, limits);
    }

    let parser_language = language
        .tree_sitter_language()
        .context("selected language does not provide a tree-sitter grammar")?;
    let mut parser = Parser::new();
    parser
        .set_language(&parser_language)
        .context("failed to initialize the tree-sitter grammar")?;
    let tree = if limits.max_parse_callbacks == usize::MAX {
        parser.parse(&source, None)
    } else {
        let mut callback_count = 0;
        let mut callback_limit_exceeded = false;
        let tree = {
            let mut progress = |_state: &ParseState| {
                if callback_count >= limits.max_parse_callbacks {
                    callback_limit_exceeded = true;
                    ControlFlow::Break(())
                } else {
                    callback_count += 1;
                    ControlFlow::Continue(())
                }
            };
            let options = ParseOptions::new().progress_callback(&mut progress);
            let mut read = |offset, _position| source.get(offset..).unwrap_or_default();
            parser.parse_with_options(&mut read, None, Some(options))
        };
        if callback_limit_exceeded {
            bail!(
                "tree-sitter parse exceeds the supported callback count of {}; refusing unbounded parsing",
                limits.max_parse_callbacks
            );
        }
        tree
    }
    .context("tree-sitter returned no syntax tree")?;
    let root = tree.root_node();
    let mut nodes = Vec::new();
    let root_id = collect(
        root,
        None,
        None,
        0,
        &mut CollectContext {
            source: &source,
            nodes: &mut nodes,
            language,
            limits,
        },
    )?;
    Ok(ParsedSyntax {
        source,
        root: root_id,
        root_kind: root.kind().to_owned(),
        nodes,
        language,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UniversalTokenClass {
    AsciiWord,
    Whitespace,
    LineFeed,
    AsciiPunctuation,
    OpaqueBytes,
}

impl UniversalTokenClass {
    fn of(byte: u8) -> Self {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => Self::AsciiWord,
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => Self::Whitespace,
            b'\n' => Self::LineFeed,
            0x21..=0x7e => Self::AsciiPunctuation,
            _ => Self::OpaqueBytes,
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::AsciiWord => "universal_ascii_word",
            Self::Whitespace => "universal_whitespace",
            Self::LineFeed => "universal_line_feed",
            Self::AsciiPunctuation => "universal_ascii_punctuation",
            Self::OpaqueBytes => "universal_opaque_bytes",
        }
    }
}

fn parse_universal(source: Vec<u8>, limits: &ParseLimits) -> Result<ParsedSyntax> {
    let mut nodes = Vec::new();
    let root = push_universal_node(
        &mut nodes,
        limits,
        0,
        "universal_file",
        None,
        None,
        Span {
            start_byte: 0,
            end_byte: source.len(),
            start: Position { row: 0, column: 0 },
            end: universal_end_position(&source),
        },
    )?;

    let mut root_children = Vec::new();
    let mut line_start = 0;
    let mut row = 0;
    while line_start < source.len() {
        let line_end = source[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |relative| line_start + relative + 1);
        let has_line_feed = source[line_end - 1] == b'\n';
        let line_end_position = if has_line_feed {
            Position {
                row: row + 1,
                column: 0,
            }
        } else {
            Position {
                row,
                column: line_end - line_start,
            }
        };
        let line = push_universal_node(
            &mut nodes,
            limits,
            1,
            "universal_line",
            Some(root),
            Some("line".to_owned()),
            Span {
                start_byte: line_start,
                end_byte: line_end,
                start: Position { row, column: 0 },
                end: line_end_position,
            },
        )?;
        push_universal_child(&mut root_children, line, "universal root children")?;

        let mut token_children = Vec::new();
        let mut token_start = line_start;
        while token_start < line_end {
            let class = UniversalTokenClass::of(source[token_start]);
            let mut token_end = token_start + 1;
            while token_end < line_end && UniversalTokenClass::of(source[token_end]) == class {
                token_end += 1;
            }
            let start_column = token_start - line_start;
            let end_position = if class == UniversalTokenClass::LineFeed {
                Position {
                    row: row + 1,
                    column: 0,
                }
            } else {
                Position {
                    row,
                    column: token_end - line_start,
                }
            };
            let token = push_universal_node(
                &mut nodes,
                limits,
                2,
                class.kind(),
                Some(line),
                Some("token".to_owned()),
                Span {
                    start_byte: token_start,
                    end_byte: token_end,
                    start: Position {
                        row,
                        column: start_column,
                    },
                    end: end_position,
                },
            )?;
            finish_universal_node(&source, &mut nodes, token);
            push_universal_child(&mut token_children, token, "universal line tokens")?;
            token_start = token_end;
        }
        nodes[line].children = token_children;
        finish_universal_node(&source, &mut nodes, line);

        line_start = line_end;
        if has_line_feed {
            row += 1;
        }
    }
    nodes[root].children = root_children;
    finish_universal_node(&source, &mut nodes, root);

    Ok(ParsedSyntax {
        source,
        nodes,
        root,
        root_kind: "universal_file".to_owned(),
        language: Language::Universal,
    })
}

fn push_universal_node(
    nodes: &mut Vec<SyntaxNode>,
    limits: &ParseLimits,
    depth: usize,
    kind: &str,
    parent: Option<usize>,
    field: Option<String>,
    span: Span,
) -> Result<usize> {
    if depth > limits.max_depth {
        bail!(
            "universal syntax tree exceeds the supported depth of {}; refusing recursive analysis",
            limits.max_depth
        );
    }
    if nodes.len() >= limits.max_nodes {
        bail!(
            "universal syntax tree exceeds the supported node count of {}; refusing analysis",
            limits.max_nodes
        );
    }
    if nodes.len() == nodes.capacity() {
        nodes
            .try_reserve(1)
            .context("failed to reserve bounded universal syntax nodes")?;
    }
    let id = nodes.len();
    nodes.push(SyntaxNode {
        id,
        kind: kind.to_owned(),
        named: true,
        extra: false,
        missing: false,
        field,
        span,
        parent,
        children: Vec::new(),
        subtree_size: 1,
        byte_hash: [0; 32],
        syntax_hash: [0; 32],
        shape_hash: [0; 32],
    });
    Ok(id)
}

fn push_universal_child(children: &mut Vec<usize>, child: usize, label: &str) -> Result<()> {
    if children.len() == children.capacity() {
        children
            .try_reserve(1)
            .with_context(|| format!("failed to reserve bounded {label}"))?;
    }
    children.push(child);
    Ok(())
}

fn finish_universal_node(source: &[u8], nodes: &mut [SyntaxNode], id: usize) {
    let node = &nodes[id];
    let subtree_size = 1 + node
        .children
        .iter()
        .map(|child| nodes[*child].subtree_size)
        .sum::<usize>();
    let byte_hash = blake3_hex(
        b"stratadiff.byte.v1",
        &source[node.span.start_byte..node.span.end_byte],
    );
    let syntax_hash =
        universal_structural_hash(b"stratadiff.syntax.v1", source, node, nodes, false);
    let shape_hash = universal_structural_hash(b"stratadiff.shape.v1", source, node, nodes, true);
    let node = &mut nodes[id];
    node.subtree_size = subtree_size;
    node.byte_hash = byte_hash;
    node.syntax_hash = syntax_hash;
    node.shape_hash = shape_hash;
}

fn universal_structural_hash(
    domain: &[u8],
    source: &[u8],
    node: &SyntaxNode,
    nodes: &[SyntaxNode],
    shape: bool,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    put(&mut hasher, domain);
    put(&mut hasher, node.kind.as_bytes());
    put_flags_values(&mut hasher, node.named, node.extra, node.missing);
    if node.children.is_empty() {
        // Universal token classes only define boundaries; they do not make semantic equivalence
        // claims, so both structural hashes retain the exact leaf bytes.
        put(
            &mut hasher,
            &source[node.span.start_byte..node.span.end_byte],
        );
    } else {
        for child in &node.children {
            put_optional(&mut hasher, nodes[*child].field.as_deref());
            put(
                &mut hasher,
                if shape {
                    &nodes[*child].shape_hash
                } else {
                    &nodes[*child].syntax_hash
                },
            );
        }
    }
    *hasher.finalize().as_bytes()
}

fn universal_end_position(source: &[u8]) -> Position {
    let mut row = 0;
    let mut column = 0;
    for byte in source {
        if *byte == b'\n' {
            row += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    Position { row, column }
}

struct CollectContext<'a> {
    source: &'a [u8],
    nodes: &'a mut Vec<SyntaxNode>,
    language: Language,
    limits: &'a ParseLimits,
}

fn collect(
    node: Node<'_>,
    parent: Option<usize>,
    field: Option<String>,
    depth: usize,
    context: &mut CollectContext<'_>,
) -> Result<usize> {
    if depth > context.limits.max_depth {
        bail!(
            "syntax tree exceeds the supported depth of {}; refusing recursive analysis",
            context.limits.max_depth
        );
    }
    if context.nodes.len() >= context.limits.max_nodes {
        bail!(
            "syntax tree exceeds the supported node count of {}; refusing recursive analysis",
            context.limits.max_nodes
        );
    }
    if node.is_error() || node.is_missing() {
        let start = node.start_position();
        let end = node.end_position();
        bail!(
            "{} parser produced {} bytes {}-{} at {}:{}-{}:{}; refusing to present a partial parse as an exact structural diff",
            format!("{:?}", context.language).to_ascii_lowercase(),
            if node.is_missing() {
                "a missing node"
            } else {
                "an ERROR node"
            },
            node.start_byte(),
            node.end_byte(),
            start.row,
            start.column,
            end.row,
            end.column,
        );
    }
    let id = context.nodes.len();
    context.nodes.push(SyntaxNode {
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

    let child_count = node.child_count() as usize;
    let remaining_nodes = context.limits.max_nodes - context.nodes.len();
    if child_count > remaining_nodes {
        bail!(
            "syntax tree exceeds the supported node count of {}; refusing recursive analysis",
            context.limits.max_nodes
        );
    }
    let mut children = Vec::new();
    children
        .try_reserve_exact(child_count)
        .context("failed to reserve bounded syntax children")?;
    for index in 0..node.child_count() {
        let child = node
            .child(index)
            .expect("child index is bounded by child_count");
        let child_field = node.field_name_for_child(index).map(str::to_owned);
        children.push(collect(child, Some(id), child_field, depth + 1, context)?);
    }

    let subtree_size = 1 + children
        .iter()
        .map(|child| context.nodes[*child].subtree_size)
        .sum::<usize>();
    let byte_hash = blake3_hex(b"stratadiff.byte.v1", &context.source[node.byte_range()]);
    let syntax_hash = syntax_hash(node, &children, context.source, context.nodes);
    let shape_hash = shape_hash(node, &children, context.nodes);

    context.nodes[id].children = children;
    context.nodes[id].subtree_size = subtree_size;
    context.nodes[id].byte_hash = byte_hash;
    context.nodes[id].syntax_hash = syntax_hash;
    context.nodes[id].shape_hash = shape_hash;
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
    put_flags_values(hasher, node.is_named(), node.is_extra(), node.is_missing());
}

fn put_flags_values(hasher: &mut blake3::Hasher, named: bool, extra: bool, missing: bool) {
    hasher.update(&[u8::from(named), u8::from(extra), u8::from(missing)]);
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
    if left.language != right.language {
        return false;
    }
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
    if left.language != right.language {
        return false;
    }
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
    if left.language == Language::Universal && left_node.children.is_empty() {
        return left.source[left_node.span.start_byte..left_node.span.end_byte]
            == right.source[right_node.span.start_byte..right_node.span.end_byte];
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

#[cfg(test)]
mod tests {
    use super::{ParseLimits, parse, parse_with_limits, shape_equal};
    use crate::{Language, Position};

    #[test]
    fn bounded_parse_accepts_a_normal_input() {
        let parsed = parse_with_limits(
            br#"{"value": 1}"#.to_vec(),
            Language::Json,
            &ParseLimits {
                max_nodes: 32,
                max_depth: 16,
                max_parse_callbacks: 1_000,
            },
        )
        .unwrap();

        assert_eq!(parsed.root_kind, "document");
        assert!(parsed.nodes.len() <= 32);
    }

    #[test]
    fn node_limit_is_checked_before_inserting_the_next_node() {
        let error = parse_with_limits(
            br#"{"value": 1}"#.to_vec(),
            Language::Json,
            &ParseLimits {
                max_nodes: 1,
                max_depth: 16,
                max_parse_callbacks: usize::MAX,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("supported node count of 1"));
    }

    #[test]
    fn depth_limit_is_checked_before_descending() {
        let error = parse_with_limits(
            b"[]".to_vec(),
            Language::Json,
            &ParseLimits {
                max_nodes: 32,
                max_depth: 0,
                max_parse_callbacks: usize::MAX,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("supported depth of 0"));
    }

    #[test]
    fn parser_callback_limit_cancels_with_a_diagnostic() {
        let source = "value = 1\n".repeat(10_000).into_bytes();
        let error = parse_with_limits(
            source,
            Language::Python,
            &ParseLimits {
                max_nodes: usize::MAX,
                max_depth: 512,
                max_parse_callbacks: 0,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("supported callback count of 0"));
    }

    #[test]
    fn invalid_tree_diagnostics_remain_inside_the_node_budget() {
        let source = b"[0,]".to_vec();
        let bounded_error = parse_with_limits(
            source.clone(),
            Language::Json,
            &ParseLimits {
                max_nodes: 1,
                max_depth: 16,
                max_parse_callbacks: usize::MAX,
            },
        )
        .unwrap_err();
        assert!(
            bounded_error
                .to_string()
                .contains("supported node count of 1")
        );

        let syntax_error = parse_with_limits(
            source,
            Language::Json,
            &ParseLimits {
                max_nodes: 32,
                max_depth: 16,
                max_parse_callbacks: usize::MAX,
            },
        )
        .unwrap_err();
        assert!(syntax_error.to_string().contains("parser produced"));
    }

    #[test]
    fn compatibility_entry_point_keeps_the_existing_depth_policy() {
        let deeply_nested = format!("{}0{}", "[".repeat(600), "]".repeat(600));
        let error = parse(deeply_nested.into_bytes(), Language::Json).unwrap_err();

        assert!(error.to_string().contains("supported depth of 512"));
    }

    #[test]
    fn universal_parser_builds_a_byte_defined_line_and_token_tree() {
        let source = vec![0xff, b'a', b'b', b' ', b'+', b'\r', b'\n', 0, b'_'];
        let parsed = parse(source, Language::Universal).unwrap();

        assert_eq!(parsed.language, Language::Universal);
        assert_eq!(parsed.root, 0);
        assert_eq!(parsed.root_kind, "universal_file");
        assert_eq!(parsed.nodes.len(), 11);
        assert_eq!(parsed.nodes[0].children, [1, 8]);
        assert_eq!(parsed.nodes[1].children, [2, 3, 4, 5, 6, 7]);
        assert_eq!(parsed.nodes[8].children, [9, 10]);
        assert_eq!(parsed.nodes[2].kind, "universal_opaque_bytes");
        assert_eq!(parsed.nodes[3].kind, "universal_ascii_word");
        assert_eq!(parsed.nodes[4].kind, "universal_whitespace");
        assert_eq!(parsed.nodes[5].kind, "universal_ascii_punctuation");
        assert_eq!(parsed.nodes[6].kind, "universal_whitespace");
        assert_eq!(parsed.nodes[7].kind, "universal_line_feed");
        assert_eq!(parsed.nodes[9].kind, "universal_opaque_bytes");
        assert_eq!(parsed.nodes[10].kind, "universal_ascii_word");
        assert_eq!(parsed.nodes[1].span.end, Position { row: 1, column: 0 });
        assert_eq!(parsed.nodes[8].span.start, Position { row: 1, column: 0 });
        assert_eq!(parsed.nodes[0].span.end, Position { row: 1, column: 2 });
        assert_eq!(parsed.nodes[0].subtree_size, parsed.nodes.len());
    }

    #[test]
    fn universal_shape_equality_does_not_guess_changed_token_identity() {
        let before = parse(b"old".to_vec(), Language::Universal).unwrap();
        let after = parse(b"new".to_vec(), Language::Universal).unwrap();

        assert!(!shape_equal(&before, 0, &after, 0));
        assert!(!shape_equal(&before, 2, &after, 2));
    }

    #[test]
    fn universal_token_runs_cover_every_byte_value_without_decoding() {
        let source: Vec<u8> = (u8::MIN..=u8::MAX).collect();
        let parsed = parse(source.clone(), Language::Universal).unwrap();
        let mut cursor = 0;
        for token in parsed
            .nodes
            .iter()
            .filter(|node| node.field.as_deref() == Some("token"))
        {
            assert_eq!(token.span.start_byte, cursor);
            cursor = token.span.end_byte;
        }
        assert_eq!(cursor, source.len());

        let empty = parse(Vec::new(), Language::Universal).unwrap();
        assert_eq!(empty.nodes.len(), 1);
        assert_eq!(empty.nodes[0].span.start, Position { row: 0, column: 0 });
        assert_eq!(empty.nodes[0].span.end, Position { row: 0, column: 0 });
    }

    #[test]
    fn universal_line_boundaries_cover_empty_crlf_and_unterminated_inputs() {
        let cases: &[(&[u8], usize, Position)] = &[
            (b"", 0, Position { row: 0, column: 0 }),
            (b"\n", 1, Position { row: 1, column: 0 }),
            (b"\n\n", 2, Position { row: 2, column: 0 }),
            (b"\r", 1, Position { row: 0, column: 1 }),
            (b"a\r\nb\rc\n", 2, Position { row: 2, column: 0 }),
            (b"a\r\nb\rc", 2, Position { row: 1, column: 3 }),
        ];

        for (source, expected_lines, expected_end) in cases {
            let parsed = parse(source.to_vec(), Language::Universal).unwrap();
            let root = &parsed.nodes[parsed.root];
            assert_eq!(root.children.len(), *expected_lines, "source: {source:?}");
            assert_eq!(root.span.end, *expected_end, "source: {source:?}");

            let mut cursor = 0;
            for line_id in &root.children {
                let line = &parsed.nodes[*line_id];
                assert_eq!(line.parent, Some(parsed.root));
                assert_eq!(line.field.as_deref(), Some("line"));
                assert_eq!(line.span.start_byte, cursor);
                for token_id in &line.children {
                    let token = &parsed.nodes[*token_id];
                    assert_eq!(token.parent, Some(*line_id));
                    assert_eq!(token.field.as_deref(), Some("token"));
                    assert!(token.children.is_empty());
                    assert_eq!(token.span.start_byte, cursor);
                    cursor = token.span.end_byte;
                }
                assert_eq!(line.span.end_byte, cursor);
            }
            assert_eq!(cursor, source.len());
        }
    }

    #[test]
    fn universal_parser_obeys_node_and_depth_limits() {
        let exact = ParseLimits {
            max_nodes: 3,
            max_depth: 2,
            max_parse_callbacks: 0,
        };
        assert!(parse_with_limits(b"x".to_vec(), Language::Universal, &exact).is_ok());

        let node_error = parse_with_limits(
            b"x".to_vec(),
            Language::Universal,
            &ParseLimits {
                max_nodes: 2,
                ..exact
            },
        )
        .unwrap_err();
        assert!(node_error.to_string().contains("supported node count of 2"));

        let depth_error = parse_with_limits(
            b"x".to_vec(),
            Language::Universal,
            &ParseLimits {
                max_depth: 1,
                ..exact
            },
        )
        .unwrap_err();
        assert!(depth_error.to_string().contains("supported depth of 1"));
    }
}
