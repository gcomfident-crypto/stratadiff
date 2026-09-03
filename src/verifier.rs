use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::model::{
    AmbiguityGroup, Artifact, ChangeKind, Correspondence, DiffReport, NodeRef, Predicate, Relation,
    StructuralChange, Summary,
};
use crate::patch::apply_patch;
use crate::syntax::{ParsedSyntax, SyntaxNode, parse, shape_equal, syntax_equal};

pub(crate) const REPORT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json";

const INPUT_PAIR_EVIDENCE: &[&str] = &["caller_supplied_file_pair"];
const GLOBAL_ANCHOR_EVIDENCE: &[&str] = &[
    "globally_unique_identical_syntax_subtree",
    "recursive_syntax_equality_check",
];
const LOCAL_ANCHOR_EVIDENCE: &[&str] = &[
    "unique_identical_child_under_mapped_parent",
    "recursive_syntax_equality_check",
];
const EXACT_DESCENDANT_EVIDENCE: &[&str] = &["isomorphic_path_under_exact_anchor"];
const STABLE_CORE_EVIDENCE: &[&str] = &[
    "bounded_ordered_child_alignment_v1",
    "pair_present_in_every_optimal_alignment",
    "recursive_shape_equality_check",
    "not_a_historical_identity_claim",
];
const REPEATED_AMBIGUITY_REASON: &str = "repeated shape-equivalent children are not treated as identities even when source order selects one optimal alignment";
const OPTIONAL_AMBIGUITY_REASON: &str =
    "the shape-equivalent pair is absent from at least one optimal ordered alignment";
const ORDERED_ALIGNMENT_COMPONENT_LIMIT: usize = 64;

type ExactKey = (Option<String>, String, [u8; 32]);
type ShapeKey = (Option<String>, String, [u8; 32]);

struct VerifiedMappings<'a> {
    before_to_after: Vec<Option<usize>>,
    after_to_before: Vec<Option<usize>>,
    by_before: Vec<Option<&'a Relation>>,
}

struct PhaseMappings {
    before_to_after: Vec<Option<usize>>,
    after_to_before: Vec<Option<usize>>,
}

struct GlobalExactIndex {
    before: HashMap<(String, [u8; 32]), Vec<usize>>,
    after: HashMap<(String, [u8; 32]), Vec<usize>>,
}

pub(crate) fn verify_report(report: &DiffReport, before: &[u8], after: &[u8]) -> Result<()> {
    verify_header(report)?;
    verify_artifact("before", &report.before, before)?;
    verify_artifact("after", &report.after, after)?;
    verify_patch_and_certificate(report, before, after)?;

    let parsed_before = parse(before.to_vec(), report.parser.language)?;
    let parsed_after = parse(after.to_vec(), report.parser.language)?;
    verify_parser_manifest(report, &parsed_before, &parsed_after)?;

    let mappings = verify_relations(report, &parsed_before, &parsed_after)?;
    let global_index = global_exact_index(&parsed_before, &parsed_after);
    verify_exact_anchor_evidence(
        report,
        &parsed_before,
        &parsed_after,
        &mappings,
        &global_index,
    )?;
    verify_global_anchor_completeness(&parsed_before, &parsed_after, &mappings, &global_index)?;
    verify_local_exact_relations(&parsed_before, &parsed_after, &mappings)?;
    let expected_ambiguities = verify_stable_core(&parsed_before, &parsed_after, &mappings)?;
    verify_ambiguity_node_references(report, &parsed_before, &parsed_after)?;
    if report.ambiguities != expected_ambiguities {
        bail!("ambiguity groups do not match the independently derived unmatched candidate sets");
    }

    let expected_changes = derive_changes(
        &parsed_before,
        &parsed_after,
        &mappings,
        &expected_ambiguities,
    );
    verify_change_node_references(report, &parsed_before, &parsed_after)?;
    if report.changes != expected_changes {
        bail!("structural changes do not match the independently derived events");
    }

    let expected_summary = Summary {
        model_forced_relations: report
            .relations
            .iter()
            .filter(|relation| relation.correspondence == Correspondence::ModelForced)
            .count(),
        suggested_relations: report
            .relations
            .iter()
            .filter(|relation| relation.correspondence == Correspondence::Suggested)
            .count(),
        ambiguity_groups: expected_ambiguities.len(),
        structural_changes: expected_changes.len(),
    };
    if report.summary != expected_summary {
        bail!("summary does not match the verified structural claims");
    }
    Ok(())
}

fn verify_header(report: &DiffReport) -> Result<()> {
    if report.schema != REPORT_SCHEMA {
        bail!("unsupported report schema {}", report.schema);
    }
    if report.engine_version != env!("CARGO_PKG_VERSION") {
        bail!(
            "report engine version {} is not supported by verifier {}",
            report.engine_version,
            env!("CARGO_PKG_VERSION")
        );
    }
    Ok(())
}

fn verify_artifact(side: &str, artifact: &Artifact, bytes: &[u8]) -> Result<()> {
    if artifact.byte_len != bytes.len() {
        bail!("{side} byte length does not match the report");
    }
    if artifact.blake3 != digest(bytes) {
        bail!("{side} BLAKE3 digest does not match the report");
    }
    Ok(())
}

