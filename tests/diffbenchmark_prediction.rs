use stratadiff::diffbenchmark::{
    GodMappingGroup, GodMappingRecord, GodReport, OffsetRange, SharedNodeRole,
    TreeSitterComparableNode, comparable_tree_sitter_java_node_origins,
};
use stratadiff::diffbenchmark_case::adapt_intra_file_case;
use stratadiff::diffbenchmark_eval::{CaseEvaluationInput, evaluate_case};
use stratadiff::diffbenchmark_prediction::{
    EnumeratedJdtNode, PredictionAdapterInput, adapt_predictions,
};
use stratadiff::{
    AmbiguityAbstentionCause, AmbiguityConstraint, AmbiguityGroup, AmbiguityPair, Language,
    NodeRef, PairClaims, Position, Predicate, Span, analyze_bytes,
};

#[test]
fn projects_forced_relations_through_exact_enumerated_jdt_keys() {
    let before = b"class Demo { int oldName; }\n";
    let after = b"class Demo { int newName; }\n";
    let before_field = range(before, "int oldName;");
    let after_field = range(after, "int newName;");
    let before_name = range(before, "oldName");
    let after_name = range(after, "newName");
    let god = GodReport {
        intra_file_mappings: GodMappingGroup {
            matched_elements: vec![record("FieldDeclaration", before_field, after_field)],
            mappings: vec![record("SimpleName", before_name, after_name)],
        },
        inter_file_mappings: Default::default(),
    };
    let oracle = adapt_intra_file_case("Demo.java", "Demo.java", before, after, &god).unwrap();
    let report = analyze_bytes(
        before.to_vec(),
        after.to_vec(),
        "Demo.java".to_owned(),
        "Demo.java".to_owned(),
        Language::Java,
    )
    .unwrap();
    let adapted = adapt_predictions(&PredictionAdapterInput {
        before_file: "Demo.java",
        after_file: "Demo.java",
        before_source: before,
        after_source: after,
        before_jdt_nodes: &[
            enumerated("FieldDeclaration", before_field),
            enumerated("SimpleName", before_name),
        ],
        after_jdt_nodes: &[
            enumerated("FieldDeclaration", after_field),
            enumerated("SimpleName", after_name),
        ],
        oracle: &oracle,
        report: &report,
    })
    .unwrap();
    let evaluation = evaluate_case(&CaseEvaluationInput {
        universe: adapted.universe,
        oracle: oracle.oracle_relations,
        prediction: adapted.predictions,
    })
    .unwrap();

    assert_eq!(
        evaluation.program_elements.exact_relations.true_positives,
        1
    );
    assert_eq!(evaluation.mappings.exact_relations.true_positives, 1);
    assert_eq!(
        evaluation.program_elements.exact_relations.false_positives,
        0
    );
    assert_eq!(evaluation.mappings.exact_relations.false_positives, 0);
}

#[test]
fn projects_only_explicit_ordered_ambiguity_pairs() {
    let before = concat!(
        "class Demo {\n",
        "  int addOld(int value) { return value + 1; }\n",
        "  int multiplyOld(int value) { return value * 2; }\n",
        "}\n",
    )
    .as_bytes();
    let after = concat!(
        "class Demo {\n",
        "  int multiplyNew(int item) { return item * 3; }\n",
        "  int addNew(int item) { return item + 4; }\n",
        "}\n",
    )
    .as_bytes();
    let before_add = range(before, "int addOld(int value) { return value + 1; }");
    let before_multiply = range(before, "int multiplyOld(int value) { return value * 2; }");
    let after_multiply = range(after, "int multiplyNew(int item) { return item * 3; }");
    let after_add = range(after, "int addNew(int item) { return item + 4; }");
    let god = GodReport {
        intra_file_mappings: GodMappingGroup {
            matched_elements: vec![
                record("MethodDeclaration", before_add, after_add),
                record("MethodDeclaration", before_multiply, after_multiply),
            ],
            mappings: Vec::new(),
        },
        inter_file_mappings: Default::default(),
    };
    let oracle = adapt_intra_file_case("Demo.java", "Demo.java", before, after, &god).unwrap();
    let mut report = analyze_bytes(
        before.to_vec(),
        after.to_vec(),
        "Demo.java".to_owned(),
        "Demo.java".to_owned(),
        Language::Java,
    )
    .unwrap();
    let before_refs = method_refs(before);
    let after_refs = method_refs(after);
    report
        .relations
        .retain(|relation| relation.before.kind != "method_declaration");
    report.ambiguities = vec![AmbiguityGroup {
        parent_before: 0,
        parent_after: 0,
        before: before_refs.clone(),
        after: after_refs.clone(),
        constraint: AmbiguityConstraint::ExactOrderedAlignment {
            predicate: Predicate::ShapeEqual,
            required_matches: 1,
            possible_pairs: vec![
                AmbiguityPair {
                    before_id: before_refs[0].id,
                    after_id: after_refs[1].id,
                },
                AmbiguityPair {
                    before_id: before_refs[1].id,
                    after_id: after_refs[0].id,
                },
            ],
        },
        reason: "test exact choices".to_owned(),
    }];

    let adapted = adapt_predictions(&PredictionAdapterInput {
        before_file: "Demo.java",
        after_file: "Demo.java",
        before_source: before,
        after_source: after,
        before_jdt_nodes: &[
            enumerated("MethodDeclaration", before_add),
            enumerated("MethodDeclaration", before_multiply),
        ],
        after_jdt_nodes: &[
            enumerated("MethodDeclaration", after_multiply),
            enumerated("MethodDeclaration", after_add),
        ],
        oracle: &oracle,
        report: &report,
    })
    .unwrap();

    assert_eq!(
        adapted
            .predictions
            .program_elements
            .ambiguity_candidates
            .len(),
        2
    );
}

