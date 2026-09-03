use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::diffbenchmark::{ComparableNode, OffsetRange, SharedNodeRole};

/// The two DiffBenchmark relation categories, evaluated in separate containers.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelationCategory {
    ProgramElements,
    Mappings,
}

/// Exact identity of one DiffBenchmark endpoint.
///
/// The JDT kind and UTF-16 range remain authoritative even when multiple JDT kinds normalize to
/// the same shared role and UTF-8 span.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct NodeKey {
    pub file: String,
    pub jdt_kind: String,
    pub utf16_code_units: OffsetRange,
}

/// One adapter-normalized endpoint with exact identity and comparable parser metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct NormalizedNode {
    pub key: NodeKey,
    pub role: SharedNodeRole,
    pub utf8_bytes: OffsetRange,
}

impl NormalizedNode {
    pub fn from_comparable(key: NodeKey, node: ComparableNode) -> Self {
        Self {
            key,
            role: node.role,
            utf8_bytes: node.utf8_bytes,
        }
    }
}

/// A directed before-to-after relation. Its category is determined by its containing field.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct NormalizedRelation {
    pub before: NodeKey,
    pub after: NodeKey,
}

/// The compact relation universe for one category.
///
/// A relation is scorable exactly when both endpoints are comparable, their shared roles match,
/// and at least one endpoint is incident to a scorable gold relation in this category.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelationUniverse {
    pub comparable_before: Vec<NormalizedNode>,
    pub comparable_after: Vec<NormalizedNode>,
    pub gold_incident_before: Vec<NodeKey>,
    pub gold_incident_after: Vec<NodeKey>,
}

/// Category-specific universes supplied by the adapter before predictions are scored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterUniverse {
    pub program_elements: RelationUniverse,
    pub mappings: RelationUniverse,
}

/// Multiplicity computed by the adapter from the complete raw category `info` set.
///
/// This annotation must be assigned before taxonomy or parser exclusions. The evaluator consumes
/// it verbatim and never infers multiplicity from the surviving normalized oracle relations.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Multiplicity {
    Singleton,
    Multi,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OracleRelation {
    pub relation: NormalizedRelation,
    pub multiplicity: Multiplicity,
}

/// Oracle relations remain separated by their DiffBenchmark container category.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OracleRelations {
    pub program_elements: Vec<OracleRelation>,
    pub mappings: Vec<OracleRelation>,
}

/// Predictions distinguish asserted relations from explicitly uncertain candidate relations.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PredictionRelations {
    pub forced: Vec<NormalizedRelation>,
    pub ambiguity_candidates: Vec<NormalizedRelation>,
}

/// Predictions remain separated by their DiffBenchmark container category.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CasePredictions {
    pub program_elements: PredictionRelations,
    pub mappings: PredictionRelations,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaseEvaluationInput {
    pub universe: AdapterUniverse,
    pub oracle: OracleRelations,
    pub prediction: CasePredictions,
}

/// Exact forced-relation counts over every scorable gold relation, regardless of multiplicity.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExactRelationScore {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
}

impl ExactRelationScore {
    pub fn precision(self) -> Option<f64> {
        ratio(
            self.true_positives,
            self.true_positives + self.false_positives,
        )
    }

    pub fn recall(self) -> Option<f64> {
        ratio(
            self.true_positives,
            self.true_positives + self.false_negatives,
        )
    }

    pub fn f1(self) -> Option<f64> {
        let denominator = 2.0 * self.true_positives as f64
            + self.false_positives as f64
            + self.false_negatives as f64;
        if denominator == 0.0 {
            None
        } else {
            Some(2.0 * self.true_positives as f64 / denominator)
        }
    }
}

/// Forced-relation diagnostics for one adapter-provided multiplicity lane.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MultiplicityLaneScore {
    pub oracle_relations: usize,
    pub forced_true_positives: usize,
    pub forced_false_negatives: usize,
}

impl MultiplicityLaneScore {
    pub fn recall(self) -> Option<f64> {
        ratio(self.forced_true_positives, self.oracle_relations)
    }
}