fn verify_patch_and_certificate(report: &DiffReport, before: &[u8], after: &[u8]) -> Result<()> {
    if report.patch.algorithm != "patience-lines+bounded-myers-bytes-v1" {
        bail!("unsupported patch algorithm {}", report.patch.algorithm);
    }
    let mut previous_end = 0;
    for edit in &report.patch.edits {
        if edit.old_start < previous_end
            || edit.old_end < edit.old_start
            || edit.old_end > before.len()
        {
            bail!(
                "invalid or overlapping edit range {}..{}",
                edit.old_start,
                edit.old_end
            );
        }
        let replacement = STANDARD
            .decode(&edit.replacement_base64)
            .context("patch replacement is not valid base64")?;
        if STANDARD.encode(&replacement) != edit.replacement_base64 {
            bail!("patch replacement is not canonical RFC 4648 base64");
        }
        previous_end = edit.old_end;
    }

    if !report.certificate.patch_verified {
        bail!("report does not carry a successful replay certificate");
    }
    if report.certificate.before_len != before.len() || report.certificate.after_len != after.len()
    {
        bail!("certificate lengths do not match the supplied snapshots");
    }
    let before_hash = digest(before);
    let after_hash = digest(after);
    if report.certificate.before_blake3 != before_hash
        || report.certificate.after_blake3 != after_hash
        || report.certificate.before_blake3 != report.before.blake3
        || report.certificate.after_blake3 != report.after.blake3
    {
        bail!("certificate hashes do not match the supplied artifacts");
    }

    let reconstructed = apply_patch(before, &report.patch)?;
    if reconstructed != after {
        bail!("patch replay differs from the supplied after snapshot");
    }
    if report.certificate.reconstructed_blake3 != digest(&reconstructed)
        || report.certificate.reconstructed_blake3 != after_hash
    {
        bail!("reconstructed bytes do not match the certificate hashes");
    }
    Ok(())
}

fn verify_parser_manifest(
    report: &DiffReport,
    before: &ParsedSyntax,
    after: &ParsedSyntax,
) -> Result<()> {
    let language = report.parser.language;
    if report.parser.engine != "tree-sitter"
        || report.parser.runtime_version != "0.27.0"
        || report.parser.grammar_name != language.grammar_name()
        || report.parser.grammar_version != language.grammar_version()
        || report.parser.grammar_abi != language.parser_language().abi_version()
        || report.parser.node_types_blake3 != digest(language.node_types().as_bytes())
        || report.parser.coordinate_unit != "zero_based_row_utf8_byte_column"
        || report.parser.root_kind != before.root_kind
        || report.parser.root_kind != after.root_kind
        || report.parser.before_nodes != before.nodes.len()
        || report.parser.after_nodes != after.nodes.len()
        || !report.parser.error_free
    {
        bail!("parser manifest does not match independent fresh parses");
    }
    Ok(())
}

fn verify_relations<'a>(
    report: &'a DiffReport,
    before: &ParsedSyntax,
    after: &ParsedSyntax,
) -> Result<VerifiedMappings<'a>> {
    let mut mappings = VerifiedMappings {
        before_to_after: vec![None; before.nodes.len()],
        after_to_before: vec![None; after.nodes.len()],
        by_before: vec![None; before.nodes.len()],
    };
    let mut previous_before_id = None;
    let mut input_pairs = 0;

    for relation in &report.relations {
        verify_node_ref(before, &relation.before, "relation before endpoint")?;
        verify_node_ref(after, &relation.after, "relation after endpoint")?;
        if previous_before_id.is_some_and(|id| relation.before.id <= id) {
            bail!("relations are not in canonical before-node order");
        }
        previous_before_id = Some(relation.before.id);
        if mappings.before_to_after[relation.before.id].is_some()
            || mappings.after_to_before[relation.after.id].is_some()
        {
            bail!("relations violate one-to-one correspondence");
        }

        let predicate_holds = match relation.predicate {
            Predicate::InputPair => {
                relation.before.id == before.root && relation.after.id == after.root
            }
            Predicate::ByteEqual => {
                node_bytes(before, relation.before.id) == node_bytes(after, relation.after.id)
            }
            Predicate::SyntaxEqual => {
                syntax_equal(before, relation.before.id, after, relation.after.id)
            }
            Predicate::ShapeEqual => {
                shape_equal(before, relation.before.id, after, relation.after.id)
            }
        };
        if !predicate_holds {
            bail!(
                "certified predicate {:?} is false for relation {} -> {}",
                relation.predicate,
                relation.before.id,
                relation.after.id
            );
        }

        match relation.correspondence {
            Correspondence::InputPair => {
                input_pairs += 1;
                if relation.predicate != Predicate::InputPair
                    || relation.before.id != before.root
                    || relation.after.id != after.root
                    || !has_evidence(relation, INPUT_PAIR_EVIDENCE)
                {
                    bail!("input-pair relation has an invalid endpoint, predicate, or evidence");
                }
            }
            Correspondence::ModelForced => {
                if has_evidence(relation, STABLE_CORE_EVIDENCE) {
                    if relation.predicate != Predicate::ShapeEqual {
                        bail!("stable-core relation does not carry the shape predicate");
                    }
                } else {
                    let expected_predicate =
                        exact_predicate(before, relation.before.id, after, relation.after.id)
                            .context("exact model-forced relation has unequal syntax")?;
                    if relation.predicate != expected_predicate {
                        bail!("exact relation does not use its strongest predicate");
                    }
                    if !has_evidence(relation, GLOBAL_ANCHOR_EVIDENCE)
                        && !has_evidence(relation, LOCAL_ANCHOR_EVIDENCE)
                        && !has_evidence(relation, EXACT_DESCENDANT_EVIDENCE)
                    {
                        bail!("model-forced relation has unsupported evidence");
                    }
                }
            }
            Correspondence::Suggested => {
                bail!("current report schema has no producer rule for suggested relations");
            }
        }
        if relation.predicate == Predicate::InputPair
            && relation.correspondence != Correspondence::InputPair
        {
            bail!("input_pair predicate is reserved for the input-pair correspondence");
        }

        mappings.before_to_after[relation.before.id] = Some(relation.after.id);
        mappings.after_to_before[relation.after.id] = Some(relation.before.id);
        mappings.by_before[relation.before.id] = Some(relation);
    }

    if input_pairs != 1
        || mappings.before_to_after[before.root] != Some(after.root)
        || mappings.after_to_before[after.root] != Some(before.root)
    {
        bail!("report must contain exactly one root input-pair relation");
    }

    Ok(mappings)
}

