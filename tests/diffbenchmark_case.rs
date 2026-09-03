use std::collections::BTreeMap;

use stratadiff::diffbenchmark::{
    GodMappingGroup, GodMappingRecord, GodReport, OffsetRange, SharedNodeRole,
};
use stratadiff::diffbenchmark_case::{
    EndpointExclusionReason, EndpointSide, RelationExclusionReason, adapt_intra_file_case,
};
use stratadiff::diffbenchmark_eval::Multiplicity;

fn record(info: impl Into<String>) -> GodMappingRecord {
    GodMappingRecord {
        left: "diagnostic left text".to_owned(),
        right: "diagnostic right text".to_owned(),
        info: info.into(),
    }
}

fn report(matched_elements: Vec<GodMappingRecord>, mappings: Vec<GodMappingRecord>) -> GodReport {
    GodReport {
        intra_file_mappings: GodMappingGroup {
            matched_elements,
            mappings,
        },
        inter_file_mappings: BTreeMap::new(),
    }
}

fn utf16_range(source: &str, needle: &str) -> OffsetRange {
    let start = source.find(needle).unwrap();
    let end = start + needle.len();
    OffsetRange {
        start: source[..start].encode_utf16().count(),
        end: source[..end].encode_utf16().count(),
    }
}

fn info(
    before_kind: &str,
    before_range: OffsetRange,
    after_kind: &str,
    after_range: OffsetRange,
) -> String {
    format!(
        "{before_kind}[{}-{}]:{after_kind}[{}-{}]",
        before_range.start, before_range.end, after_range.start, after_range.end
    )
}

#[test]
fn raw_multi_survives_when_its_partner_is_excluded() {
    let source = "class A { int x; int y; }";
    let x = utf16_range(source, "x");
    let y = utf16_range(source, "y");
    let god = report(
        vec![
            record(info("SimpleName", x, "SimpleName", x)),
            record(info("SimpleName", x, "UnsupportedNode", y)),
        ],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "src/old/A.java",
        "src/new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();

    assert_eq!(adapted.oracle_relations.program_elements.len(), 1);
    assert_eq!(
        adapted.oracle_relations.program_elements[0].multiplicity,
        Multiplicity::Multi
    );
    assert_eq!(
        adapted.oracle_relations.program_elements[0].raw_multi_group_id,
        Some(0)
    );
    let groups = &adapted.oracle_relations.raw_multi_groups.program_elements;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, 0);
    assert_eq!(groups[0].before_endpoints.len(), 1);
    assert_eq!(groups[0].after_endpoints.len(), 1);
    let coverage = &adapted.coverage.program_elements;
    assert_eq!(coverage.raw_relations, 2);
    assert_eq!(coverage.scorable_relations, 1);
    assert_eq!(coverage.excluded_relations, 1);
    assert_eq!(coverage.exclusions[0].raw_group_id, Some(0));
    let RelationExclusionReason::EndpointFailures { failures } = &coverage.exclusions[0].reason
    else {
        panic!("expected endpoint failure");
    };
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].side, EndpointSide::After);
    assert_eq!(
        failures[0].reason,
        EndpointExclusionReason::UnsupportedJdtKind
    );
}

#[test]
fn raw_component_is_not_split_by_a_role_mismatch_exclusion() {
    let source = "class A { int x = 1; int y = 2; }";
    let x = utf16_range(source, "x");
    let one = utf16_range(source, "1");
    let two = utf16_range(source, "2");
    let god = report(
        vec![
            record(info("SimpleName", x, "SimpleName", x)),
            record(info("SimpleName", x, "NumberLiteral", two)),
            record(info("NumberLiteral", one, "NumberLiteral", two)),
        ],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();

    let relations = &adapted.oracle_relations.program_elements;
    assert_eq!(relations.len(), 2);
    assert!(
        relations
            .iter()
            .all(|relation| relation.multiplicity == Multiplicity::Multi)
    );
    assert!(
        relations
            .iter()
            .all(|relation| relation.raw_multi_group_id == Some(0))
    );
    let groups = &adapted.oracle_relations.raw_multi_groups.program_elements;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, 0);
    assert_eq!(groups[0].before_endpoints.len(), 2);
    assert_eq!(groups[0].after_endpoints.len(), 2);
    assert!(matches!(
        adapted.coverage.program_elements.exclusions[0].reason,
        RelationExclusionReason::RoleMismatch { .. }
    ));
    assert_eq!(
        adapted.coverage.program_elements.exclusions[0].raw_group_id,
        Some(0)
    );
}

