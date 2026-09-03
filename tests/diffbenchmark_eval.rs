use stratadiff::diffbenchmark::{ComparableNode, OffsetRange, SharedNodeRole};
use stratadiff::diffbenchmark_eval::{
    AdapterUniverse, CaseEvaluationInput, CasePredictions, CategoryRawMultiGroups, EvaluationError,
    Multiplicity, NodeKey, NormalizedNode, NormalizedRelation, OracleRelation, OracleRelations,
    PredictionRelations, RawMultiGroup, RelationCategory, RelationSet, RelationUniverse,
    UniverseNodeSet, UniverseSide, evaluate_case,
};

fn node(
    file: &str,
    jdt_kind: &str,
    role: SharedNodeRole,
    utf16_start: usize,
    utf8_start: usize,
) -> NormalizedNode {
    NormalizedNode::from_comparable(
        NodeKey {
            file: file.to_owned(),
            jdt_kind: jdt_kind.to_owned(),
            utf16_code_units: OffsetRange {
                start: utf16_start,
                end: utf16_start + 1,
            },
        },
        ComparableNode {
            role,
            utf8_bytes: OffsetRange {
                start: utf8_start,
                end: utf8_start + 1,
            },
        },
    )
}

fn relation(before: &NormalizedNode, after: &NormalizedNode) -> NormalizedRelation {
    NormalizedRelation {
        before: before.key.clone(),
        after: after.key.clone(),
    }
}

fn oracle(relation: &NormalizedRelation, multiplicity: Multiplicity) -> OracleRelation {
    OracleRelation {
        relation: relation.clone(),
        multiplicity,
        raw_multi_group_id: None,
    }
}

fn multi_oracle(relation: &NormalizedRelation, group_id: usize) -> OracleRelation {
    OracleRelation {
        relation: relation.clone(),
        multiplicity: Multiplicity::Multi,
        raw_multi_group_id: Some(group_id),
    }
}

fn raw_multi_group(
    id: usize,
    before: &[&NormalizedNode],
    after: &[&NormalizedNode],
) -> RawMultiGroup {
    RawMultiGroup {
        id,
        before_endpoints: before.iter().map(|node| node.key.clone()).collect(),
        after_endpoints: after.iter().map(|node| node.key.clone()).collect(),
    }
}

fn empty_universe() -> RelationUniverse {
    RelationUniverse {
        comparable_before: Vec::new(),
        comparable_after: Vec::new(),
        gold_incident_before: Vec::new(),
        gold_incident_after: Vec::new(),
    }
}

fn empty_predictions() -> PredictionRelations {
    PredictionRelations {
        forced: Vec::new(),
        ambiguity_candidates: Vec::new(),
    }
}

fn program_case(
    universe: RelationUniverse,
    oracle: Vec<OracleRelation>,
    prediction: PredictionRelations,
) -> CaseEvaluationInput {
    program_case_with_multi_groups(universe, oracle, Vec::new(), prediction)
}

fn program_case_with_multi_groups(
    universe: RelationUniverse,
    oracle: Vec<OracleRelation>,
    raw_multi_groups: Vec<RawMultiGroup>,
    prediction: PredictionRelations,
) -> CaseEvaluationInput {
    CaseEvaluationInput {
        universe: AdapterUniverse {
            program_elements: universe,
            mappings: empty_universe(),
        },
        oracle: OracleRelations {
            program_elements: oracle,
            mappings: Vec::new(),
            raw_multi_groups: CategoryRawMultiGroups {
                program_elements: raw_multi_groups,
                mappings: Vec::new(),
            },
        },
        prediction: CasePredictions {
            program_elements: prediction,
            mappings: empty_predictions(),
        },
    }
}