fn verify_exact_anchor_evidence(
    report: &DiffReport,
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    global_index: &GlobalExactIndex,
) -> Result<()> {
    let mut certified_descendants = HashMap::new();
    for relation in &report.relations {
        if relation.correspondence != Correspondence::ModelForced {
            continue;
        }
        if has_evidence(relation, GLOBAL_ANCHOR_EVIDENCE) {
            verify_global_anchor(before, after, relation, global_index)?;
            certify_exact_subtree(
                before,
                after,
                mappings,
                relation.before.id,
                relation.after.id,
                &mut certified_descendants,
            )?;
        } else if has_evidence(relation, LOCAL_ANCHOR_EVIDENCE) {
            verify_mapped_parents(before, after, mappings, relation)?;
            certify_exact_subtree(
                before,
                after,
                mappings,
                relation.before.id,
                relation.after.id,
                &mut certified_descendants,
            )?;
        }
    }

    for relation in &report.relations {
        if has_evidence(relation, EXACT_DESCENDANT_EVIDENCE)
            && certified_descendants.get(&relation.before.id) != Some(&relation.after.id)
        {
            bail!(
                "relation {} -> {} is not below a certified exact anchor",
                relation.before.id,
                relation.after.id
            );
        }
    }
    Ok(())
}

fn verify_global_anchor(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    relation: &Relation,
    global_index: &GlobalExactIndex,
) -> Result<()> {
    let before_node = &before.nodes[relation.before.id];
    let after_node = &after.nodes[relation.after.id];
    if before_node.id == before.root
        || after_node.id == after.root
        || before_node.subtree_size < 3
        || after_node.subtree_size < 3
    {
        bail!("global exact anchor does not satisfy the minimum subtree rule");
    }
    let before_key = (before_node.kind.clone(), before_node.syntax_hash);
    let after_key = (after_node.kind.clone(), after_node.syntax_hash);
    let before_count = global_index.before[&before_key].len();
    let after_count = global_index.after[&after_key].len();
    if before_count != 1 || after_count != 1 {
        bail!("global exact anchor is not unique in both snapshots");
    }
    Ok(())
}

fn certify_exact_subtree(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    before_id: usize,
    after_id: usize,
    certified_descendants: &mut HashMap<usize, usize>,
) -> Result<()> {
    let before_node = &before.nodes[before_id];
    let after_node = &after.nodes[after_id];
    if before_node.children.len() != after_node.children.len() {
        bail!("exact anchor children are not isomorphic");
    }
    for (before_child, after_child) in before_node.children.iter().zip(&after_node.children) {
        if mappings.before_to_after[*before_child] != Some(*after_child) {
            bail!("exact anchor is missing an isomorphic descendant relation");
        }
        let relation = mappings.by_before[*before_child]
            .context("exact anchor descendant has no relation record")?;
        if relation.correspondence != Correspondence::ModelForced {
            bail!("exact anchor descendant is not model-forced");
        }
        if !has_evidence(relation, EXACT_DESCENDANT_EVIDENCE)
            && !has_evidence(relation, GLOBAL_ANCHOR_EVIDENCE)
            && !has_evidence(relation, LOCAL_ANCHOR_EVIDENCE)
        {
            bail!("exact anchor descendant has invalid evidence");
        }
        if certified_descendants
            .insert(*before_child, *after_child)
            .is_some_and(|existing| existing != *after_child)
        {
            bail!("exact anchor certificates disagree on a descendant mapping");
        }
        certify_exact_subtree(
            before,
            after,
            mappings,
            *before_child,
            *after_child,
            certified_descendants,
        )?;
    }
    Ok(())
}

fn verify_global_anchor_completeness(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    global_index: &GlobalExactIndex,
) -> Result<()> {
    for (key, before_ids) in &global_index.before {
        let Some(after_ids) = global_index.after.get(key) else {
            continue;
        };
        if before_ids.len() == 1
            && after_ids.len() == 1
            && syntax_equal(before, before_ids[0], after, after_ids[0])
            && (mappings.before_to_after[before_ids[0]] != Some(after_ids[0])
                || !is_exact_before(mappings, before_ids[0]))
        {
            bail!("globally unique exact anchor is absent from the relation set");
        }
    }
    Ok(())
}

fn global_exact_index(before: &ParsedSyntax, after: &ParsedSyntax) -> GlobalExactIndex {
    GlobalExactIndex {
        before: exact_subtree_buckets(before),
        after: exact_subtree_buckets(after),
    }
}

fn exact_subtree_buckets(syntax: &ParsedSyntax) -> HashMap<(String, [u8; 32]), Vec<usize>> {
    let mut buckets = HashMap::new();
    for node in syntax
        .nodes
        .iter()
        .filter(|node| node.id != syntax.root && node.subtree_size >= 3)
    {
        buckets
            .entry((node.kind.clone(), node.syntax_hash))
            .or_insert_with(Vec::new)
            .push(node.id);
    }
    buckets
}

