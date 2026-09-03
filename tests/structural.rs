use stratadiff::{ChangeKind, Correspondence, Language, Predicate, analyze_bytes, verify_report};

fn python(before: &str, after: &str) -> stratadiff::DiffReport {
    analyze_bytes(
        before.as_bytes().to_vec(),
        after.as_bytes().to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap()
}

#[test]
fn duplicate_subtrees_are_ambiguous_instead_of_guessed() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = python(source, source);

    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 2
            && group.after.len() == 2
            && group
                .before
                .iter()
                .all(|node| node.kind == "function_definition")
    }));
    assert!(!report.changes.iter().any(|change| {
        matches!(change.kind, ChangeKind::Insert | ChangeKind::Delete)
            && change
                .before
                .as_ref()
                .or(change.after.as_ref())
                .is_some_and(|node| node.kind == "function_definition")
    }));
    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.correspondence == Correspondence::ModelForced
    }));
}

#[test]
fn a_unique_shape_change_is_forced_by_every_optimal_alignment() {
    let report = python(
        "def greet(name):\n    return 'hello ' + name\n",
        "def welcome(person):\n    return 'hello ' + person\n",
    );

    assert!(report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.predicate == Predicate::ShapeEqual
            && relation.correspondence == Correspondence::ModelForced
            && relation
                .evidence
                .iter()
                .any(|evidence| evidence == "pair_present_in_every_optimal_alignment")
    }));
    assert!(report.changes.iter().any(|change| {
        change.kind == ChangeKind::ModelForcedUpdate
            && change
                .before
                .as_ref()
                .is_some_and(|node| node.kind == "function_definition")
    }));
}

#[test]
fn crossing_shape_candidates_remain_ambiguous() {
    let before = concat!(
        "def add_old(value):\n    return value + 1\n\n",
        "def multiply_old(value):\n    return value * 2\n",
    );
    let after = concat!(
        "def multiply_new(item):\n    return item * 3\n\n",
        "def add_new(item):\n    return item + 4\n",
    );
    let report = python(before, after);

    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.correspondence == Correspondence::ModelForced
    }));
    assert_eq!(
        report
            .ambiguities
            .iter()
            .filter(|group| {
                group.before.len() == 1
                    && group.after.len() == 1
                    && group.before[0].kind == "function_definition"
            })
            .count(),
        2
    );
}

#[test]
fn wrapper_insertion_keeps_only_the_exact_inner_anchor() {
    let before = "def render():\n    emit(value)\n";
    let after = "def render():\n    if ready:\n        emit(value)\n";
    let report = python(before, after);

    assert!(report.relations.iter().any(|relation| {
        relation.before.kind == "expression_statement"
            && relation.correspondence == Correspondence::ModelForced
            && matches!(
                relation.predicate,
                Predicate::ByteEqual | Predicate::SyntaxEqual
            )
    }));
    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition" && relation.predicate == Predicate::ShapeEqual
    }));
}

#[test]
fn move_plus_edit_does_not_create_a_forced_crossing_pair() {
    let before = concat!(
        "def left_old(value):\n    return value + 1\n\n",
        "def right_old(value):\n    return value * 2\n",
    );
    let after = concat!(
        "def right_new(item):\n    return item * 30\n\n",
        "def left_new(item):\n    return item + 40\n",
    );
    let report = python(before, after);

    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.predicate == Predicate::ShapeEqual
            && relation.correspondence == Correspondence::ModelForced
    }));
    assert!(report.ambiguities.iter().any(|group| {
        group
            .before
            .iter()
            .any(|node| node.kind == "function_definition")
            && group.reason.contains("optimal ordered alignment")
    }));
}

