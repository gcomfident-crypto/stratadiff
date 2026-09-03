use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::diffbenchmark::{
    GodMappingRecord, GodReport, JdtNodeResolver, JdtOracleMapping, JdtOracleNode, OffsetRange,
    SharedNodeRole, comparable_tree_sitter_java_nodes, jdt_node_role, parse_god_info,
};
use crate::diffbenchmark_eval::{
    CategoryRawMultiGroups, Multiplicity, NodeKey, NormalizedNode, NormalizedRelation,
    OracleRelation, OracleRelations, RawMultiGroup,
};

/// The side of an intra-file oracle relation being adapted.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointSide {
    Before,
    After,
}

/// A machine-readable reason why one endpoint could not enter the scoring oracle.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EndpointExclusionReason {
    UnsupportedJdtKind,
    InvalidUtf16Range {
        message: String,
    },
    ResolverError {
        message: String,
    },
    UnresolvedExactRoleAndSpan {
        role: SharedNodeRole,
        utf8_bytes: OffsetRange,
    },
}

/// One failed endpoint. Multiple endpoint failures still exclude their relation only once.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EndpointExclusion {
    pub side: EndpointSide,
    pub jdt_kind: String,
    pub utf16_code_units: OffsetRange,
    pub reason: EndpointExclusionReason,
}

/// A machine-readable reason why one raw oracle relation is not scorable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelationExclusionReason {
    InfoParseError {
        message: String,
    },
    EndpointFailures {
        failures: Vec<EndpointExclusion>,
    },
    RoleMismatch {
        before: SharedNodeRole,
        after: SharedNodeRole,
    },
}

/// Complete diagnostic provenance for one excluded raw relation.
///
/// `left_display` and `right_display` are copied for diagnostics only. Adaptation identity and all
/// scoring decisions use `info` exclusively.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExcludedRelation {
    pub raw_index: usize,
    pub raw_group_id: Option<usize>,
    pub info: String,
    pub left_display: String,
    pub right_display: String,
    pub reason: RelationExclusionReason,
}

/// Coverage for one original `GOD.json` relation container.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CategoryCoverageLedger {
    pub raw_relations: usize,
    pub scorable_relations: usize,
    pub excluded_relations: usize,
    pub exclusions: Vec<ExcludedRelation>,
}

/// Coverage remains isolated between `matchedElements` and `mappings`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageLedger {
    pub program_elements: CategoryCoverageLedger,
    pub mappings: CategoryCoverageLedger,
}

/// Comparable endpoints incident to scorable gold relations in one category.
///
/// This is deliberately not a prediction-comparison universe. A later prediction adapter must
/// expand it with its fixed, independently selected comparable endpoints before scoring.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoldComparableEndpoints {
    pub before: Vec<NormalizedNode>,
    pub after: Vec<NormalizedNode>,
    pub incident_before: Vec<NodeKey>,
    pub incident_after: Vec<NodeKey>,
}

/// Gold-only comparable endpoints remain isolated by DiffBenchmark relation category.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CategoryGoldComparableEndpoints {
    pub program_elements: GoldComparableEndpoints,
    pub mappings: GoldComparableEndpoints,
}

/// Stage-1 intra-file oracle data, without any StrataDiff prediction adaptation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdaptedIntraFileCase {
    pub oracle_relations: OracleRelations,
    pub gold_comparable_endpoints: CategoryGoldComparableEndpoints,
    pub coverage: CoverageLedger,
}