#[test]
fn explicit_raw_multi_survives_an_excluded_partner_and_forced_hit_is_exact_tp() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let edge = relation(&before, &after);
    let group = raw_multi_group(0, &[&before], &[&after]);

    // The raw partner was excluded by the adapter, but multiplicity was already classified.
    let evaluation = evaluate_case(&program_case_with_multi_groups(
        RelationUniverse {
            comparable_before: vec![before.clone()],
            comparable_after: vec![after.clone()],
            gold_incident_before: vec![before.key],
            gold_incident_after: vec![after.key],
        },
        vec![multi_oracle(&edge, 0)],
        vec![group],
        PredictionRelations {
            forced: vec![edge],
            ambiguity_candidates: Vec::new(),
        },
    ))
    .unwrap();

    let score = evaluation.program_elements;
    assert_eq!(score.exact_relations.true_positives, 1);
    assert_eq!(score.exact_relations.false_positives, 0);
    assert_eq!(score.exact_relations.false_negatives, 0);
    assert_eq!(score.exact_relations.precision(), Some(1.0));
    assert_eq!(score.exact_relations.recall(), Some(1.0));
    assert_eq!(score.exact_relations.f1(), Some(1.0));
    assert_eq!(score.singleton_relations.oracle_relations, 0);
    assert_eq!(score.multi_relations.oracle_relations, 1);
    assert_eq!(score.multi_relations.forced_true_positives, 1);
    assert_eq!(score.multi_relations.forced_false_negatives, 0);
    assert_eq!(score.multi_relations.recall(), Some(1.0));
    assert_eq!(score.representation_warning.eligible_multi_groups, 1);
    assert_eq!(score.representation_warning.forced_touched_multi_groups, 1);
    assert_eq!(
        score
            .representation_warning
            .forced_gold_edges_in_multi_groups,
        1
    );
    assert_eq!(
        score
            .representation_warning
            .forced_false_positive_edges_incident_to_multi_groups,
        0
    );
    assert_eq!(
        score.representation_warning.multi_group_overclaim_rate(),
        Some(1.0)
    );
}

#[test]
fn multi_group_overclaim_counts_correct_and_incorrect_forced_edges() {
    let before_first = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let before_second = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let before_partner = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        4,
        4,
    );
    let after_first = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after_second = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let after_partner = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        4,
        4,
    );
    let first_gold = relation(&before_first, &after_first);
    let second_gold = relation(&before_second, &after_second);
    let cross_group_false_positive = relation(&before_first, &after_second);
    let first_group = raw_multi_group(0, &[&before_first, &before_partner], &[&after_first]);
    let second_group = raw_multi_group(4, &[&before_second], &[&after_second, &after_partner]);

    let evaluation = evaluate_case(&program_case_with_multi_groups(
        RelationUniverse {
            comparable_before: vec![before_first.clone(), before_second, before_partner],
            comparable_after: vec![after_first.clone(), after_second, after_partner],
            gold_incident_before: vec![first_gold.before.clone(), second_gold.before.clone()],
            gold_incident_after: vec![first_gold.after.clone(), second_gold.after.clone()],
        },
        vec![multi_oracle(&first_gold, 0), multi_oracle(&second_gold, 4)],
        vec![first_group, second_group],
        PredictionRelations {
            forced: vec![first_gold, cross_group_false_positive],
            ambiguity_candidates: Vec::new(),
        },
    ))
    .unwrap();

    let score = evaluation.program_elements;
    assert_eq!(score.multi_relations.oracle_relations, 2);
    assert_eq!(score.multi_relations.forced_true_positives, 1);
    assert_eq!(score.multi_relations.forced_false_negatives, 1);
    assert_eq!(score.representation_warning.eligible_multi_groups, 2);
    assert_eq!(score.representation_warning.forced_touched_multi_groups, 2);
    assert_eq!(
        score
            .representation_warning
            .forced_gold_edges_in_multi_groups,
        1
    );
    assert_eq!(
        score
            .representation_warning
            .forced_false_positive_edges_incident_to_multi_groups,
        1
    );
    assert_eq!(
        score.representation_warning.multi_group_overclaim_rate(),
        Some(1.0)
    );
}