#[test]
fn edited_move_cannot_cross_an_exact_sibling_anchor() {
    let before = concat!(
        "def moved(value):\n    return value + 1\n\n",
        "def stable():\n    return 0\n",
    );
    let after = concat!(
        "def stable():\n    return 0\n\n",
        "def moved(item):\n    return item + 2\n",
    );
    let report = python(before, after);

    assert!(report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && matches!(
                relation.predicate,
                Predicate::ByteEqual | Predicate::SyntaxEqual
            )
            && relation.correspondence == Correspondence::ModelForced
    }));
    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.predicate == Predicate::ShapeEqual
            && relation.correspondence == Correspondence::ModelForced
    }));
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn duplicate_symmetry_closure_does_not_label_an_unselected_copy_deleted() {
    let before = concat!(
        "old_x_one()\n",
        "left_a = 1\n",
        "while left_b:\n    old_body_b()\n",
        "if left_c:\n    old_body_c()\n",
        "old_x_two()\n",
    );
    let after = concat!(
        "right_a = 2\n",
        "while right_b:\n    new_body_b()\n",
        "if right_c:\n    new_body_c()\n",
        "new_x()\n",
    );
    let report = python(before, after);

    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 2
            && group.after.len() == 1
            && group
                .before
                .iter()
                .all(|node| node.kind == "expression_statement")
    }));
    assert!(!report.changes.iter().any(|change| {
        change.kind == ChangeKind::Delete
            && change.before.as_ref().is_some_and(|node| {
                node.kind == "expression_statement"
                    && (node.span.start.row == 0 || node.span.start.row == 6)
            })
    }));
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn oversized_child_region_abstains_without_building_an_alignment_matrix() {
    let mut before = String::new();
    let mut after = String::new();
    for index in 0..65 {
        before.push_str(&format!(
            "def old_{index}(value_{index}):\n    return value_{index} + {index}\n\n"
        ));
        after.push_str(&format!(
            "def new_{index}(item_{index}):\n    return item_{index} + {}\n\n",
            index + 1_000
        ));
    }
    let report = python(&before, &after);

    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition"
            && relation.predicate == Predicate::ShapeEqual
            && relation.correspondence == Correspondence::ModelForced
    }));
    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 65
            && group.after.len() == 65
            && group.before[0].kind == "function_definition"
            && group.reason.contains("64-child per-side cap")
    }));
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn ten_thousand_duplicate_siblings_abstain_without_a_quadratic_candidate_scan() {
    let source = "def repeated(value):\n    return value + 1\n\n".repeat(10_000);
    let report = python(&source, &source);

    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 10_000
            && group.after.len() == 10_000
            && group.before[0].kind == "function_definition"
            && group.reason.contains("16384-pair cap")
    }));
    verify_report(&report, source.as_bytes(), source.as_bytes()).unwrap();
}

#[test]
fn independent_components_are_aligned_past_the_total_region_limit() {
    let mut before = String::new();
    let mut after = String::new();
    for index in 0..65 {
        before.push_str(&format!(
            "def old_function_{index}(old_value):\n    unique_marker_{index}()\n    return old_value + 1\n\n"
        ));
        after.push_str(&format!(
            "def new_function_{index}(new_value):\n    unique_marker_{index}()\n    return new_value + 2\n\n"
        ));
    }
    let report = python(&before, &after);

    let functions: Vec<_> = report
        .relations
        .iter()
        .filter(|relation| {
            relation.before.kind == "function_definition"
                && relation.predicate == Predicate::ShapeEqual
                && relation.correspondence == Correspondence::ModelForced
        })
        .collect();
    assert_eq!(functions.len(), 65);
    assert!(
        functions
            .iter()
            .all(|relation| relation.before.shape_hash == functions[0].before.shape_hash)
    );
    assert!(
        !report
            .ambiguities
            .iter()
            .any(|group| group.reason.contains("64-child per-side cap"))
    );
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn incompatible_sparse_candidates_do_not_create_symbolic_ambiguity() {
    let mut before = String::new();
    let mut after = String::new();
    for index in 0..65 {
        let before_expression = format!("{}old_value", "not ".repeat(index + 1));
        let after_expression = format!("{}new_value", "not ".repeat(index + 1));
        before.push_str(&format!(
            "def old_function_{index}():\n    unique_marker_{index}()\n    return {before_expression}\n\n"
        ));
        after.push_str(&format!(
            "def new_function_{index}():\n    unique_marker_{}()\n    return {after_expression}\n\n",
            (index + 1) % 65
        ));
    }
    let report = python(&before, &after);

    assert!(!report.relations.iter().any(|relation| {
        relation.before.kind == "function_definition" && relation.predicate == Predicate::ShapeEqual
    }));
    assert!(!report.ambiguities.iter().any(|group| {
        group
            .before
            .iter()
            .any(|node| node.kind == "function_definition")
    }));
    assert_eq!(
        report
            .changes
            .iter()
            .filter(|change| {
                matches!(change.kind, ChangeKind::Insert | ChangeKind::Delete)
                    && change
                        .before
                        .as_ref()
                        .or(change.after.as_ref())
                        .is_some_and(|node| node.kind == "function_definition")
            })
            .count(),
        130
    );
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn small_component_is_not_hidden_by_an_unrelated_oversized_component() {
    let mut before = String::new();
    let mut after = String::new();
    for index in 0..65 {
        before.push_str(&format!(
            "def old_{index}(value_{index}):\n    return value_{index} + {index}\n\n"
        ));
        after.push_str(&format!(
            "def new_{index}(item_{index}):\n    return item_{index} + {}\n\n",
            index + 1_000
        ));
    }
    before.push_str("if old_condition:\n    old_tail()\n");
    after.push_str("if new_condition:\n    new_tail()\n");
    let report = python(&before, &after);

    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 65
            && group.after.len() == 65
            && group.before[0].kind == "function_definition"
            && group.reason.contains("64-child per-side cap")
    }));
    assert!(report.relations.iter().any(|relation| {
        relation.before.kind == "if_statement"
            && relation.predicate == Predicate::ShapeEqual
            && relation.correspondence == Correspondence::ModelForced
    }));
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn reordered_unique_children_are_reported_at_the_parent() {
    let before = "def first():\n    return 1\n\ndef second():\n    return 2\n";
    let after = "def second():\n    return 2\n\ndef first():\n    return 1\n";
    let report = python(before, after);

    assert!(
        report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::ChildOrderChanged)
    );
}