fn verify_local_exact_relations(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
) -> Result<()> {
    let mut verified_local_pairs = HashSet::new();
    for (before_parent, after_parent) in mapped_pairs(mappings) {
        let before_children = local_exact_candidates(before, before_parent, mappings);
        let after_children = local_exact_candidates_after(after, after_parent, mappings);
        for (before_id, after_id) in
            unique_pairs(before, after, &before_children, &after_children, exact_key)
        {
            if syntax_equal(before, before_id, after, after_id) {
                if mappings.before_to_after[before_id] != Some(after_id)
                    || !is_exact_before(mappings, before_id)
                {
                    bail!("unique exact children under a mapped parent are not related");
                }
                verified_local_pairs.insert((before_id, after_id));
            }
        }
    }

    for relation in mappings
        .by_before
        .iter()
        .flatten()
        .filter(|relation| has_evidence(relation, LOCAL_ANCHOR_EVIDENCE))
    {
        verify_mapped_parents(before, after, mappings, relation)?;
        if !verified_local_pairs.contains(&(relation.before.id, relation.after.id)) {
            bail!("local exact anchor is not unique under its mapped parent");
        }
    }
    Ok(())
}

fn verify_stable_core(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
) -> Result<Vec<AmbiguityGroup>> {
    let mut expected_pairs = HashSet::new();
    let mut ambiguities = Vec::new();
    let global_mappings = global_phase_mappings(before, after, mappings)?;
    for (before_parent, after_parent) in mapped_pairs(mappings) {
        for (before_children, after_children) in
            ordered_alignment_regions(before, after, mappings, before_parent, after_parent)
        {
            let outcome = independent_stable_core(
                before,
                after,
                &global_mappings,
                &before_children,
                &after_children,
            )?;
            for (before_id, after_id) in outcome.forced {
                expected_pairs.insert((before_id, after_id));
                let relation = mappings.by_before[before_id]
                    .context("all-optima stable-core pair is missing from the relation set")?;
                if relation.after.id != after_id
                    || relation.correspondence != Correspondence::ModelForced
                    || relation.predicate != Predicate::ShapeEqual
                    || !has_evidence(relation, STABLE_CORE_EVIDENCE)
                {
                    bail!("all-optima stable-core pair has an invalid relation certificate");
                }
            }
            for ambiguity in outcome.ambiguities {
                ambiguities.push(AmbiguityGroup {
                    parent_before: before_parent,
                    parent_after: after_parent,
                    predicate: Predicate::ShapeEqual,
                    before: ambiguity
                        .before
                        .iter()
                        .map(|id| before.nodes[*id].as_ref())
                        .collect(),
                    after: ambiguity
                        .after
                        .iter()
                        .map(|id| after.nodes[*id].as_ref())
                        .collect(),
                    reason: ambiguity.reason,
                });
            }
        }
    }

    for relation in mappings
        .by_before
        .iter()
        .flatten()
        .filter(|relation| has_evidence(relation, STABLE_CORE_EVIDENCE))
    {
        if !expected_pairs.contains(&(relation.before.id, relation.after.id)) {
            bail!("stable-core relation is not present in every optimal ordered alignment");
        }
    }

    ambiguities.sort_by_key(|group| {
        (
            group.parent_before,
            group.parent_after,
            group.before.first().map_or(usize::MAX, |node| node.id),
            group.after.first().map_or(usize::MAX, |node| node.id),
        )
    });
    Ok(ambiguities)
}

fn ordered_alignment_regions(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    before_parent: usize,
    after_parent: usize,
) -> Vec<(Vec<usize>, Vec<usize>)> {
    let before_children = &before.nodes[before_parent].children;
    let after_children = &after.nodes[after_parent].children;
    let after_positions: HashMap<_, _> = after_children
        .iter()
        .enumerate()
        .map(|(position, id)| (*id, position))
        .collect();
    let exact_anchors: Vec<_> = before_children
        .iter()
        .enumerate()
        .filter_map(|(before_position, before_id)| {
            let relation = mappings.by_before[*before_id]?;
            if !is_exact_relation(relation) {
                return None;
            }
            let after_position = after_positions.get(&relation.after.id).copied()?;
            Some((before_position, after_position))
        })
        .collect();
    let barriers: Vec<_> = exact_anchors
        .iter()
        .copied()
        .filter(|(before_position, after_position)| {
            exact_anchors.iter().all(|(other_before, other_after)| {
                before_position == other_before
                    || (before_position < other_before) == (after_position < other_after)
            })
        })
        .collect();

    let mut regions = Vec::with_capacity(barriers.len() + 1);
    let mut before_start = 0;
    let mut after_start = 0;
    for (before_end, after_end) in barriers.into_iter().chain(std::iter::once((
        before_children.len(),
        after_children.len(),
    ))) {
        let before_region = before_children[before_start..before_end]
            .iter()
            .copied()
            .filter(|id| !is_exact_before(mappings, *id))
            .collect();
        let after_region = after_children[after_start..after_end]
            .iter()
            .copied()
            .filter(|id| !is_exact_after(mappings, *id))
            .collect();
        regions.push((before_region, after_region));
        before_start = before_end.saturating_add(1);
        after_start = after_end.saturating_add(1);
    }
    regions
}

fn global_phase_mappings(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
) -> Result<PhaseMappings> {
    let mut phase = PhaseMappings {
        before_to_after: vec![None; before.nodes.len()],
        after_to_before: vec![None; after.nodes.len()],
    };
    for relation in mappings
        .by_before
        .iter()
        .flatten()
        .filter(|relation| has_evidence(relation, GLOBAL_ANCHOR_EVIDENCE))
    {
        mark_phase_subtree(
            before,
            after,
            mappings,
            relation.before.id,
            relation.after.id,
            &mut phase,
        )?;
    }
    Ok(phase)
}

