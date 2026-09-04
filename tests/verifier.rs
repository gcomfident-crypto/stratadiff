use stratadiff::{
    AmbiguityAbstentionCause, AmbiguityConstraint, ChangeKind, Correspondence, DiffReport,
    Language, Predicate, Relation, analyze_bytes, verify_report,
};

fn python(before: &str, after: &str) -> DiffReport {
    analyze_bytes(
        before.as_bytes().to_vec(),
        after.as_bytes().to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap()
}

fn assert_tampered<F>(report: &DiffReport, before: &str, after: &str, mutate: F)
where
    F: FnOnce(&mut DiffReport),
{
    let mut tampered = report.clone();
    mutate(&mut tampered);
    assert!(
        verify_report(&tampered, before.as_bytes(), after.as_bytes()).is_err(),
        "tampered report unexpectedly verified"
    );
}

fn corrupt_hash(hash: &mut String) {
    let replacement = if hash.starts_with('0') { "1" } else { "0" };
    hash.replace_range(0..1, replacement);
}

#[test]
fn verifier_source_does_not_call_the_producer_matcher() {
    let source = include_str!("../crates/stratadiff-verifier/src/verifier.rs");
    let producer_entry_point = ["match", "_trees"].concat();
    let matcher_path = ["matcher", "::"].concat();

    assert!(!source.contains(&producer_entry_point));
    assert!(!source.contains(&matcher_path));
}

#[test]
fn independently_verified_report_is_accepted() {
    let before = "def alpha(x):\n    return x + 1\n\ndef beta():\n    return 2\n";
    let after = "def beta():\n    return 2\n\ndef alpha(value):\n    return value + 1\n";
    let report = python(before, after);

    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn barrier_and_duplicate_symmetry_reports_verify() {
    let barrier_before = concat!(
        "def moved(value):\n    return value + 1\n\n",
        "def stable():\n    return 0\n",
    );
    let barrier_after = concat!(
        "def stable():\n    return 0\n\n",
        "def moved(item):\n    return item + 2\n",
    );
    let barrier_report = python(barrier_before, barrier_after);
    verify_report(
        &barrier_report,
        barrier_before.as_bytes(),
        barrier_after.as_bytes(),
    )
    .unwrap();

    let duplicate_before = concat!(
        "old_x_one()\n",
        "left_a = 1\n",
        "while left_b:\n    old_body_b()\n",
        "if left_c:\n    old_body_c()\n",
        "old_x_two()\n",
    );
    let duplicate_after = concat!(
        "right_a = 2\n",
        "while right_b:\n    new_body_b()\n",
        "if right_c:\n    new_body_c()\n",
        "new_x()\n",
    );
    let duplicate_report = python(duplicate_before, duplicate_after);
    verify_report(
        &duplicate_report,
        duplicate_before.as_bytes(),
        duplicate_after.as_bytes(),
    )
    .unwrap();
}

#[test]
fn producer_and_verifier_agree_across_small_permutation_matrix() {
    let before_parts = [
        "def stable():\n    return 0\n\n",
        "def old_add_one(value):\n    return value + 1\n\n",
        "def old_add_two(value):\n    return value + 2\n\n",
        "while old_flag:\n    old_call()\n\n",
    ];
    let after_parts = [
        "def stable():\n    return 0\n\n",
        "def new_add_one(item):\n    return item + 10\n\n",
        "def new_add_two(item):\n    return item + 20\n\n",
        "while new_flag:\n    new_call()\n\n",
    ];
    let permutations = permutations([0, 1, 2, 3]);
    for before_order in &permutations {
        let before: String = before_order
            .iter()
            .map(|index| before_parts[*index])
            .collect();
        for after_order in &permutations {
            let after: String = after_order
                .iter()
                .map(|index| after_parts[*index])
                .collect();
            let report = python(&before, &after);
            verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
        }
    }
}

fn permutations(mut values: [usize; 4]) -> Vec<[usize; 4]> {
    fn generate(values: &mut [usize; 4], start: usize, output: &mut Vec<[usize; 4]>) {
        if start == values.len() {
            output.push(*values);
            return;
        }
        for index in start..values.len() {
            values.swap(start, index);
            generate(values, start + 1, output);
            values.swap(start, index);
        }
    }

    let mut output = Vec::new();
    generate(&mut values, 0, &mut output);
    output
}

#[test]
fn header_and_artifact_tampering_is_rejected() {
    let before = "value = 1\n";
    let after = "value = 2\n";
    let report = python(before, after);

    assert_tampered(&report, before, after, |report| {
        report.schema.push_str(".tampered");
    });
    assert_tampered(&report, before, after, |report| {
        report.engine_version = "999.0.0".to_owned();
    });
    assert_tampered(&report, before, after, |report| {
        report.before.byte_len += 1;
    });
    assert_tampered(&report, before, after, |report| {
        corrupt_hash(&mut report.after.blake3);
    });
}

#[test]
fn every_replay_certificate_field_is_checked() {
    let before = "value = 1\n";
    let after = "value = 200\n";
    let report = python(before, after);

    assert_tampered(&report, before, after, |report| {
        corrupt_hash(&mut report.certificate.before_blake3);
    });
    assert_tampered(&report, before, after, |report| {
        corrupt_hash(&mut report.certificate.after_blake3);
    });
    assert_tampered(&report, before, after, |report| {
        corrupt_hash(&mut report.certificate.reconstructed_blake3);
    });
    assert_tampered(&report, before, after, |report| {
        report.certificate.before_len += 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.certificate.after_len += 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.certificate.patch_verified = false;
    });
}

#[test]
fn patch_algorithm_ranges_payload_and_replay_are_checked() {
    let before = "value = 1\n";
    let after = "value = 200\n";
    let report = python(before, after);
    assert!(!report.patch.edits.is_empty());

    assert_tampered(&report, before, after, |report| {
        report.patch.algorithm = "untrusted".to_owned();
    });
    assert_tampered(&report, before, after, |report| {
        report.patch.edits[0].old_end = before.len() + 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.patch.edits[0].replacement_base64 = "not base64".to_owned();
    });
    assert_tampered(&report, before, after, |report| {
        report.patch.edits[0].replacement_base64 = "eA==".to_owned();
    });
}

#[test]
fn every_parser_manifest_field_is_checked() {
    let before = "value = 1\n";
    let after = "value = 2\n";
    let report = python(before, after);

    assert_tampered(&report, before, after, |report| {
        report.parser.engine.push_str("-fork");
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.runtime_version = "0.0.0".to_owned();
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.language = Language::Json;
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.grammar_name.push_str("-fork");
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.grammar_version = "0.0.0".to_owned();
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.grammar_abi += 1;
    });
    assert_tampered(&report, before, after, |report| {
        corrupt_hash(&mut report.parser.node_types_blake3);
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.coordinate_unit.push_str("_characters");
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.root_kind.push_str("_other");
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.before_nodes += 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.after_nodes += 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.parser.error_free = false;
    });
}

#[test]
fn relation_metadata_predicates_cardinality_and_order_are_checked() {
    let before = "def greet(name):\n    return 'hi ' + name\n";
    let after = "def welcome(person):\n    return 'hi ' + person\n";
    let report = python(before, after);
    let stable = report
        .relations
        .iter()
        .position(|relation| {
            relation.correspondence == Correspondence::ModelForced
                && relation.predicate == Predicate::ShapeEqual
        })
        .unwrap();

    assert_tampered(&report, before, after, |report| {
        report.relations[stable].before.kind.push_str("_fake");
    });
    assert_tampered(&report, before, after, |report| {
        report.relations[stable].predicate = Predicate::ByteEqual;
    });
    assert_tampered(&report, before, after, |report| {
        report.relations[stable].correspondence = Correspondence::Suggested;
    });
    assert_tampered(&report, before, after, |report| {
        report.relations[stable].evidence[0].push_str("_fake");
    });
    assert_tampered(&report, before, after, |report| {
        report.relations.push(report.relations[0].clone());
    });
    assert_tampered(&report, before, after, |report| {
        report.relations.swap(0, 1);
    });
    assert_tampered(&report, before, after, |report| {
        report.relations.remove(stable);
    });
}

#[test]
fn exact_anchor_uniqueness_and_descendant_membership_are_checked() {
    let before = "def kept():\n    value = 1\n    return value\n\ndef changed():\n    return 2\n";
    let after = "def kept():\n    value = 1\n    return value\n\ndef changed():\n    return 3\n";
    let report = python(before, after);
    let descendant = report
        .relations
        .iter()
        .position(|relation| relation.evidence == ["isomorphic_path_under_exact_anchor".to_owned()])
        .unwrap();

    assert_tampered(&report, before, after, |report| {
        report.relations[descendant].evidence = vec![
            "globally_unique_identical_syntax_subtree".to_owned(),
            "recursive_syntax_equality_check".to_owned(),
        ];
    });
    assert_tampered(&report, before, after, |report| {
        report.relations.remove(descendant);
    });
}

#[test]
fn optional_optimal_pair_cannot_be_promoted_to_model_forced() {
    let before = concat!(
        "def add_old(value):\n    return value + 1\n\n",
        "def multiply_old(value):\n    return value * 2\n",
    );
    let after = concat!(
        "def multiply_new(item):\n    return item * 3\n\n",
        "def add_new(item):\n    return item + 4\n",
    );
    let report = python(before, after);
    let group = report
        .ambiguities
        .iter()
        .find(|group| {
            group.before[0].kind == "function_definition"
                && matches!(
                    group.constraint,
                    AmbiguityConstraint::ExactOrderedAlignment { .. }
                )
        })
        .unwrap();
    let AmbiguityConstraint::ExactOrderedAlignment { possible_pairs, .. } = &group.constraint
    else {
        unreachable!();
    };
    let candidate = &possible_pairs[0];
    let fabricated = Relation {
        before: group
            .before
            .iter()
            .find(|node| node.id == candidate.before_id)
            .unwrap()
            .clone(),
        after: group
            .after
            .iter()
            .find(|node| node.id == candidate.after_id)
            .unwrap()
            .clone(),
        predicate: Predicate::ShapeEqual,
        correspondence: Correspondence::ModelForced,
        evidence: vec![
            "bounded_ordered_child_alignment_v1".to_owned(),
            "pair_present_in_every_optimal_alignment".to_owned(),
            "recursive_shape_equality_check".to_owned(),
            "not_a_historical_identity_claim".to_owned(),
        ],
    };

    assert_tampered(&report, before, after, |report| {
        let index = report
            .relations
            .partition_point(|relation| relation.before.id < fabricated.before.id);
        report.relations.insert(index, fabricated);
    });
}

#[test]
fn oversized_alignment_abstention_is_recomputed() {
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
    let oversized = report
        .ambiguities
        .iter()
        .position(|group| group.reason.contains("64-child per-side cap"))
        .unwrap();

    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
    assert_tampered(&report, &before, &after, |report| {
        report.ambiguities[oversized].before.pop();
    });
}

#[test]
fn symbolic_ambiguity_parents_memberships_cause_and_reason_are_checked() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = python(source, source);
    assert!(!report.ambiguities.is_empty());

    assert_tampered(&report, source, source, |report| {
        report.ambiguities[0].parent_before += 1;
    });
    assert_tampered(&report, source, source, |report| {
        let AmbiguityConstraint::SymbolicAbstention { cause, .. } =
            &mut report.ambiguities[0].constraint
        else {
            unreachable!();
        };
        *cause = AmbiguityAbstentionCause::ComponentLimit;
    });
    assert_tampered(&report, source, source, |report| {
        report.ambiguities[0].reason.push_str(" (guessed)");
    });
    assert_tampered(&report, source, source, |report| {
        report.ambiguities[0].before[0].span.end_byte += 1;
    });
    assert_tampered(&report, source, source, |report| {
        report.ambiguities[0].after.pop();
    });
    assert_tampered(&report, source, source, |report| {
        report.ambiguities.clear();
    });
}

#[test]
fn exact_ambiguity_constraint_tampering_is_rejected() {
    let before = concat!(
        "def add_old(value):\n    return value + 1\n\n",
        "def multiply_old(value):\n    return value * 2\n",
    );
    let after = concat!(
        "def multiply_new(item):\n    return item * 3\n\n",
        "def add_new(item):\n    return item + 4\n",
    );
    let report = python(before, after);
    let exact = report
        .ambiguities
        .iter()
        .position(|group| {
            matches!(
                group.constraint,
                AmbiguityConstraint::ExactOrderedAlignment { .. }
            )
        })
        .unwrap();

    assert_tampered(&report, before, after, |report| {
        let AmbiguityConstraint::ExactOrderedAlignment {
            required_matches, ..
        } = &mut report.ambiguities[exact].constraint
        else {
            unreachable!();
        };
        *required_matches += 1;
    });
    assert_tampered(&report, before, after, |report| {
        let AmbiguityConstraint::ExactOrderedAlignment { possible_pairs, .. } =
            &mut report.ambiguities[exact].constraint
        else {
            unreachable!();
        };
        possible_pairs.pop();
    });
    assert_tampered(&report, before, after, |report| {
        let AmbiguityConstraint::ExactOrderedAlignment { possible_pairs, .. } =
            &mut report.ambiguities[exact].constraint
        else {
            unreachable!();
        };
        possible_pairs.swap(0, 1);
    });
    assert_tampered(&report, before, after, |report| {
        let AmbiguityConstraint::ExactOrderedAlignment { predicate, .. } =
            &mut report.ambiguities[exact].constraint
        else {
            unreachable!();
        };
        *predicate = Predicate::SyntaxEqual;
    });
    assert_tampered(&report, before, after, |report| {
        let AmbiguityConstraint::ExactOrderedAlignment { possible_pairs, .. } =
            &mut report.ambiguities[exact].constraint
        else {
            unreachable!();
        };
        possible_pairs[0].before_id = usize::MAX;
    });
}

#[test]
fn derived_change_endpoints_kind_detail_and_completeness_are_checked() {
    let before = "value = 1\n";
    let after = "value = 1\nother = 2\n";
    let report = python(before, after);
    let insertion = report
        .changes
        .iter()
        .position(|change| change.kind == ChangeKind::Insert)
        .unwrap();

    assert_tampered(&report, before, after, |report| {
        report.changes[insertion].kind = ChangeKind::Delete;
    });
    assert_tampered(&report, before, after, |report| {
        report.changes[insertion].detail.push_str(" (unverified)");
    });
    assert_tampered(&report, before, after, |report| {
        report.changes[insertion].after.as_mut().unwrap().id += 1;
    });
    assert_tampered(&report, before, after, |report| {
        report.changes[insertion].after = None;
    });
    assert_tampered(&report, before, after, |report| {
        report.changes.remove(insertion);
    });
}

#[test]
fn model_forced_update_is_independently_derived() {
    let before = "def greet(name):\n    return 'hi ' + name\n";
    let after = "def welcome(person):\n    return 'hi ' + person\n";
    let report = python(before, after);
    let update = report
        .changes
        .iter()
        .position(|change| change.kind == ChangeKind::ModelForcedUpdate)
        .unwrap();

    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
    assert_tampered(&report, before, after, |report| {
        report.changes[update]
            .detail
            .push_str(" (historical identity)");
    });
}

#[test]
fn every_summary_counter_is_derived_instead_of_trusted() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = python(source, source);

    assert_tampered(&report, source, source, |report| {
        report.summary.model_forced_relations += 1;
    });
    assert_tampered(&report, source, source, |report| {
        report.summary.suggested_relations += 1;
    });
    assert_tampered(&report, source, source, |report| {
        report.summary.ambiguity_groups += 1;
    });
    assert_tampered(&report, source, source, |report| {
        report.summary.structural_changes += 1;
    });
}