/// Coverage and expansion of explicit ambiguity candidates over oracle multi relations.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AmbiguityScore {
    pub oracle_multi_relations: usize,
    pub predicted_candidates: usize,
    pub covered_multi_relations: usize,
    pub missed_multi_relations: usize,
    pub extra_candidates: usize,
}

impl AmbiguityScore {
    pub fn coverage(self) -> Option<f64> {
        ratio(self.covered_multi_relations, self.oracle_multi_relations)
    }

    pub fn expansion(self) -> Option<f64> {
        ratio(self.predicted_candidates, self.covered_multi_relations)
    }
}

/// Representation diagnostics that do not change exact scoring.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepresentationWarning {
    /// Gold multi relations emitted as forced relations. These are exact true positives.
    pub forced_on_multi: usize,
}

/// Predictions outside the fixed category universe are reported but not scored.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnscoredPredictionCounts {
    pub forced: usize,
    pub ambiguity_candidates: usize,
}

impl UnscoredPredictionCounts {
    pub fn total(self) -> usize {
        self.forced + self.ambiguity_candidates
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CategoryScore {
    pub exact_relations: ExactRelationScore,
    pub singleton_relations: MultiplicityLaneScore,
    pub multi_relations: MultiplicityLaneScore,
    pub ambiguity: AmbiguityScore,
    pub representation_warning: RepresentationWarning,
    pub unscored_predictions: UnscoredPredictionCounts,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaseEvaluation {
    pub program_elements: CategoryScore,
    pub mappings: CategoryScore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniverseSide {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniverseNodeSet {
    Comparable,
    GoldIncident,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationSet {
    Oracle,
    ForcedPrediction,
    AmbiguityPrediction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationError {
    DuplicateUniverseNode {
        category: RelationCategory,
        side: UniverseSide,
        node_set: UniverseNodeSet,
        key: NodeKey,
    },
    GoldIncidentNodeNotComparable {
        category: RelationCategory,
        side: UniverseSide,
        key: NodeKey,
    },
    DuplicateRelation {
        category: RelationCategory,
        relation_set: RelationSet,
        relation: Box<NormalizedRelation>,
    },
    ConflictingPrediction {
        category: RelationCategory,
        relation: Box<NormalizedRelation>,
    },
    OracleRelationOutsideUniverse {
        category: RelationCategory,
        relation_index: usize,
        relation: Box<NormalizedRelation>,
    },
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateUniverseNode {
                category,
                side,
                node_set,
                key,
            } => write!(
                formatter,
                "duplicate {category:?} {side:?} {node_set:?} universe node {key:?}"
            ),
            Self::GoldIncidentNodeNotComparable {
                category,
                side,
                key,
            } => write!(
                formatter,
                "{category:?} {side:?} gold-incident node is not comparable: {key:?}"
            ),
            Self::DuplicateRelation {
                category,
                relation_set,
                relation,
            } => write!(
                formatter,
                "duplicate {category:?} {relation_set:?} relation {relation:?}"
            ),
            Self::ConflictingPrediction { category, relation } => write!(
                formatter,
                "{category:?} relation is both forced and an ambiguity candidate: {relation:?}"
            ),
            Self::OracleRelationOutsideUniverse {
                category,
                relation_index,
                relation,
            } => write!(
                formatter,
                "{category:?} oracle relation {relation_index} is outside the relation universe: {relation:?}"
            ),
        }
    }
}

impl Error for EvaluationError {}

/// Score one case without parsing files, inferring multiplicity, or expanding its universes.
pub fn evaluate_case(input: &CaseEvaluationInput) -> Result<CaseEvaluation, EvaluationError> {
    Ok(CaseEvaluation {
        program_elements: evaluate_category(
            RelationCategory::ProgramElements,
            &input.universe.program_elements,
            &input.oracle.program_elements,
            &input.prediction.program_elements,
        )?,
        mappings: evaluate_category(
            RelationCategory::Mappings,
            &input.universe.mappings,
            &input.oracle.mappings,
            &input.prediction.mappings,
        )?,
    })
}

fn evaluate_category(
    category: RelationCategory,
    universe: &RelationUniverse,
    oracle: &[OracleRelation],
    prediction: &PredictionRelations,
) -> Result<CategoryScore, EvaluationError> {
    let universe = PreparedUniverse::new(category, universe)?;
    let oracle_relations = oracle;
    let oracle = unique_oracle_relations(category, oracle_relations)?;
    let forced = unique_relations(category, &prediction.forced, RelationSet::ForcedPrediction)?;
    let ambiguity_candidates = unique_relations(
        category,
        &prediction.ambiguity_candidates,
        RelationSet::AmbiguityPrediction,
    )?;

    if let Some(relation) = forced.intersection(&ambiguity_candidates).next() {
        return Err(EvaluationError::ConflictingPrediction {
            category,
            relation: Box::new(relation.clone()),
        });
    }

    for (relation_index, oracle_relation) in oracle_relations.iter().enumerate() {
        if !universe.contains(&oracle_relation.relation) {
            return Err(EvaluationError::OracleRelationOutsideUniverse {
                category,
                relation_index,
                relation: Box::new(oracle_relation.relation.clone()),
            });
        }
    }

    let (forced, forced_unscored) = partition_by_universe(forced, &universe);
    let (ambiguity_candidates, ambiguity_unscored) =
        partition_by_universe(ambiguity_candidates, &universe);
    let all_gold: BTreeSet<_> = oracle.keys().cloned().collect();
    let singleton_gold: BTreeSet<_> = oracle
        .iter()
        .filter_map(|(relation, multiplicity)| {
            (*multiplicity == Multiplicity::Singleton).then_some(relation.clone())
        })
        .collect();
    let multi_gold: BTreeSet<_> = oracle
        .iter()
        .filter_map(|(relation, multiplicity)| {
            (*multiplicity == Multiplicity::Multi).then_some(relation.clone())
        })
        .collect();

    let true_positives = forced.intersection(&all_gold).count();
    let false_positives = forced.difference(&all_gold).count();
    let false_negatives = all_gold.difference(&forced).count();
    let covered_multi_relations = ambiguity_candidates.intersection(&multi_gold).count();

    Ok(CategoryScore {
        exact_relations: ExactRelationScore {
            true_positives,
            false_positives,
            false_negatives,
        },
        singleton_relations: score_multiplicity_lane(&singleton_gold, &forced),
        multi_relations: score_multiplicity_lane(&multi_gold, &forced),
        ambiguity: AmbiguityScore {
            oracle_multi_relations: multi_gold.len(),
            predicted_candidates: ambiguity_candidates.len(),
            covered_multi_relations,
            missed_multi_relations: multi_gold.difference(&ambiguity_candidates).count(),
            extra_candidates: ambiguity_candidates.difference(&multi_gold).count(),
        },
        representation_warning: RepresentationWarning {
            forced_on_multi: forced.intersection(&multi_gold).count(),
        },
        unscored_predictions: UnscoredPredictionCounts {
            forced: forced_unscored.len(),
            ambiguity_candidates: ambiguity_unscored.len(),
        },
    })
}

fn score_multiplicity_lane(
    oracle: &BTreeSet<NormalizedRelation>,
    forced: &BTreeSet<NormalizedRelation>,
) -> MultiplicityLaneScore {
    MultiplicityLaneScore {
        oracle_relations: oracle.len(),
        forced_true_positives: forced.intersection(oracle).count(),
        forced_false_negatives: oracle.difference(forced).count(),
    }
}

struct PreparedUniverse {
    comparable_before: BTreeMap<NodeKey, SharedNodeRole>,
    comparable_after: BTreeMap<NodeKey, SharedNodeRole>,
    gold_incident_before: BTreeSet<NodeKey>,
    gold_incident_after: BTreeSet<NodeKey>,
}

impl PreparedUniverse {
    fn new(
        category: RelationCategory,
        universe: &RelationUniverse,
    ) -> Result<Self, EvaluationError> {
        let comparable_before =
            unique_comparable_nodes(category, UniverseSide::Before, &universe.comparable_before)?;
        let comparable_after =
            unique_comparable_nodes(category, UniverseSide::After, &universe.comparable_after)?;
        let gold_incident_before = unique_incident_nodes(
            category,
            UniverseSide::Before,
            &universe.gold_incident_before,
            &comparable_before,
        )?;
        let gold_incident_after = unique_incident_nodes(
            category,
            UniverseSide::After,
            &universe.gold_incident_after,
            &comparable_after,
        )?;

        Ok(Self {
            comparable_before,
            comparable_after,
            gold_incident_before,
            gold_incident_after,
        })
    }

    fn contains(&self, relation: &NormalizedRelation) -> bool {
        let Some(before_role) = self.comparable_before.get(&relation.before) else {
            return false;
        };
        let Some(after_role) = self.comparable_after.get(&relation.after) else {
            return false;
        };

        before_role == after_role
            && (self.gold_incident_before.contains(&relation.before)
                || self.gold_incident_after.contains(&relation.after))
    }
}

fn unique_comparable_nodes(
    category: RelationCategory,
    side: UniverseSide,
    nodes: &[NormalizedNode],
) -> Result<BTreeMap<NodeKey, SharedNodeRole>, EvaluationError> {
    let mut unique = BTreeMap::new();
    for node in nodes {
        if unique.insert(node.key.clone(), node.role).is_some() {
            return Err(EvaluationError::DuplicateUniverseNode {
                category,
                side,
                node_set: UniverseNodeSet::Comparable,
                key: node.key.clone(),
            });
        }
    }
    Ok(unique)
}

fn unique_incident_nodes(
    category: RelationCategory,
    side: UniverseSide,
    nodes: &[NodeKey],
    comparable: &BTreeMap<NodeKey, SharedNodeRole>,
) -> Result<BTreeSet<NodeKey>, EvaluationError> {
    let mut unique = BTreeSet::new();
    for node in nodes {
        if !unique.insert(node.clone()) {
            return Err(EvaluationError::DuplicateUniverseNode {
                category,
                side,
                node_set: UniverseNodeSet::GoldIncident,
                key: node.clone(),
            });
        }
        if !comparable.contains_key(node) {
            return Err(EvaluationError::GoldIncidentNodeNotComparable {
                category,
                side,
                key: node.clone(),
            });
        }
    }
    Ok(unique)
}

fn unique_oracle_relations(
    category: RelationCategory,
    relations: &[OracleRelation],
) -> Result<BTreeMap<NormalizedRelation, Multiplicity>, EvaluationError> {
    let mut unique = BTreeMap::new();
    for oracle_relation in relations {
        if unique
            .insert(
                oracle_relation.relation.clone(),
                oracle_relation.multiplicity,
            )
            .is_some()
        {
            return Err(EvaluationError::DuplicateRelation {
                category,
                relation_set: RelationSet::Oracle,
                relation: Box::new(oracle_relation.relation.clone()),
            });
        }
    }
    Ok(unique)
}

fn unique_relations(
    category: RelationCategory,
    relations: &[NormalizedRelation],
    relation_set: RelationSet,
) -> Result<BTreeSet<NormalizedRelation>, EvaluationError> {
    let mut unique = BTreeSet::new();
    for relation in relations {
        if !unique.insert(relation.clone()) {
            return Err(EvaluationError::DuplicateRelation {
                category,
                relation_set,
                relation: Box::new(relation.clone()),
            });
        }
    }
    Ok(unique)
}

fn partition_by_universe(
    relations: BTreeSet<NormalizedRelation>,
    universe: &PreparedUniverse,
) -> (BTreeSet<NormalizedRelation>, BTreeSet<NormalizedRelation>) {
    relations
        .into_iter()
        .partition(|relation| universe.contains(relation))
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}