#[test]
fn trivia_only_edits_are_classified_without_losing_bytes() {
    let before = "answer=40+2\n";
    let after = "answer = 40 + 2\n";
    let report = python(before, after);

    assert!(
        report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::FormattingOnly)
    );
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn report_generation_is_deterministic() {
    let before = "def f(x):\n    return x + 1\n";
    let after = "def f(value):\n    return value + 2\n";
    assert_eq!(python(before, after), python(before, after));
}

#[test]
fn tampered_patch_fails_verification() {
    let before = "x = 1\n";
    let after = "x = 2\n";
    let mut report = python(before, after);
    report.patch.edits[0].replacement_base64 = "eA==".to_owned();

    assert!(verify_report(&report, before.as_bytes(), after.as_bytes()).is_err());
}

#[test]
fn tampered_structural_claim_fails_verification() {
    let before = "x = 1\n";
    let after = "x = 2\n";
    let mut report = python(before, after);
    report.summary.structural_changes += 1;

    assert!(verify_report(&report, before.as_bytes(), after.as_bytes()).is_err());
}

#[test]
fn tampered_ambiguity_fails_verification() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let mut report = python(source, source);
    report.ambiguities.clear();

    assert!(verify_report(&report, source.as_bytes(), source.as_bytes()).is_err());
}

#[test]
fn malformed_syntax_fails_clearly() {
    let result = analyze_bytes(
        b"def broken(:\n".to_vec(),
        b"def fixed():\n    pass\n".to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("refusing"));
}

#[test]
fn all_advertised_grammars_parse_and_replay() {
    let cases = [
        (Language::Javascript, "const x = 1;\n", "const x = 2;\n"),
        (
            Language::Typescript,
            "const x: number = 1;\n",
            "const x: number = 2;\n",
        ),
        (
            Language::Tsx,
            "const x = <p>a</p>;\n",
            "const x = <p>b</p>;\n",
        ),
        (
            Language::Rust,
            "fn main() { let x = 1; }\n",
            "fn main() { let x = 2; }\n",
        ),
        (
            Language::Java,
            "class Main { int value() { return 1; } }\n",
            "class Main { int value() { return 2; } }\n",
        ),
        (Language::Json, "{\"x\":1}\n", "{\"x\":2}\n"),
    ];
    for (language, before, after) in cases {
        let report = analyze_bytes(
            before.as_bytes().to_vec(),
            after.as_bytes().to_vec(),
            "before".to_owned(),
            "after".to_owned(),
            language,
        )
        .unwrap();
        verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
    }
}

#[test]
fn excessive_tree_depth_fails_instead_of_overflowing() {
    let deeply_nested = format!("{}0{}", "[".repeat(600), "]".repeat(600));
    let result = analyze_bytes(
        deeply_nested.as_bytes().to_vec(),
        b"[]".to_vec(),
        "before.json".to_owned(),
        "after.json".to_owned(),
        Language::Json,
    );
    assert!(result.is_err());
}