fn mark_phase_subtree(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    before_id: usize,
    after_id: usize,
    phase: &mut PhaseMappings,
) -> Result<()> {
    if mappings.before_to_after[before_id] != Some(after_id) {
        bail!("global-phase exact certificate is incomplete");
    }
    if phase.before_to_after[before_id].is_some_and(|existing| existing != after_id)
        || phase.after_to_before[after_id].is_some_and(|existing| existing != before_id)
    {
        bail!("global-phase exact certificates conflict");
    }
    phase.before_to_after[before_id] = Some(after_id);
    phase.after_to_before[after_id] = Some(before_id);
    for (before_child, after_child) in before.nodes[before_id]
        .children
        .iter()
        .zip(&after.nodes[after_id].children)
    {
        mark_phase_subtree(before, after, mappings, *before_child, *after_child, phase)?;
    }
    Ok(())
}

#[derive(Default)]
struct AlignmentOutcome {
    forced: Vec<(usize, usize)>,
    ambiguities: Vec<AlignmentAmbiguity>,
}

struct AlignmentAmbiguity {
    before: Vec<usize>,
    after: Vec<usize>,
    reason: String,
}

struct VerifiedShapeClass {
    before: Vec<usize>,
    after: Vec<usize>,
}

struct VerifiedShapeClasses {
    classes: Vec<VerifiedShapeClass>,
    before_class: Vec<Option<usize>>,
    after_class: Vec<Option<usize>>,
}

fn verified_shape_classes(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    before_ids: &[usize],
    after_ids: &[usize],
) -> VerifiedShapeClasses {
    let before_index = buckets(before, before_ids, shape_key);
    let after_index = buckets(after, after_ids, shape_key);
    let mut classes = Vec::new();
    for (raw_key, before_bucket) in before_index {
        let Some(after_bucket) = after_index.get(&raw_key) else {
            continue;
        };
        classes.extend(partition_index_bucket(
            &before_bucket,
            after_bucket,
            |id, representative| shape_equal(before, id, before, representative),
            |representative, id| shape_equal(before, representative, after, id),
        ));
    }

    let mut before_class = vec![None; before.nodes.len()];
    let mut after_class = vec![None; after.nodes.len()];
    for (class_id, class) in classes.iter().enumerate() {
        for id in &class.before {
            before_class[*id] = Some(class_id);
        }
        for id in &class.after {
            after_class[*id] = Some(class_id);
        }
    }
    VerifiedShapeClasses {
        classes,
        before_class,
        after_class,
    }
}

fn partition_index_bucket(
    before_ids: &[usize],
    after_ids: &[usize],
    same_before_shape: impl Fn(usize, usize) -> bool,
    cross_shape: impl Fn(usize, usize) -> bool,
) -> Vec<VerifiedShapeClass> {
    let mut classes: Vec<VerifiedShapeClass> = Vec::new();
    for id in before_ids {
        if let Some(class) = classes
            .iter_mut()
            .find(|class| same_before_shape(*id, class.before[0]))
        {
            class.before.push(*id);
        } else {
            classes.push(VerifiedShapeClass {
                before: vec![*id],
                after: Vec::new(),
            });
        }
    }
    for id in after_ids {
        if let Some(class) = classes
            .iter_mut()
            .find(|class| cross_shape(class.before[0], *id))
        {
            class.after.push(*id);
        }
    }
    classes.retain(|class| !class.after.is_empty());
    classes
}

fn independent_stable_core(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    global_mappings: &PhaseMappings,
    before_ids: &[usize],
    after_ids: &[usize],
) -> Result<AlignmentOutcome> {
    let partition = verified_shape_classes(before, after, before_ids, after_ids);
    if partition.classes.is_empty() {
        return Ok(AlignmentOutcome::default());
    }

    let active_before: Vec<_> = before_ids
        .iter()
        .copied()
        .filter(|id| partition.before_class[*id].is_some())
        .collect();
    let active_after: Vec<_> = after_ids
        .iter()
        .copied()
        .filter(|id| partition.after_class[*id].is_some())
        .collect();
    if active_before.len() > ORDERED_ALIGNMENT_COMPONENT_LIMIT
        || active_after.len() > ORDERED_ALIGNMENT_COMPONENT_LIMIT
    {
        return Ok(oversized_alignment(&partition.classes));
    }

    let mut candidates = vec![vec![false; active_after.len()]; active_before.len()];
    for (before_index, before_id) in active_before.iter().copied().enumerate() {
        for (after_index, after_id) in active_after.iter().copied().enumerate() {
            candidates[before_index][after_index] = partition.before_class[before_id]
                == partition.after_class[after_id]
                && shape_equal(before, before_id, after, after_id)
                && phase_mappings_are_compatible(
                    before,
                    after,
                    global_mappings,
                    before_id,
                    after_id,
                );
        }
    }

    let prefix = alignment_prefix_scores(&candidates, None);
    let suffix = alignment_suffix_scores(&candidates);
    let optimum = prefix[active_before.len()][active_after.len()];
    if optimum == 0 {
        return Ok(AlignmentOutcome::default());
    }

    let mut outcome = AlignmentOutcome::default();
    let mut ambiguous: BTreeMap<usize, (BTreeSet<usize>, BTreeSet<usize>)> = BTreeMap::new();
    for (before_index, _) in active_before.iter().enumerate() {
        for (after_index, _) in active_after.iter().enumerate() {
            if !candidates[before_index][after_index]
                || prefix[before_index][after_index] + 1 + suffix[before_index + 1][after_index + 1]
                    != optimum
            {
                continue;
            }
            let before_id = active_before[before_index];
            let after_id = active_after[after_index];
            let class_id = partition.before_class[before_id]
                .context("active before node is missing its verified shape class")?;
            let class = &partition.classes[class_id];
            let unique_signature = class.before.len() == 1 && class.after.len() == 1;
            if unique_signature
                && alignment_score(&candidates, Some((before_index, after_index))) < optimum
            {
                outcome.forced.push((before_id, after_id));
            } else {
                let endpoints = ambiguous.entry(class_id).or_default();
                if unique_signature {
                    endpoints.0.insert(before_id);
                    endpoints.1.insert(after_id);
                } else {
                    endpoints.0.extend(class.before.iter().copied());
                    endpoints.1.extend(class.after.iter().copied());
                }
            }
        }
    }
    outcome.forced.sort_unstable();
    for (class_id, (before_ids, after_ids)) in ambiguous {
        let class = &partition.classes[class_id];
        let repeated = class.before.len() > 1 || class.after.len() > 1;
        outcome.ambiguities.push(AlignmentAmbiguity {
            before: before_ids.into_iter().collect(),
            after: after_ids.into_iter().collect(),
            reason: if repeated {
                REPEATED_AMBIGUITY_REASON.to_owned()
            } else {
                OPTIONAL_AMBIGUITY_REASON.to_owned()
            },
        });
    }
    Ok(outcome)
}

