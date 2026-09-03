use stratadiff::diffbenchmark::{GodMappingGroup, GodMappingRecord, GodReport, OffsetRange};
use stratadiff::diffbenchmark_case::adapt_intra_file_case;
use stratadiff::diffbenchmark_eval::{CaseEvaluationInput, evaluate_case};
use stratadiff::diffbenchmark_prediction::{
    EnumeratedJdtNode, PredictionAdapterInput, adapt_predictions,
};
use stratadiff::{Language, analyze_bytes};

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