/// Adapt one strict `GOD.json` report and its exact before/after Java bytes.
///
/// Only `intraFileMappings` is consumed. Repository paths are retained verbatim in every
/// [`NodeKey`]. Both sources must be UTF-8 and parse without Tree-sitter errors.
pub fn adapt_intra_file_case(
    before_repository_path: &str,
    after_repository_path: &str,
    before_source: &[u8],
    after_source: &[u8],
    god: &GodReport,
) -> Result<AdaptedIntraFileCase> {
    let before_source =
        std::str::from_utf8(before_source).context("before Java source is not UTF-8")?;
    let after_source =
        std::str::from_utf8(after_source).context("after Java source is not UTF-8")?;
    let before_candidates = comparable_tree_sitter_java_nodes(before_source.as_bytes())
        .context("failed to parse before Java source")?;
    let after_candidates = comparable_tree_sitter_java_nodes(after_source.as_bytes())
        .context("failed to parse after Java source")?;
    let context = CaseContext {
        before_file: before_repository_path,
        after_file: after_repository_path,
        before_resolver: JdtNodeResolver::new(before_source, &before_candidates),
        after_resolver: JdtNodeResolver::new(after_source, &after_candidates),
    };

    let program_elements = adapt_category(
        "matchedElements",
        &god.intra_file_mappings.matched_elements,
        &context,
    )?;
    let mappings = adapt_category("mappings", &god.intra_file_mappings.mappings, &context)?;

    Ok(AdaptedIntraFileCase {
        oracle_relations: OracleRelations {
            program_elements: program_elements.relations,
            mappings: mappings.relations,
            raw_multi_groups: CategoryRawMultiGroups {
                program_elements: program_elements.raw_multi_groups,
                mappings: mappings.raw_multi_groups,
            },
        },
        gold_comparable_endpoints: CategoryGoldComparableEndpoints {
            program_elements: program_elements.endpoints,
            mappings: mappings.endpoints,
        },
        coverage: CoverageLedger {
            program_elements: program_elements.coverage,
            mappings: mappings.coverage,
        },
    })
}

struct AdaptedCategory {
    relations: Vec<OracleRelation>,
    raw_multi_groups: Vec<RawMultiGroup>,
    endpoints: GoldComparableEndpoints,
    coverage: CategoryCoverageLedger,
}

enum ParsedRecord {
    Mapping(JdtOracleMapping),
    InvalidInfo(String),
}

type EndpointIdentity = (String, usize, usize);

#[derive(Clone, Copy)]
struct RawGroupAssignment {
    id: usize,
    multiplicity: Multiplicity,
}

struct RawMultiGroupBuilder {
    id: usize,
    before_endpoints: BTreeSet<NodeKey>,
    after_endpoints: BTreeSet<NodeKey>,
}

struct CaseContext<'a> {
    before_file: &'a str,
    after_file: &'a str,
    before_resolver: JdtNodeResolver<'a>,
    after_resolver: JdtNodeResolver<'a>,
}