fn oversized_alignment(classes: &[VerifiedShapeClass]) -> AlignmentOutcome {
    let ambiguities = classes
        .iter()
        .map(|class| AlignmentAmbiguity {
            before: class.before.clone(),
            after: class.after.clone(),
            reason: format!(
                "ordered alignment candidate region exceeds the {ORDERED_ALIGNMENT_COMPONENT_LIMIT}-child per-side cap; candidates are preserved symbolically"
            ),
        })
        .collect();
    AlignmentOutcome {
        forced: Vec::new(),
        ambiguities,
    }
}

fn phase_mappings_are_compatible(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &PhaseMappings,
    before_id: usize,
    after_id: usize,
) -> bool {
    if mappings.before_to_after[before_id].is_some_and(|mapped_after| mapped_after != after_id)
        || mappings.after_to_before[after_id]
            .is_some_and(|mapped_before| mapped_before != before_id)
    {
        return false;
    }
    let before_node = &before.nodes[before_id];
    let after_node = &after.nodes[after_id];
    before_node.children.len() == after_node.children.len()
        && before_node.children.iter().zip(&after_node.children).all(
            |(before_child, after_child)| {
                phase_mappings_are_compatible(before, after, mappings, *before_child, *after_child)
            },
        )
}

fn alignment_prefix_scores(
    candidates: &[Vec<bool>],
    forbidden: Option<(usize, usize)>,
) -> Vec<Vec<usize>> {
    let before_len = candidates.len();
    let after_len = candidates.first().map_or(0, Vec::len);
    let mut scores = vec![vec![0; after_len + 1]; before_len + 1];
    for before_index in 0..before_len {
        for after_index in 0..after_len {
            let mut best =
                scores[before_index][after_index + 1].max(scores[before_index + 1][after_index]);
            if candidates[before_index][after_index]
                && forbidden != Some((before_index, after_index))
            {
                best = best.max(scores[before_index][after_index] + 1);
            }
            scores[before_index + 1][after_index + 1] = best;
        }
    }
    scores
}

fn alignment_suffix_scores(candidates: &[Vec<bool>]) -> Vec<Vec<usize>> {
    let before_len = candidates.len();
    let after_len = candidates.first().map_or(0, Vec::len);
    let mut scores = vec![vec![0; after_len + 1]; before_len + 1];
    for before_index in (0..before_len).rev() {
        for after_index in (0..after_len).rev() {
            let mut best =
                scores[before_index + 1][after_index].max(scores[before_index][after_index + 1]);
            if candidates[before_index][after_index] {
                best = best.max(scores[before_index + 1][after_index + 1] + 1);
            }
            scores[before_index][after_index] = best;
        }
    }
    scores
}

fn alignment_score(candidates: &[Vec<bool>], forbidden: Option<(usize, usize)>) -> usize {
    let scores = alignment_prefix_scores(candidates, forbidden);
    scores[candidates.len()][candidates.first().map_or(0, Vec::len)]
}