#[test]
fn symbolic_abstention_projects_no_pair_candidates() {
    let source = concat!(
        "class Demo {\n",
        "  int same() { return 1; }\n",
        "  int same() { return 1; }\n",
        "}\n",
    )
    .as_bytes();
    let method_ranges = ranges(source, "int same() { return 1; }");
    let god = GodReport {
        intra_file_mappings: GodMappingGroup {
            matched_elements: vec![record(
                "MethodDeclaration",
                method_ranges[0],
                method_ranges[0],
            )],
            mappings: Vec::new(),
        },
        inter_file_mappings: Default::default(),
    };
    let oracle = adapt_intra_file_case("Demo.java", "Demo.java", source, source, &god).unwrap();
    let mut report = analyze_bytes(
        source.to_vec(),
        source.to_vec(),
        "Demo.java".to_owned(),
        "Demo.java".to_owned(),
        Language::Java,
    )
    .unwrap();
    let refs = method_refs(source);
    report
        .relations
        .retain(|relation| relation.before.kind != "method_declaration");
    report.ambiguities = vec![AmbiguityGroup {
        parent_before: 0,
        parent_after: 0,
        before: refs.clone(),
        after: refs.clone(),
        constraint: AmbiguityConstraint::SymbolicAbstention {
            cause: AmbiguityAbstentionCause::DuplicateSymmetry,
            pair_claims: PairClaims::None,
        },
        reason: "test symbolic abstention".to_owned(),
    }];

    let nodes = [
        enumerated("MethodDeclaration", method_ranges[0]),
        enumerated("MethodDeclaration", method_ranges[1]),
    ];
    let adapted = adapt_predictions(&PredictionAdapterInput {
        before_file: "Demo.java",
        after_file: "Demo.java",
        before_source: source,
        after_source: source,
        before_jdt_nodes: &nodes,
        after_jdt_nodes: &nodes,
        oracle: &oracle,
        report: &report,
    })
    .unwrap();

    assert!(
        adapted
            .predictions
            .program_elements
            .ambiguity_candidates
            .is_empty()
    );
}

fn record(kind: &str, before: OffsetRange, after: OffsetRange) -> GodMappingRecord {
    GodMappingRecord {
        left: String::new(),
        right: String::new(),
        info: format!(
            "{kind}[{}-{}]:{kind}[{}-{}]",
            before.start, before.end, after.start, after.end
        ),
    }
}

fn enumerated(kind: &str, utf16_code_units: OffsetRange) -> EnumeratedJdtNode {
    EnumeratedJdtNode {
        node_type: kind.to_owned(),
        utf16_code_units,
    }
}

fn range(source: &[u8], fragment: &str) -> OffsetRange {
    let source = std::str::from_utf8(source).unwrap();
    let start = source.find(fragment).unwrap();
    OffsetRange {
        start: source[..start].encode_utf16().count(),
        end: source[..start + fragment.len()].encode_utf16().count(),
    }
}

fn ranges(source: &[u8], fragment: &str) -> Vec<OffsetRange> {
    let source = std::str::from_utf8(source).unwrap();
    source
        .match_indices(fragment)
        .map(|(start, value)| OffsetRange {
            start: source[..start].encode_utf16().count(),
            end: source[..start + value.len()].encode_utf16().count(),
        })
        .collect()
}

fn method_refs(source: &[u8]) -> Vec<NodeRef> {
    comparable_tree_sitter_java_node_origins(source)
        .unwrap()
        .into_iter()
        .filter(|origin| origin.comparable.role == SharedNodeRole::MethodDeclaration)
        .map(node_ref)
        .collect()
}

fn node_ref(origin: TreeSitterComparableNode) -> NodeRef {
    NodeRef {
        id: origin.origin_id,
        kind: origin.origin_kind,
        named: true,
        extra: false,
        missing: false,
        field: None,
        span: Span {
            start_byte: origin.origin_utf8_bytes.start,
            end_byte: origin.origin_utf8_bytes.end,
            start: Position { row: 0, column: 0 },
            end: Position { row: 0, column: 0 },
        },
        subtree_size: 1,
        syntax_hash: String::new(),
        shape_hash: String::new(),
    }
}