#[test]
fn ambiguity_coverage_does_not_replace_an_exact_true_positive() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after_gold = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after_extra = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let gold = relation(&before, &after_gold);
    let extra = relation(&before, &after_extra);
    let group = raw_multi_group(0, &[&before], &[&after_gold]);

    let evaluation = evaluate_case(&program_case_with_multi_groups(
        RelationUniverse {
            comparable_before: vec![before.clone()],
            comparable_after: vec![after_gold.clone(), after_extra],
            gold_incident_before: vec![before.key],
            gold_incident_after: vec![after_gold.key],
        },
        vec![multi_oracle(&gold, 0)],
        vec![group],
        PredictionRelations {
            forced: Vec::new(),
            ambiguity_candidates: vec![gold, extra],
        },
    ))
    .unwrap();

    let score = evaluation.program_elements;
    assert_eq!(score.exact_relations.true_positives, 0);
    assert_eq!(score.exact_relations.false_positives, 0);
    assert_eq!(score.exact_relations.false_negatives, 1);
    assert_eq!(score.multi_relations.forced_false_negatives, 1);
    assert_eq!(score.ambiguity.oracle_multi_relations, 1);
    assert_eq!(score.ambiguity.predicted_candidates, 2);
    assert_eq!(score.ambiguity.covered_multi_relations, 1);
    assert_eq!(score.ambiguity.missed_multi_relations, 0);
    assert_eq!(score.ambiguity.extra_candidates, 1);
    assert_eq!(score.ambiguity.coverage(), Some(1.0));
    assert_eq!(score.ambiguity.expansion(), Some(2.0));
    assert_eq!(score.ambiguity_covered_oracle_relations, 1);
    assert_eq!(score.ambiguity_covered_gold_relation_rate(), Some(1.0));
}

#[test]
fn ambiguity_coverage_counts_gold_across_all_multiplicities() {
    let before_first = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let before_second = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let after_first = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after_second = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let covered = relation(&before_first, &after_first);
    let uncovered = relation(&before_second, &after_second);

    let evaluation = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before_first.clone(), before_second.clone()],
            comparable_after: vec![after_first.clone(), after_second.clone()],
            gold_incident_before: vec![before_first.key, before_second.key],
            gold_incident_after: vec![after_first.key, after_second.key],
        },
        vec![
            oracle(&covered, Multiplicity::Singleton),
            oracle(&uncovered, Multiplicity::Singleton),
        ],
        PredictionRelations {
            forced: Vec::new(),
            ambiguity_candidates: vec![covered],
        },
    ))
    .unwrap();

    let score = evaluation.program_elements;
    assert_eq!(score.exact_relations.false_negatives, 2);
    assert_eq!(score.ambiguity.oracle_multi_relations, 0);
    assert_eq!(score.ambiguity_covered_oracle_relations, 1);
    assert_eq!(score.ambiguity_covered_gold_relation_rate(), Some(0.5));
}

#[test]
fn incident_closure_scores_nearby_relations_and_excludes_unrelated_or_incompatible_ones() {
    let before_gold = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let before_nearby = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let after_gold = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after_nearby = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        2,
        2,
    );
    let after_incompatible = node(
        "after/A.java",
        "NumberLiteral",
        SharedNodeRole::NumberLiteral,
        4,
        4,
    );
    let gold = relation(&before_gold, &after_gold);
    let shares_before_incident = relation(&before_gold, &after_nearby);
    let shares_after_incident = relation(&before_nearby, &after_gold);
    let nonincident = relation(&before_nearby, &after_nearby);
    let incompatible = relation(&before_gold, &after_incompatible);

    let evaluation = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before_gold.clone(), before_nearby],
            comparable_after: vec![after_gold.clone(), after_nearby, after_incompatible],
            gold_incident_before: vec![before_gold.key],
            gold_incident_after: vec![after_gold.key],
        },
        vec![oracle(&gold, Multiplicity::Singleton)],
        PredictionRelations {
            forced: vec![
                shares_before_incident,
                shares_after_incident,
                nonincident,
                incompatible,
            ],
            ambiguity_candidates: Vec::new(),
        },
    ))
    .unwrap();

    let score = evaluation.program_elements;
    assert_eq!(score.exact_relations.true_positives, 0);
    assert_eq!(score.exact_relations.false_positives, 2);
    assert_eq!(score.exact_relations.false_negatives, 1);
    assert_eq!(score.unscored_predictions.forced, 2);
    assert_eq!(score.unscored_predictions.total(), 2);
}