fn derive_changes(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    ambiguities: &[AmbiguityGroup],
) -> Vec<StructuralChange> {
    let ambiguous_before: HashSet<_> = ambiguities
        .iter()
        .flat_map(|group| group.before.iter().map(|node| node.id))
        .collect();
    let ambiguous_after: HashSet<_> = ambiguities
        .iter()
        .flat_map(|group| group.after.iter().map(|node| node.id))
        .collect();
    let mut changes = Vec::new();

    if syntax_equal(before, before.root, after, after.root) && before.source != after.source {
        changes.push(StructuralChange {
            kind: ChangeKind::FormattingOnly,
            before: Some(before.nodes[before.root].as_ref()),
            after: Some(after.nodes[after.root].as_ref()),
            detail: "syntax is identical after discarding trivia; bytes differ".to_owned(),
        });
    }

    for (before_parent, after_parent) in mapped_pairs(mappings) {
        let after_positions: HashMap<_, _> = after.nodes[after_parent]
            .children
            .iter()
            .enumerate()
            .map(|(position, id)| (*id, position))
            .collect();
        let positions: Vec<_> = before.nodes[before_parent]
            .children
            .iter()
            .filter_map(|before_child| {
                let after_child = mappings.before_to_after[*before_child]?;
                if after.nodes[after_child].parent != Some(after_parent) {
                    return None;
                }
                after_positions.get(&after_child).copied()
            })
            .collect();
        if positions.windows(2).any(|pair| pair[0] > pair[1]) {
            changes.push(StructuralChange {
                kind: ChangeKind::ChildOrderChanged,
                before: Some(before.nodes[before_parent].as_ref()),
                after: Some(after.nodes[after_parent].as_ref()),
                detail: "the relative order of mapped direct children changed; no historical move identity is asserted"
                    .to_owned(),
            });
        }
    }

    for relation in mappings.by_before.iter().flatten().filter(|relation| {
        relation.before.id != before.root && relation.correspondence != Correspondence::InputPair
    }) {
        let before_id = relation.before.id;
        let after_id = relation.after.id;
        let before_parent = before.nodes[before_id].parent;
        let after_parent = after.nodes[after_id].parent;
        if relation.correspondence == Correspondence::ModelForced
            && matches!(
                relation.predicate,
                Predicate::ByteEqual | Predicate::SyntaxEqual
            )
            && before_parent.is_some()
            && after_parent.is_some()
            && mappings.before_to_after[before_parent.expect("checked")].is_some()
            && mappings.before_to_after[before_parent.expect("checked")] != after_parent
        {
            changes.push(StructuralChange {
                kind: ChangeKind::EquivalentRelocation,
                before: Some(before.nodes[before_id].as_ref()),
                after: Some(after.nodes[after_id].as_ref()),
                detail: "an exact syntax subtree occurs under a different mapped parent".to_owned(),
            });
        }
        if relation.predicate == Predicate::ShapeEqual
            && !syntax_equal(before, before_id, after, after_id)
            && !before_parent.is_some_and(|parent| {
                mappings.by_before[parent].is_some_and(|parent_relation| {
                    parent_relation.predicate == Predicate::ShapeEqual
                })
            })
        {
            match relation.correspondence {
                Correspondence::ModelForced => changes.push(StructuralChange {
                    kind: ChangeKind::ModelForcedUpdate,
                    before: Some(before.nodes[before_id].as_ref()),
                    after: Some(after.nodes[after_id].as_ref()),
                    detail: "the pair occurs in every optimal ordered alignment and has equal shape but different syntax; no historical identity is asserted"
                        .to_owned(),
                }),
                Correspondence::Suggested => changes.push(StructuralChange {
                    kind: ChangeKind::SuggestedUpdate,
                    before: Some(before.nodes[before_id].as_ref()),
                    after: Some(after.nodes[after_id].as_ref()),
                    detail: "local shape match; reported as a suggestion, not an identity fact"
                        .to_owned(),
                }),
                Correspondence::InputPair => {}
            }
        }
    }

    for node in before.nodes.iter().skip(1) {
        if mappings.before_to_after[node.id].is_none()
            && !ambiguous_before.contains(&node.id)
            && node
                .parent
                .is_some_and(|parent| mappings.before_to_after[parent].is_some())
        {
            changes.push(StructuralChange {
                kind: ChangeKind::Delete,
                before: Some(node.as_ref()),
                after: None,
                detail: "maximal unmatched subtree in the before snapshot".to_owned(),
            });
        }
    }
    for node in after.nodes.iter().skip(1) {
        if mappings.after_to_before[node.id].is_none()
            && !ambiguous_after.contains(&node.id)
            && node
                .parent
                .is_some_and(|parent| mappings.after_to_before[parent].is_some())
        {
            changes.push(StructuralChange {
                kind: ChangeKind::Insert,
                before: None,
                after: Some(node.as_ref()),
                detail: "maximal unmatched subtree in the after snapshot".to_owned(),
            });
        }
    }
    changes.sort_by_key(change_order);
    changes
}

fn verify_ambiguity_node_references(
    report: &DiffReport,
    before: &ParsedSyntax,
    after: &ParsedSyntax,
) -> Result<()> {
    for group in &report.ambiguities {
        before
            .nodes
            .get(group.parent_before)
            .context("ambiguity references an unknown before parent")?;
        after
            .nodes
            .get(group.parent_after)
            .context("ambiguity references an unknown after parent")?;
        for node in &group.before {
            verify_node_ref(before, node, "ambiguity before member")?;
        }
        for node in &group.after {
            verify_node_ref(after, node, "ambiguity after member")?;
        }
    }
    Ok(())
}

fn verify_change_node_references(
    report: &DiffReport,
    before: &ParsedSyntax,
    after: &ParsedSyntax,
) -> Result<()> {
    for change in &report.changes {
        if let Some(node) = &change.before {
            verify_node_ref(before, node, "change before endpoint")?;
        }
        if let Some(node) = &change.after {
            verify_node_ref(after, node, "change after endpoint")?;
        }
        let valid_sides = match change.kind {
            ChangeKind::Insert => change.before.is_none() && change.after.is_some(),
            ChangeKind::Delete => change.before.is_some() && change.after.is_none(),
            ChangeKind::EquivalentRelocation
            | ChangeKind::ChildOrderChanged
            | ChangeKind::ModelForcedUpdate
            | ChangeKind::SuggestedUpdate
            | ChangeKind::FormattingOnly => change.before.is_some() && change.after.is_some(),
        };
        if !valid_sides {
            bail!("structural change has invalid endpoints for its kind");
        }
    }
    Ok(())
}

fn verify_node_ref<'a>(
    syntax: &'a ParsedSyntax,
    reference: &NodeRef,
    context: &str,
) -> Result<&'a SyntaxNode> {
    let node = syntax
        .nodes
        .get(reference.id)
        .with_context(|| format!("{context} references unknown node {}", reference.id))?;
    if node.as_ref() != *reference {
        bail!("{context} metadata does not match a fresh parse");
    }
    Ok(node)
}

fn verify_mapped_parents(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    mappings: &VerifiedMappings<'_>,
    relation: &Relation,
) -> Result<()> {
    let before_parent = before.nodes[relation.before.id]
        .parent
        .context("local relation has no before parent")?;
    let after_parent = after.nodes[relation.after.id]
        .parent
        .context("local relation has no after parent")?;
    if mappings.before_to_after[before_parent] != Some(after_parent) {
        bail!("local relation endpoints do not share a mapped parent pair");
    }
    Ok(())
}

