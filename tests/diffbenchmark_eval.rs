use stratadiff::diffbenchmark::{ComparableNode, OffsetRange, SharedNodeRole};
use stratadiff::diffbenchmark_eval::{
    AdapterUniverse, CaseEvaluationInput, CasePredictions, EvaluationError, Multiplicity, NodeKey,
    NormalizedNode, NormalizedRelation, OracleRelation, OracleRelations, PredictionRelations,
    RelationCategory, RelationSet, RelationUniverse, UniverseNodeSet, UniverseSide, evaluate_case,
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
    CaseEvaluationInput {
        universe: AdapterUniverse {
            program_elements: universe,
            mappings: empty_universe(),
        },
        oracle: OracleRelations {
            program_elements: oracle,
            mappings: Vec::new(),
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

    // The raw partner was excluded by the adapter, but multiplicity was already classified.
    let evaluation = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before.clone()],
            comparable_after: vec![after.clone()],
            gold_incident_before: vec![before.key],
            gold_incident_after: vec![after.key],
        },
        vec![oracle(&edge, Multiplicity::Multi)],
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
    assert_eq!(score.representation_warning.forced_on_multi, 1);
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

    let evaluation = evaluate_case(&program_case(
        RelationUniverse {
            comparable_before: vec![before.clone()],
            comparable_after: vec![after_gold.clone(), after_extra],
            gold_incident_before: vec![before.key],
            gold_incident_after: vec![after_gold.key],
        },
        vec![oracle(&gold, Multiplicity::Multi)],
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
    }
}
