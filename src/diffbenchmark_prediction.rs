use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::diffbenchmark::{
    JdtOracleNode, OffsetRange, comparable_tree_sitter_java_node_origins, jdt_node_role,
    resolve_jdt_node,
};
use crate::diffbenchmark_case::{AdaptedIntraFileCase, GoldComparableEndpoints};
use crate::diffbenchmark_eval::{
    AdapterUniverse, CasePredictions, NodeKey, NormalizedNode, NormalizedRelation,
    PredictionRelations, RelationUniverse,
};
use crate::{Correspondence, DiffReport, NodeRef};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnumeratedJdtNode {
    pub node_type: String,
    pub utf16_code_units: OffsetRange,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeCoverage {
    pub enumerated_nodes: usize,
    pub supported_nodes: usize,
    pub bridged_nodes: usize,
    pub unsupported_nodes: usize,
    pub unresolved_supported_nodes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PredictionAdapterDiagnostics {
    pub before_bridge: BridgeCoverage,
    pub after_bridge: BridgeCoverage,
    pub ignored_input_pair_relations: usize,
    pub ignored_suggested_relations: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdaptedPredictions {
    pub universe: AdapterUniverse,
    pub predictions: CasePredictions,
    pub diagnostics: PredictionAdapterDiagnostics,
}

pub struct PredictionAdapterInput<'a> {
    pub before_file: &'a str,
    pub after_file: &'a str,
    pub before_source: &'a [u8],
    pub after_source: &'a [u8],
    pub before_jdt_nodes: &'a [EnumeratedJdtNode],
    pub after_jdt_nodes: &'a [EnumeratedJdtNode],
    pub oracle: &'a AdaptedIntraFileCase,
    pub report: &'a DiffReport,
}

/// Project one StrataDiff report into DiffBenchmark's exact JDT node identity space.
pub fn adapt_predictions(input: &PredictionAdapterInput<'_>) -> Result<AdaptedPredictions> {
    let before = bridge_side(
        input.before_file,
        input.before_source,
        input.before_jdt_nodes,
    )
    .context("failed to bridge before JDT nodes")?;
    let after = bridge_side(input.after_file, input.after_source, input.after_jdt_nodes)
        .context("failed to bridge after JDT nodes")?;

    let program_universe = relation_universe(
        true,
        &before,
        &after,
        &input.oracle.gold_comparable_endpoints.program_elements,
    )?;
    let mapping_universe = relation_universe(
        false,
        &before,
        &after,
        &input.oracle.gold_comparable_endpoints.mappings,
    )?;
    let universe = AdapterUniverse {
        program_elements: program_universe,
        mappings: mapping_universe,
    };

    let mut program_forced = BTreeSet::new();
    let mut mapping_forced = BTreeSet::new();
    let mut ignored_input_pair_relations = 0;
    let mut ignored_suggested_relations = 0;
    for relation in &input.report.relations {
        match relation.correspondence {
            Correspondence::ModelForced => project_relation(
                &relation.before,
                &relation.after,
                &before,
                &after,
                &mut program_forced,
                &mut mapping_forced,
            )?,
            Correspondence::InputPair => ignored_input_pair_relations += 1,
            Correspondence::Suggested => ignored_suggested_relations += 1,
        }
    }

    let mut program_ambiguity = BTreeSet::new();
    let mut mapping_ambiguity = BTreeSet::new();
    for group in &input.report.ambiguities {
        project_ambiguity(
            &group.before,
            &group.after,
            &before,
            &after,
            &universe.program_elements,
            &universe.mappings,
            &mut program_ambiguity,
            &mut mapping_ambiguity,
        )?;
    }
    program_ambiguity.retain(|relation| !program_forced.contains(relation));
    mapping_ambiguity.retain(|relation| !mapping_forced.contains(relation));

    Ok(AdaptedPredictions {
        universe,
        predictions: CasePredictions {
            program_elements: PredictionRelations {
                forced: program_forced.into_iter().collect(),
                ambiguity_candidates: program_ambiguity.into_iter().collect(),
            },
            mappings: PredictionRelations {
                forced: mapping_forced.into_iter().collect(),
                ambiguity_candidates: mapping_ambiguity.into_iter().collect(),
            },
        },
        diagnostics: PredictionAdapterDiagnostics {
            before_bridge: before.coverage,
            after_bridge: after.coverage,
            ignored_input_pair_relations,
            ignored_suggested_relations,
        },
    })
}

struct BridgedSide {
    nodes: Vec<NormalizedNode>,
    by_origin: BTreeMap<usize, BridgedOrigin>,
    coverage: BridgeCoverage,
}

struct BridgedOrigin {
    kind: String,
    utf8_bytes: OffsetRange,
    nodes: Vec<NormalizedNode>,
}

fn bridge_side(file: &str, source: &[u8], jdt_nodes: &[EnumeratedJdtNode]) -> Result<BridgedSide> {
    let source_text = std::str::from_utf8(source).context("Java source is not UTF-8")?;
    let origins = comparable_tree_sitter_java_node_origins(source)?;
    let candidates: Vec<_> = origins.iter().map(|node| node.comparable).collect();
    let mut nodes = Vec::new();
    let mut by_origin: BTreeMap<usize, BridgedOrigin> = BTreeMap::new();
    let mut coverage = BridgeCoverage {
        enumerated_nodes: jdt_nodes.len(),
        ..BridgeCoverage::default()
    };

    for enumerated in jdt_nodes {
        if jdt_node_role(&enumerated.node_type).is_none() {
            coverage.unsupported_nodes += 1;
            continue;
        }
        coverage.supported_nodes += 1;
        let oracle_node = JdtOracleNode {
            node_type: enumerated.node_type.clone(),
            utf16_code_units: enumerated.utf16_code_units,
        };
        let Some(comparable) = resolve_jdt_node(&oracle_node, source_text, &candidates)? else {
            coverage.unresolved_supported_nodes += 1;
            continue;
        };
        let origin = origins
            .iter()
            .find(|origin| origin.comparable == comparable)
            .expect("a resolved comparable node has one tree-sitter origin");
        let normalized = NormalizedNode::from_comparable(
            NodeKey {
                file: file.to_owned(),
                jdt_kind: enumerated.node_type.clone(),
                utf16_code_units: enumerated.utf16_code_units,
            },
            comparable,
        );
        let entry = by_origin
            .entry(origin.origin_id)
            .or_insert_with(|| BridgedOrigin {
                kind: origin.origin_kind.clone(),
                utf8_bytes: origin.origin_utf8_bytes,
                nodes: Vec::new(),
            });
        ensure!(
            entry.kind == origin.origin_kind && entry.utf8_bytes == origin.origin_utf8_bytes,
            "one tree-sitter origin ID has conflicting metadata"
        );
        ensure!(
            !entry.nodes.iter().any(|node| node.key == normalized.key),
            "duplicate enumerated JDT node {:?}",
            normalized.key
        );
        entry.nodes.push(normalized.clone());
        nodes.push(normalized);
        coverage.bridged_nodes += 1;
    }
    ensure!(
        coverage.supported_nodes == coverage.bridged_nodes + coverage.unresolved_supported_nodes,
        "bridge coverage accounting mismatch"
    );
    Ok(BridgedSide {
        nodes,
        by_origin,
        coverage,
    })
}

fn relation_universe(
    program_elements: bool,
    before: &BridgedSide,
    after: &BridgedSide,
    gold: &GoldComparableEndpoints,
) -> Result<RelationUniverse> {
    let comparable_before: Vec<_> = before
        .nodes
        .iter()
        .filter(|node| is_program_element(&node.key.jdt_kind) == program_elements)
        .cloned()
        .collect();
    let comparable_after: Vec<_> = after
        .nodes
        .iter()
        .filter(|node| is_program_element(&node.key.jdt_kind) == program_elements)
        .cloned()
        .collect();
    require_gold_nodes("before", &gold.before, &comparable_before)?;
    require_gold_nodes("after", &gold.after, &comparable_after)?;
    Ok(RelationUniverse {
        comparable_before,
        comparable_after,
        gold_incident_before: gold.incident_before.clone(),
        gold_incident_after: gold.incident_after.clone(),
    })
}

fn require_gold_nodes(
    side: &str,
    gold: &[NormalizedNode],
    enumerated: &[NormalizedNode],
) -> Result<()> {
    let enumerated: BTreeSet<_> = enumerated.iter().collect();
    if let Some(node) = gold.iter().find(|node| !enumerated.contains(node)) {
        bail!(
            "scorable gold {side} node is absent from the independent JDT bridge: {:?}",
            node.key
        );
    }
    Ok(())
}

fn project_relation(
    before_ref: &NodeRef,
    after_ref: &NodeRef,
    before: &BridgedSide,
    after: &BridgedSide,
    program: &mut BTreeSet<NormalizedRelation>,
    mappings: &mut BTreeSet<NormalizedRelation>,
) -> Result<()> {
    let before_nodes = nodes_for_ref(before_ref, before)?;
    let after_nodes = nodes_for_ref(after_ref, after)?;
    for before_node in before_nodes {
        for after_node in after_nodes {
            if before_node.role != after_node.role {
                continue;
            }
            let relation = NormalizedRelation {
                before: before_node.key.clone(),
                after: after_node.key.clone(),
            };
            if is_program_element(&before_node.key.jdt_kind) {
                program.insert(relation);
            } else {
                mappings.insert(relation);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn project_ambiguity(
    before_refs: &[NodeRef],
    after_refs: &[NodeRef],
    before: &BridgedSide,
    after: &BridgedSide,
    program_universe: &RelationUniverse,
    mapping_universe: &RelationUniverse,
    program: &mut BTreeSet<NormalizedRelation>,
    mappings: &mut BTreeSet<NormalizedRelation>,
) -> Result<()> {
    let before_nodes = collect_nodes(before_refs, before)?;
    let after_nodes = collect_nodes(after_refs, after)?;
    for before_node in &before_nodes {
        let (universe, output) = if is_program_element(&before_node.key.jdt_kind) {
            (program_universe, &mut *program)
        } else {
            (mapping_universe, &mut *mappings)
        };
        let before_incident = universe.gold_incident_before.contains(&before_node.key);
        for after_node in &after_nodes {
            if before_node.role == after_node.role
                && (before_incident || universe.gold_incident_after.contains(&after_node.key))
            {
                output.insert(NormalizedRelation {
                    before: before_node.key.clone(),
                    after: after_node.key.clone(),
                });
            }
        }
    }
    Ok(())
}

fn collect_nodes<'a>(refs: &[NodeRef], side: &'a BridgedSide) -> Result<Vec<&'a NormalizedNode>> {
    let mut nodes = Vec::new();
    for node_ref in refs {
        nodes.extend(nodes_for_ref(node_ref, side)?);
    }
    Ok(nodes)
}

fn nodes_for_ref<'a>(node_ref: &NodeRef, side: &'a BridgedSide) -> Result<&'a [NormalizedNode]> {
    let Some(origin) = side.by_origin.get(&node_ref.id) else {
        return Ok(&[]);
    };
    let span = OffsetRange {
        start: node_ref.span.start_byte,
        end: node_ref.span.end_byte,
    };
    ensure!(
        origin.kind == node_ref.kind && origin.utf8_bytes == span,
        "DiffReport node {} disagrees with the prediction adapter parse",
        node_ref.id
    );
    Ok(&origin.nodes)
}

fn is_program_element(jdt_kind: &str) -> bool {
    matches!(
        jdt_kind,
        "TypeDeclaration"
            | "MethodDeclaration"
            | "FieldDeclaration"
            | "EnumDeclaration"
            | "RecordDeclaration"
            | "AnnotationTypeDeclaration"
            | "ImplicitTypeDeclaration"
    )
}