#[test]
fn unicode_offsets_convert_to_bytes_without_changing_node_identity() {
    let source = "class A { String s = \"😀\"; int 变量 = 1; }";
    let range = utf16_range(source, "变量");
    let bytes_start = source.find("变量").unwrap();
    let bytes_end = bytes_start + "变量".len();
    let god = report(
        vec![record(info("SimpleName", range, "SimpleName", range))],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "before/Unicode.java",
        "after/Unicode.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();
    let node = &adapted.gold_comparable_endpoints.program_elements.before[0];

    assert_eq!(node.key.file, "before/Unicode.java");
    assert_eq!(node.key.jdt_kind, "SimpleName");
    assert_eq!(node.key.utf16_code_units, range);
    assert_eq!(
        node.utf8_bytes,
        OffsetRange {
            start: bytes_start,
            end: bytes_end,
        }
    );
    assert_eq!(node.role, SharedNodeRole::SimpleName);
    assert_eq!(
        adapted
            .gold_comparable_endpoints
            .program_elements
            .incident_before,
        vec![node.key.clone()]
    );
}

#[test]
fn declaration_javadoc_is_trimmed_only_for_comparable_span() {
    let source = "/** documentation */\nclass A {}";
    let jdt_range = OffsetRange {
        start: 0,
        end: source.encode_utf16().count(),
    };
    let god = report(
        vec![record(info(
            "TypeDeclaration",
            jdt_range,
            "TypeDeclaration",
            jdt_range,
        ))],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();
    let node = &adapted.gold_comparable_endpoints.program_elements.before[0];

    assert_eq!(node.key.utf16_code_units, jdt_range);
    assert_eq!(node.utf8_bytes.start, source.find("class").unwrap());
    assert_eq!(node.utf8_bytes.end, source.len());
    assert_eq!(node.role, SharedNodeRole::TypeDeclaration);
}

#[test]
fn info_is_identity_and_display_text_is_diagnostic_only() {
    let source = "class A {}";
    let range = utf16_range(source, "A");
    let identity = info("SimpleName", range, "SimpleName", range);
    let mut first = record(identity.clone());
    first.left = "not a parseable endpoint".to_owned();
    first.right = "also not an endpoint".to_owned();
    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &report(vec![first], Vec::new()),
    )
    .unwrap();
    assert_eq!(adapted.coverage.program_elements.scorable_relations, 1);

    let mut duplicate = record(identity.clone());
    duplicate.left = "different display".to_owned();
    duplicate.right = "different display too".to_owned();
    let error = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &report(vec![record(identity), duplicate], Vec::new()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate DiffBenchmark"));
    assert!(error.to_string().contains("indices 0 and 1"));
}

#[test]
fn multiplicity_and_duplicates_are_isolated_by_category() {
    let source = "class A {}";
    let range = utf16_range(source, "A");
    let shared_info = info("SimpleName", range, "SimpleName", range);
    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &report(vec![record(shared_info.clone())], vec![record(shared_info)]),
    )
    .unwrap();

    assert_eq!(
        adapted.oracle_relations.program_elements[0].multiplicity,
        Multiplicity::Singleton
    );
    assert_eq!(
        adapted.oracle_relations.program_elements[0].raw_multi_group_id,
        None
    );
    assert_eq!(
        adapted.oracle_relations.mappings[0].multiplicity,
        Multiplicity::Singleton
    );
    assert_eq!(
        adapted
            .gold_comparable_endpoints
            .program_elements
            .before
            .len(),
        1
    );
    assert_eq!(adapted.gold_comparable_endpoints.mappings.before.len(), 1);
}

#[test]
fn both_unsupported_endpoints_exclude_the_edge_once() {
    let source = "class A {}";
    let range = utf16_range(source, "A");
    let god = report(
        vec![record(info(
            "UnsupportedBefore",
            range,
            "UnsupportedAfter",
            range,
        ))],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();
    let coverage = &adapted.coverage.program_elements;

    assert_eq!(coverage.raw_relations, 1);
    assert_eq!(coverage.scorable_relations, 0);
    assert_eq!(coverage.excluded_relations, 1);
    assert_eq!(coverage.exclusions.len(), 1);
    let RelationExclusionReason::EndpointFailures { failures } = &coverage.exclusions[0].reason
    else {
        panic!("expected endpoint failures");
    };
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].side, EndpointSide::Before);
    assert_eq!(failures[1].side, EndpointSide::After);
}

#[test]
fn malformed_info_and_unresolved_span_have_explicit_diagnostics() {
    let source = "class A {}";
    let class_prefix = utf16_range(source, "c");
    let valid_name = utf16_range(source, "A");
    let god = report(
        vec![
            record("not-an-info-value"),
            record(info("SimpleName", class_prefix, "SimpleName", valid_name)),
        ],
        Vec::new(),
    );

    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();
    let exclusions = &adapted.coverage.program_elements.exclusions;

    assert_eq!(exclusions.len(), 2);
    assert!(matches!(
        exclusions[0].reason,
        RelationExclusionReason::InfoParseError { .. }
    ));
    assert_eq!(exclusions[0].raw_group_id, None);
    let RelationExclusionReason::EndpointFailures { failures } = &exclusions[1].reason else {
        panic!("expected endpoint failure");
    };
    assert!(matches!(
        failures[0].reason,
        EndpointExclusionReason::UnresolvedExactRoleAndSpan { .. }
    ));
}

#[test]
fn inter_file_groups_are_out_of_scope() {
    let source = "class A {}";
    let mut god = report(Vec::new(), Vec::new());
    god.inter_file_mappings.insert(
        "Moved to File: B.java".to_owned(),
        GodMappingGroup {
            matched_elements: vec![record("malformed")],
            mappings: vec![record("also malformed")],
        },
    );

    let adapted = adapt_intra_file_case(
        "old/A.java",
        "new/A.java",
        source.as_bytes(),
        source.as_bytes(),
        &god,
    )
    .unwrap();

    assert_eq!(adapted.coverage.program_elements.raw_relations, 0);
    assert_eq!(adapted.coverage.mappings.raw_relations, 0);
}