fn adapt_category(
    category: &str,
    records: &[GodMappingRecord],
    context: &CaseContext<'_>,
) -> Result<AdaptedCategory> {
    reject_duplicate_info(category, records)?;

    let parsed: Vec<_> = records
        .iter()
        .map(|record| match parse_god_info(&record.info) {
            Ok(mapping) => ParsedRecord::Mapping(mapping),
            Err(error) => ParsedRecord::InvalidInfo(format!("{error:#}")),
        })
        .collect();
    let group_assignments = raw_group_assignments(&parsed);
    let mut raw_multi_groups: BTreeMap<_, _> = group_assignments
        .iter()
        .flatten()
        .filter(|assignment| assignment.multiplicity == Multiplicity::Multi)
        .map(|assignment| {
            (
                assignment.id,
                RawMultiGroupBuilder {
                    id: assignment.id,
                    before_endpoints: BTreeSet::new(),
                    after_endpoints: BTreeSet::new(),
                },
            )
        })
        .collect();

    let mut relations = Vec::new();
    let mut before_nodes = BTreeMap::new();
    let mut after_nodes = BTreeMap::new();
    let mut exclusions = Vec::new();

    for (raw_index, (record, parsed_record)) in records.iter().zip(&parsed).enumerate() {
        let mapping = match parsed_record {
            ParsedRecord::Mapping(mapping) => mapping,
            ParsedRecord::InvalidInfo(message) => {
                exclusions.push(exclusion(
                    raw_index,
                    None,
                    record,
                    RelationExclusionReason::InfoParseError {
                        message: message.clone(),
                    },
                ));
                continue;
            }
        };

        let before = adapt_endpoint(
            EndpointSide::Before,
            context.before_file,
            &mapping.before,
            &context.before_resolver,
        );
        let after = adapt_endpoint(
            EndpointSide::After,
            context.after_file,
            &mapping.after,
            &context.after_resolver,
        );
        let group =
            group_assignments[raw_index].expect("every parsed raw relation has a group assignment");
        if group.multiplicity == Multiplicity::Multi {
            let raw_group = raw_multi_groups
                .get_mut(&group.id)
                .expect("every multi relation has a raw group");
            if let Ok(before) = &before {
                raw_group.before_endpoints.insert(before.key.clone());
            }
            if let Ok(after) = &after {
                raw_group.after_endpoints.insert(after.key.clone());
            }
        }
        let (before, after) = match (before, after) {
            (Ok(before), Ok(after)) => (before, after),
            (before, after) => {
                let mut failures = Vec::new();
                if let Err(failure) = before {
                    failures.push(failure);
                }
                if let Err(failure) = after {
                    failures.push(failure);
                }
                exclusions.push(exclusion(
                    raw_index,
                    Some(group.id),
                    record,
                    RelationExclusionReason::EndpointFailures { failures },
                ));
                continue;
            }
        };

        if before.role != after.role {
            exclusions.push(exclusion(
                raw_index,
                Some(group.id),
                record,
                RelationExclusionReason::RoleMismatch {
                    before: before.role,
                    after: after.role,
                },
            ));
            continue;
        }

        let relation = NormalizedRelation {
            before: before.key.clone(),
            after: after.key.clone(),
        };
        relations.push(OracleRelation {
            relation,
            multiplicity: group.multiplicity,
            raw_multi_group_id: (group.multiplicity == Multiplicity::Multi).then_some(group.id),
        });
        insert_gold_node(&mut before_nodes, before)?;
        insert_gold_node(&mut after_nodes, after)?;
    }

    let scorable_relations = relations.len();
    let excluded_relations = exclusions.len();
    if records.len() != scorable_relations + excluded_relations {
        bail!("internal coverage accounting mismatch for {category}");
    }

    let before: Vec<_> = before_nodes.into_values().collect();
    let after: Vec<_> = after_nodes.into_values().collect();
    let incident_before = before.iter().map(|node| node.key.clone()).collect();
    let incident_after = after.iter().map(|node| node.key.clone()).collect();
    Ok(AdaptedCategory {
        relations,
        raw_multi_groups: raw_multi_groups
            .into_values()
            .map(|group| RawMultiGroup {
                id: group.id,
                before_endpoints: group.before_endpoints.into_iter().collect(),
                after_endpoints: group.after_endpoints.into_iter().collect(),
            })
            .collect(),
        endpoints: GoldComparableEndpoints {
            before,
            after,
            incident_before,
            incident_after,
        },
        coverage: CategoryCoverageLedger {
            raw_relations: records.len(),
            scorable_relations,
            excluded_relations,
            exclusions,
        },
    })
}

fn reject_duplicate_info(category: &str, records: &[GodMappingRecord]) -> Result<()> {
    let mut first_indices = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        if let Some(first_index) = first_indices.get(record.info.as_str()) {
            bail!(
                "duplicate DiffBenchmark intra-file {category} info at indices {first_index} and {index}: {}",
                record.info
            );
        }
        first_indices.insert(record.info.as_str(), index);
    }
    Ok(())
}

fn raw_group_assignments(parsed: &[ParsedRecord]) -> Vec<Option<RawGroupAssignment>> {
    let mut parents: Vec<_> = (0..parsed.len()).collect();
    let mut before_owners = BTreeMap::new();
    let mut after_owners = BTreeMap::new();
    for (raw_index, parsed_record) in parsed.iter().enumerate() {
        let ParsedRecord::Mapping(mapping) = parsed_record else {
            continue;
        };
        union_with_endpoint_owner(
            &mut parents,
            &mut before_owners,
            endpoint_identity(&mapping.before),
            raw_index,
        );
        union_with_endpoint_owner(
            &mut parents,
            &mut after_owners,
            endpoint_identity(&mapping.after),
            raw_index,
        );
    }

    let mut component_sizes = BTreeMap::new();
    for (raw_index, parsed_record) in parsed.iter().enumerate() {
        if matches!(parsed_record, ParsedRecord::Mapping(_)) {
            let root = find_root(&mut parents, raw_index);
            *component_sizes.entry(root).or_insert(0) += 1;
        }
    }

    parsed
        .iter()
        .enumerate()
        .map(|(raw_index, parsed_record)| {
            matches!(parsed_record, ParsedRecord::Mapping(_)).then(|| {
                let id = find_root(&mut parents, raw_index);
                RawGroupAssignment {
                    id,
                    multiplicity: if component_sizes[&id] > 1 {
                        Multiplicity::Multi
                    } else {
                        Multiplicity::Singleton
                    },
                }
            })
        })
        .collect()
}