#[test]
fn exact_jdt_kinds_keep_nodes_with_equal_roles_and_spans_distinct() {
    let before_constructor_invocation = node(
        "before/A.java",
        "ConstructorInvocation",
        SharedNodeRole::ExplicitConstructorInvocation,
        0,
        0,
    );
    let before_other_kind = node(
        "before/A.java",
        "SuperConstructorInvocation",
        SharedNodeRole::ExplicitConstructorInvocation,
        0,
        0,
    );
    let after = node(
        "after/A.java",
        "ConstructorInvocation",
        SharedNodeRole::ExplicitConstructorInvocation,
        0,
        0,
    );
    let gold = relation(&before_constructor_invocation, &after);
    let wrong_kind = relation(&before_other_kind, &after);

    let evaluation = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before_constructor_invocation.clone(), before_other_kind],
            comparable_after: vec![after.clone()],
            gold_incident_before: vec![before_constructor_invocation.key],
            gold_incident_after: vec![after.key],
        },
        vec![oracle(&gold, Multiplicity::Singleton)],
        PredictionRelations {
            forced: vec![wrong_kind],
            ambiguity_candidates: Vec::new(),
        },
    ))
    .unwrap();

    let score = evaluation.program_elements.exact_relations;
    assert_eq!(score.true_positives, 0);
    assert_eq!(score.false_positives, 1);
    assert_eq!(score.false_negatives, 1);
}

#[test]
fn category_containers_use_independent_relation_universes() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let edge = relation(&before, &after);
    let evaluation = evaluate_case(&CaseEvaluationInput {
        universe: AdapterUniverse {
            program_elements: RelationUniverse {
                comparable_before: vec![before.clone()],
                comparable_after: vec![after.clone()],
                gold_incident_before: vec![before.key],
                gold_incident_after: vec![after.key],
            },
            mappings: empty_universe(),
        },
        oracle: OracleRelations {
            program_elements: vec![oracle(&edge, Multiplicity::Singleton)],
            mappings: Vec::new(),
            raw_multi_groups: CategoryRawMultiGroups {
                program_elements: Vec::new(),
                mappings: Vec::new(),
            },
        },
        prediction: CasePredictions {
            program_elements: PredictionRelations {
                forced: vec![edge.clone()],
                ambiguity_candidates: Vec::new(),
            },
            mappings: PredictionRelations {
                forced: vec![edge],
                ambiguity_candidates: Vec::new(),
            },
        },
    })
    .unwrap();

    assert_eq!(
        evaluation.program_elements.exact_relations.true_positives,
        1
    );
    assert_eq!(evaluation.mappings.exact_relations.false_positives, 0);
    assert_eq!(evaluation.mappings.unscored_predictions.forced, 1);
}

#[test]
fn oracle_relation_outside_incident_closure_is_an_adapter_error() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let edge = relation(&before, &after);
    let error = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before],
            comparable_after: vec![after],
            gold_incident_before: Vec::new(),
            gold_incident_after: Vec::new(),
        },
        vec![oracle(&edge, Multiplicity::Singleton)],
        empty_predictions(),
    ))
    .unwrap_err();

    assert_eq!(
        error,
        EvaluationError::OracleRelationOutsideUniverse {
            category: RelationCategory::ProgramElements,
            relation_index: 0,
            relation: Box::new(edge),
        }
    );
}