fn exact_predicate(
    before: &ParsedSyntax,
    before_id: usize,
    after: &ParsedSyntax,
    after_id: usize,
) -> Option<Predicate> {
    if !syntax_equal(before, before_id, after, after_id) {
        None
    } else if node_bytes(before, before_id) == node_bytes(after, after_id) {
        Some(Predicate::ByteEqual)
    } else {
        Some(Predicate::SyntaxEqual)
    }
}

fn local_exact_candidates(
    syntax: &ParsedSyntax,
    parent: usize,
    mappings: &VerifiedMappings<'_>,
) -> Vec<usize> {
    syntax.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|id| {
            mappings.by_before[*id].is_none_or(|relation| {
                !is_exact_relation(relation) || has_evidence(relation, LOCAL_ANCHOR_EVIDENCE)
            })
        })
        .collect()
}

fn local_exact_candidates_after(
    syntax: &ParsedSyntax,
    parent: usize,
    mappings: &VerifiedMappings<'_>,
) -> Vec<usize> {
    syntax.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|id| {
            mappings.after_to_before[*id].is_none_or(|before_id| {
                let relation = mappings.by_before[before_id].expect("reverse mapping has relation");
                !is_exact_relation(relation) || has_evidence(relation, LOCAL_ANCHOR_EVIDENCE)
            })
        })
        .collect()
}

fn is_exact_relation(relation: &Relation) -> bool {
    relation.correspondence == Correspondence::ModelForced
        && matches!(
            relation.predicate,
            Predicate::ByteEqual | Predicate::SyntaxEqual
        )
}

fn is_exact_before(mappings: &VerifiedMappings<'_>, id: usize) -> bool {
    mappings.by_before[id].is_some_and(is_exact_relation)
}

fn is_exact_after(mappings: &VerifiedMappings<'_>, id: usize) -> bool {
    mappings.after_to_before[id].is_some_and(|before_id| is_exact_before(mappings, before_id))
}

fn mapped_pairs<'a>(
    mappings: &'a VerifiedMappings<'_>,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    mappings
        .before_to_after
        .iter()
        .enumerate()
        .filter_map(|(before_id, after_id)| after_id.map(|after_id| (before_id, after_id)))
}

fn unique_pairs<K: Ord + Clone>(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    before_ids: &[usize],
    after_ids: &[usize],
    key: impl Fn(&SyntaxNode) -> K + Copy,
) -> Vec<(usize, usize)> {
    let before_buckets = buckets(before, before_ids, key);
    let after_buckets = buckets(after, after_ids, key);
    before_buckets
        .into_iter()
        .filter_map(|(key, before_bucket)| {
            let after_bucket = after_buckets.get(&key)?;
            (before_bucket.len() == 1 && after_bucket.len() == 1)
                .then_some((before_bucket[0], after_bucket[0]))
        })
        .collect()
}

fn buckets<K: Ord>(
    syntax: &ParsedSyntax,
    ids: &[usize],
    key: impl Fn(&SyntaxNode) -> K,
) -> BTreeMap<K, Vec<usize>> {
    let mut result: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for id in ids {
        result.entry(key(&syntax.nodes[*id])).or_default().push(*id);
    }
    result
}

fn exact_key(node: &SyntaxNode) -> ExactKey {
    (node.field.clone(), node.kind.clone(), node.syntax_hash)
}

fn shape_key(node: &SyntaxNode) -> ShapeKey {
    (node.field.clone(), node.kind.clone(), node.shape_hash)
}

fn node_bytes(syntax: &ParsedSyntax, id: usize) -> &[u8] {
    let span = &syntax.nodes[id].span;
    &syntax.source[span.start_byte..span.end_byte]
}

fn has_evidence(relation: &Relation, expected: &[&str]) -> bool {
    relation.evidence.len() == expected.len()
        && relation
            .evidence
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
}

fn change_order(change: &StructuralChange) -> (usize, usize, u8) {
    let rank = match change.kind {
        ChangeKind::Insert => 0,
        ChangeKind::Delete => 1,
        ChangeKind::EquivalentRelocation => 2,
        ChangeKind::ChildOrderChanged => 3,
        ChangeKind::ModelForcedUpdate => 4,
        ChangeKind::SuggestedUpdate => 5,
        ChangeKind::FormattingOnly => 6,
    };
    (
        change.before.as_ref().map_or(usize::MAX, |node| node.id),
        change.after.as_ref().map_or(usize::MAX, |node| node.id),
        rank,
    )
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::{oversized_alignment, partition_index_bucket};

    #[test]
    fn raw_shape_hash_collision_is_partitioned_by_recursive_equality() {
        let before_shape = |id| match id {
            0 | 2 => 'i',
            1 => 's',
            _ => unreachable!(),
        };
        let after_shape = |id| match id {
            10 => 's',
            11 | 13 => 'i',
            12 => 'x',
            _ => unreachable!(),
        };
        let classes = partition_index_bucket(
            &[0, 1, 2],
            &[10, 11, 12, 13],
            |id, representative| before_shape(id) == before_shape(representative),
            |representative, id| before_shape(representative) == after_shape(id),
        );

        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].before, [0, 2]);
        assert_eq!(classes[0].after, [11, 13]);
        assert_eq!(classes[1].before, [1]);
        assert_eq!(classes[1].after, [10]);

        let symbolic = oversized_alignment(&classes);
        assert_eq!(symbolic.ambiguities.len(), 2);
        assert_eq!(symbolic.ambiguities[0].before, [0, 2]);
        assert_eq!(symbolic.ambiguities[1].after, [10]);
    }
}