fn union_with_endpoint_owner(
    parents: &mut [usize],
    owners: &mut BTreeMap<EndpointIdentity, usize>,
    endpoint: EndpointIdentity,
    raw_index: usize,
) {
    if let Some(owner) = owners.get(&endpoint) {
        union_roots(parents, raw_index, *owner);
    } else {
        owners.insert(endpoint, raw_index);
    }
}

fn union_roots(parents: &mut [usize], left: usize, right: usize) {
    let left = find_root(parents, left);
    let right = find_root(parents, right);
    if left != right {
        parents[left.max(right)] = left.min(right);
    }
}

fn find_root(parents: &mut [usize], index: usize) -> usize {
    let mut root = index;
    while parents[root] != root {
        root = parents[root];
    }
    let mut current = index;
    while parents[current] != current {
        let next = parents[current];
        parents[current] = root;
        current = next;
    }
    root
}

fn endpoint_identity(node: &JdtOracleNode) -> EndpointIdentity {
    (
        node.node_type.clone(),
        node.utf16_code_units.start,
        node.utf16_code_units.end,
    )
}

fn adapt_endpoint(
    side: EndpointSide,
    file: &str,
    node: &JdtOracleNode,
    resolver: &JdtNodeResolver<'_>,
) -> std::result::Result<NormalizedNode, EndpointExclusion> {
    let role = jdt_node_role(&node.node_type).ok_or_else(|| {
        endpoint_exclusion(side, node, EndpointExclusionReason::UnsupportedJdtKind)
    })?;
    let utf8_bytes = match normalize_range(resolver, node.utf16_code_units) {
        Ok(range) => range,
        Err(error) => {
            return Err(endpoint_exclusion(
                side,
                node,
                EndpointExclusionReason::InvalidUtf16Range {
                    message: format!("{error:#}"),
                },
            ));
        }
    };
    let comparable = match resolver.resolve(node) {
        Ok(Some(comparable)) => comparable,
        Ok(None) => {
            return Err(endpoint_exclusion(
                side,
                node,
                EndpointExclusionReason::UnresolvedExactRoleAndSpan { role, utf8_bytes },
            ));
        }
        Err(error) => {
            return Err(endpoint_exclusion(
                side,
                node,
                EndpointExclusionReason::ResolverError {
                    message: format!("{error:#}"),
                },
            ));
        }
    };

    Ok(NormalizedNode::from_comparable(
        NodeKey {
            file: file.to_owned(),
            jdt_kind: node.node_type.clone(),
            utf16_code_units: node.utf16_code_units,
        },
        comparable,
    ))
}

fn normalize_range(resolver: &JdtNodeResolver<'_>, range: OffsetRange) -> Result<OffsetRange> {
    let start = resolver
        .utf16_offset_to_byte_offset(range.start)
        .context("invalid endpoint start offset")?;
    let end = resolver
        .utf16_offset_to_byte_offset(range.end)
        .context("invalid endpoint end offset")?;
    Ok(OffsetRange { start, end })
}

fn endpoint_exclusion(
    side: EndpointSide,
    node: &JdtOracleNode,
    reason: EndpointExclusionReason,
) -> EndpointExclusion {
    EndpointExclusion {
        side,
        jdt_kind: node.node_type.clone(),
        utf16_code_units: node.utf16_code_units,
        reason,
    }
}

fn exclusion(
    raw_index: usize,
    raw_group_id: Option<usize>,
    record: &GodMappingRecord,
    reason: RelationExclusionReason,
) -> ExcludedRelation {
    ExcludedRelation {
        raw_index,
        raw_group_id,
        info: record.info.clone(),
        left_display: record.left.clone(),
        right_display: record.right.clone(),
        reason,
    }
}

fn insert_gold_node(
    nodes: &mut BTreeMap<NodeKey, NormalizedNode>,
    node: NormalizedNode,
) -> Result<()> {
    if let Some(existing) = nodes.insert(node.key.clone(), node.clone())
        && existing != node
    {
        bail!("one exact JDT node key resolved to conflicting comparable nodes");
    }
    Ok(())
}