#[test]
fn duplicate_nodes_relations_and_conflicting_predictions_are_rejected() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let after = node(
        "after/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let edge = relation(&before, &after);

    let duplicate_node = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before.clone(), before.clone()],
            comparable_after: vec![after.clone()],
            gold_incident_before: Vec::new(),
            gold_incident_after: Vec::new(),
        },
        Vec::new(),
        empty_predictions(),
    ))
    .unwrap_err();
    assert_eq!(
        duplicate_node,
        EvaluationError::DuplicateUniverseNode {
            category: RelationCategory::ProgramElements,
            side: UniverseSide::Before,
            node_set: UniverseNodeSet::Comparable,
            key: before.key.clone(),
        }
    );

    let universe = RelationUniverse {
        comparable_before: vec![before.clone()],
        comparable_after: vec![after.clone()],
        gold_incident_before: vec![before.key.clone()],
        gold_incident_after: vec![after.key.clone()],
    };
    let duplicate_oracle = evaluate_case(&program_case(
        universe.clone(),
        vec![
            oracle(&edge, Multiplicity::Singleton),
            oracle(&edge, Multiplicity::Multi),
        ],
        empty_predictions(),
    ))
    .unwrap_err();
    assert_eq!(
        duplicate_oracle,
        EvaluationError::DuplicateRelation {
            category: RelationCategory::ProgramElements,
            relation_set: RelationSet::Oracle,
            relation: Box::new(edge.clone()),
        }
    );

    let duplicate_forced = evaluate_case(&program_case(
        universe.clone(),
        Vec::new(),
        PredictionRelations {
            forced: vec![edge.clone(), edge.clone()],
            ambiguity_candidates: Vec::new(),
        },
    ))
    .unwrap_err();
    assert_eq!(
        duplicate_forced,
        EvaluationError::DuplicateRelation {
            category: RelationCategory::ProgramElements,
            relation_set: RelationSet::ForcedPrediction,
            relation: Box::new(edge.clone()),
        }
    );

    let conflicting = evaluate_case(&program_case(
        universe,
        Vec::new(),
        PredictionRelations {
            forced: vec![edge.clone()],
            ambiguity_candidates: vec![edge.clone()],
        },
    ))
    .unwrap_err();
    assert_eq!(
        conflicting,
        EvaluationError::ConflictingPrediction {
            category: RelationCategory::ProgramElements,
            relation: Box::new(edge),
        }
    );
}

#[test]
fn incident_nodes_must_be_unique_comparable_nodes() {
    let before = node(
        "before/A.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );
    let missing = node(
        "before/Missing.java",
        "SimpleName",
        SharedNodeRole::SimpleName,
        0,
        0,
    );

    let error = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before],
            comparable_after: Vec::new(),
            gold_incident_before: vec![missing.key.clone()],
            gold_incident_after: Vec::new(),
        },
        Vec::new(),
        empty_predictions(),
    ))
    .unwrap_err();
    assert_eq!(
        error,
        EvaluationError::GoldIncidentNodeNotComparable {
            category: RelationCategory::ProgramElements,
            side: UniverseSide::Before,
            key: missing.key,
        }
    );
}

#[test]
fn empty_case_has_explicitly_undefined_zero_denominators() {
    let evaluation = evaluate_case(&CaseEvaluationInput {
        universe: AdapterUniverse {
            program_elements: empty_universe(),
            mappings: empty_universe(),
        },
        oracle: OracleRelations {
            program_elements: Vec::new(),
            mappings: Vec::new(),
            raw_multi_groups: CategoryRawMultiGroups {
                program_elements: Vec::new(),
                mappings: Vec::new(),
            },
        },
        prediction: CasePredictions {
            program_elements: empty_predictions(),
            mappings: empty_predictions(),
        },
    })
    .unwrap();

    for score in [evaluation.program_elements, evaluation.mappings] {
        assert_eq!(score.exact_relations.precision(), None);
        assert_eq!(score.exact_relations.recall(), None);
        assert_eq!(score.exact_relations.f1(), None);
        assert_eq!(score.singleton_relations.recall(), None);
        assert_eq!(score.multi_relations.recall(), None);
        assert_eq!(score.ambiguity.coverage(), None);
        assert_eq!(score.ambiguity.expansion(), None);
        assert_eq!(score.ambiguity_covered_oracle_relations, 0);
        assert_eq!(score.ambiguity_covered_gold_relation_rate(), None);
        assert_eq!(
            score.representation_warning.multi_group_overclaim_rate(),
            None
        );
    }
}
