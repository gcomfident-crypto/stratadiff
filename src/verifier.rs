use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::model::{
    AmbiguityAbstentionCause, AmbiguityConstraint, AmbiguityGroup, AmbiguityPair, Artifact,
    ChangeKind, Correspondence, DiffReport, NodeRef, PairClaims, Predicate, Relation,
    StructuralChange, Summary,
};
use crate::patch::apply_patch;
use crate::syntax::{ParsedSyntax, SyntaxNode, parse, shape_equal, syntax_equal};

pub(crate) const REPORT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json";

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
const REPEATED_AMBIGUITY_REASON: &str = "repeated shape-equivalent children are intentionally unresolved; endpoint sets make no pair claims";
const OPTIONAL_AMBIGUITY_REASON: &str =
    "multiple maximum-cardinality ordered alignments have coupled candidate choices";
const ORDERED_ALIGNMENT_COMPONENT_LIMIT: usize = 64;
const ORDERED_ALIGNMENT_CANDIDATE_SCAN_LIMIT: usize =
    ORDERED_ALIGNMENT_COMPONENT_LIMIT * ORDERED_ALIGNMENT_COMPONENT_LIMIT * 4;

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
        bail!("ambiguity constraints do not match the independently derived choice spaces");
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
                    constraint: ambiguity.constraint,
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
    constraint: AmbiguityConstraint,
    reason: String,
}

struct AlignmentComponent {
    before: Vec<usize>,
    after: Vec<usize>,
    groups: Vec<usize>,
}

struct VerifiedShapeClass {
    before: Vec<usize>,
    after: Vec<usize>,
}

struct VerifiedShapeClasses {
    classes: Vec<VerifiedShapeClass>,
}

struct CandidateGroup {
    before: Vec<usize>,
    after: Vec<usize>,
    scan_complete: bool,
}

struct CandidateGroups {
    groups: Vec<CandidateGroup>,
    before_group: HashMap<usize, usize>,
    after_group: HashMap<usize, usize>,
}

struct DisjointSets {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSets {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            rank: vec![0; len],
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parent[value] != value {
            self.parent[value] = self.find(self.parent[value]);
        }
        self.parent[value]
    }

    fn union(&mut self, left: usize, right: usize) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root == right_root {
            return;
        }
        match self.rank[left_root].cmp(&self.rank[right_root]) {
            std::cmp::Ordering::Less => self.parent[left_root] = right_root,
            std::cmp::Ordering::Greater => self.parent[right_root] = left_root,
            std::cmp::Ordering::Equal => {
                self.parent[right_root] = left_root;
                self.rank[left_root] += 1;
            }
        }
    }
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

    VerifiedShapeClasses { classes }
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

fn verified_candidate_groups(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    global_mappings: &PhaseMappings,
    partition: &VerifiedShapeClasses,
) -> CandidateGroups {
    let mut groups = Vec::new();
    for shape_class in &partition.classes {
        let within_scan_limit = shape_class
            .before
            .len()
            .checked_mul(shape_class.after.len())
            .is_some_and(|pairs| pairs <= ORDERED_ALIGNMENT_CANDIDATE_SCAN_LIMIT);
        if !within_scan_limit {
            groups.push(CandidateGroup {
                before: shape_class.before.clone(),
                after: shape_class.after.clone(),
                scan_complete: false,
            });
            continue;
        }

        let before_len = shape_class.before.len();
        let mut sets = DisjointSets::new(before_len + shape_class.after.len());
        let mut incident = vec![false; before_len + shape_class.after.len()];
        for (before_index, before_id) in shape_class.before.iter().copied().enumerate() {
            for (after_index, after_id) in shape_class.after.iter().copied().enumerate() {
                if phase_mappings_are_compatible(
                    before,
                    after,
                    global_mappings,
                    before_id,
                    after_id,
                ) {
                    let after_vertex = before_len + after_index;
                    sets.union(before_index, after_vertex);
                    incident[before_index] = true;
                    incident[after_vertex] = true;
                }
            }
        }

        let mut connected: BTreeMap<usize, CandidateGroup> = BTreeMap::new();
        for (before_index, before_id) in shape_class.before.iter().copied().enumerate() {
            if incident[before_index] {
                connected
                    .entry(sets.find(before_index))
                    .or_insert_with(|| CandidateGroup {
                        before: Vec::new(),
                        after: Vec::new(),
                        scan_complete: true,
                    })
                    .before
                    .push(before_id);
            }
        }
        for (after_index, after_id) in shape_class.after.iter().copied().enumerate() {
            let vertex = before_len + after_index;
            if incident[vertex] {
                connected
                    .entry(sets.find(vertex))
                    .or_insert_with(|| CandidateGroup {
                        before: Vec::new(),
                        after: Vec::new(),
                        scan_complete: true,
                    })
                    .after
                    .push(after_id);
            }
        }
        groups.extend(connected.into_values());
    }

    groups.sort_by_key(|group| (group.before[0], group.after[0]));
    let mut before_group = HashMap::new();
    let mut after_group = HashMap::new();
    for (group_id, group) in groups.iter().enumerate() {
        for before_id in &group.before {
            let previous = before_group.insert(*before_id, group_id);
            debug_assert!(previous.is_none());
        }
        for after_id in &group.after {
            let previous = after_group.insert(*after_id, group_id);
            debug_assert!(previous.is_none());
        }
    }
    CandidateGroups {
        groups,
        before_group,
        after_group,
    }
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
    let candidates = verified_candidate_groups(before, after, global_mappings, &partition);
    if candidates.groups.is_empty() {
        return Ok(AlignmentOutcome::default());
    }

    let mut outcome = AlignmentOutcome::default();
    for component in ordered_interaction_components(&candidates, before_ids, after_ids)? {
        let component_outcome = if component.before.len() <= ORDERED_ALIGNMENT_COMPONENT_LIMIT
            && component.after.len() <= ORDERED_ALIGNMENT_COMPONENT_LIMIT
        {
            verify_bounded_component(before, after, global_mappings, &candidates, &component)?
        } else {
            oversized_alignment(&candidates, &component)
        };
        outcome.forced.extend(component_outcome.forced);
        outcome.ambiguities.extend(component_outcome.ambiguities);
    }
    outcome.forced.sort_unstable();
    Ok(outcome)
}

fn verify_bounded_component(
    before: &ParsedSyntax,
    after: &ParsedSyntax,
    global_mappings: &PhaseMappings,
    candidate_groups: &CandidateGroups,
    component: &AlignmentComponent,
) -> Result<AlignmentOutcome> {
    let active_before = &component.before;
    let active_after = &component.after;

    let mut candidates = vec![vec![false; active_after.len()]; active_before.len()];
    for (before_index, before_id) in active_before.iter().copied().enumerate() {
        let before_group = candidate_groups
            .before_group
            .get(&before_id)
            .context("active before node is missing its verified candidate group")?;
        for (after_index, after_id) in active_after.iter().copied().enumerate() {
            let after_group = candidate_groups
                .after_group
                .get(&after_id)
                .context("active after node is missing its verified candidate group")?;
            candidates[before_index][after_index] = before_group == after_group
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
    let mut ambiguous_before = BTreeSet::new();
    let mut ambiguous_after = BTreeSet::new();
    let mut possible_pairs = Vec::new();
    let mut has_duplicate_symmetry = false;
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
            let group_id = *candidate_groups
                .before_group
                .get(&before_id)
                .context("active before node is missing its verified candidate group")?;
            let candidate_group = &candidate_groups.groups[group_id];
            let candidate_group_is_singleton =
                candidate_group.before.len() == 1 && candidate_group.after.len() == 1;
            if candidate_group_is_singleton
                && alignment_score(&candidates, Some((before_index, after_index))) < optimum
            {
                outcome.forced.push((before_id, after_id));
            } else {
                possible_pairs.push(AmbiguityPair {
                    before_id,
                    after_id,
                });
                if candidate_group_is_singleton {
                    ambiguous_before.insert(before_id);
                    ambiguous_after.insert(after_id);
                } else {
                    has_duplicate_symmetry = true;
                    ambiguous_before.extend(candidate_group.before.iter().copied());
                    ambiguous_after.extend(candidate_group.after.iter().copied());
                }
            }
        }
    }
    outcome.forced.sort_unstable();
    if !possible_pairs.is_empty() {
        let constraint = if has_duplicate_symmetry {
            AmbiguityConstraint::SymbolicAbstention {
                cause: AmbiguityAbstentionCause::DuplicateSymmetry,
                pair_claims: PairClaims::None,
            }
        } else {
            let required_matches = optimum - outcome.forced.len();
            if required_matches == 0 || possible_pairs.len() <= required_matches {
                bail!("ordered ambiguity does not contain multiple valid resolutions");
            }
            AmbiguityConstraint::ExactOrderedAlignment {
                predicate: Predicate::ShapeEqual,
                required_matches,
                possible_pairs,
            }
        };
        outcome.ambiguities.push(AlignmentAmbiguity {
            before: active_before
                .iter()
                .copied()
                .filter(|id| ambiguous_before.contains(id))
                .collect(),
            after: active_after
                .iter()
                .copied()
                .filter(|id| ambiguous_after.contains(id))
                .collect(),
            constraint,
            reason: if has_duplicate_symmetry {
                REPEATED_AMBIGUITY_REASON.to_owned()
            } else {
                OPTIONAL_AMBIGUITY_REASON.to_owned()
            },
        });
    }
    Ok(outcome)
}

fn ordered_interaction_components(
    candidates: &CandidateGroups,
    before_ids: &[usize],
    after_ids: &[usize],
) -> Result<Vec<AlignmentComponent>> {
    let active_before: Vec<_> = before_ids
        .iter()
        .copied()
        .filter(|id| candidates.before_group.contains_key(id))
        .collect();
    let active_after: Vec<_> = after_ids
        .iter()
        .copied()
        .filter(|id| candidates.after_group.contains_key(id))
        .collect();
    if active_before.is_empty() {
        return Ok(Vec::new());
    }

    let mut before_last = vec![0; candidates.groups.len()];
    for (position, id) in active_before.iter().enumerate() {
        let group_id = *candidates
            .before_group
            .get(id)
            .context("active before node is missing its verified candidate group")?;
        before_last[group_id] = position;
    }
    let mut after_last = vec![0; candidates.groups.len()];
    let mut after_counts = vec![0; candidates.groups.len()];
    for (position, id) in active_after.iter().enumerate() {
        let group_id = *candidates
            .after_group
            .get(id)
            .context("active after node is missing its verified candidate group")?;
        after_last[group_id] = position;
        after_counts[group_id] += 1;
    }

    let mut components = Vec::new();
    let mut seen = vec![false; candidates.groups.len()];
    let mut component_groups = Vec::new();
    let mut before_start = 0;
    let mut after_start = 0;
    let mut before_prefix_last = 0;
    let mut after_prefix_last = 0;
    let mut after_prefix_count = 0;

    // Re-derive only cuts whose complete groups occupy prefixes on both sides. This proves that
    // no candidate in one component can share an endpoint with or cross a candidate in the next.
    for (position, id) in active_before.iter().enumerate() {
        let group_id = *candidates
            .before_group
            .get(id)
            .context("active before node is missing its verified candidate group")?;
        if !seen[group_id] {
            seen[group_id] = true;
            component_groups.push(group_id);
            before_prefix_last = before_prefix_last.max(before_last[group_id]);
            after_prefix_last = after_prefix_last.max(after_last[group_id]);
            after_prefix_count += after_counts[group_id];
        }
        if position == before_prefix_last && after_prefix_count == after_prefix_last + 1 {
            components.push(AlignmentComponent {
                before: active_before[before_start..=position].to_vec(),
                after: active_after[after_start..after_prefix_count].to_vec(),
                groups: std::mem::take(&mut component_groups),
            });
            before_start = position + 1;
            after_start = after_prefix_count;
        }
    }

    if before_start != active_before.len()
        || after_start != active_after.len()
        || !component_groups.is_empty()
    {
        bail!("candidate groups do not form complete ordered interaction components");
    }
    Ok(components)
}

fn oversized_alignment(
    candidates: &CandidateGroups,
    component: &AlignmentComponent,
) -> AlignmentOutcome {
    let scan_complete = component
        .groups
        .iter()
        .all(|group_id| candidates.groups[*group_id].scan_complete);
    let cause = if scan_complete {
        AmbiguityAbstentionCause::ComponentLimit
    } else {
        AmbiguityAbstentionCause::CandidateScanLimit
    };
    let ambiguities = vec![AlignmentAmbiguity {
        before: component.before.clone(),
        after: component.after.clone(),
        constraint: AmbiguityConstraint::SymbolicAbstention {
            cause,
            pair_claims: PairClaims::None,
        },
        reason: if scan_complete {
            format!(
                "ordered alignment candidate region exceeds the {ORDERED_ALIGNMENT_COMPONENT_LIMIT}-child per-side cap; endpoint sets make no pair claims"
            )
        } else {
            format!(
                "shape-class candidate scan exceeds the {ORDERED_ALIGNMENT_CANDIDATE_SCAN_LIMIT}-pair cap; endpoint sets make no pair claims"
            )
        },
    }];
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
        let before_positions: HashMap<_, _> = group
            .before
            .iter()
            .enumerate()
            .map(|(position, node)| (node.id, position))
            .collect();
        let after_positions: HashMap<_, _> = group
            .after
            .iter()
            .enumerate()
            .map(|(position, node)| (node.id, position))
            .collect();
        ensure!(
            !group.before.is_empty() && !group.after.is_empty(),
            "ambiguity endpoint sets must be non-empty"
        );
        ensure!(
            before_positions.len() == group.before.len()
                && after_positions.len() == group.after.len(),
            "ambiguity endpoint sets contain duplicate nodes"
        );
        if let AmbiguityConstraint::ExactOrderedAlignment {
            predicate,
            required_matches,
            possible_pairs,
        } = &group.constraint
        {
            ensure!(
                *predicate == Predicate::ShapeEqual,
                "exact ordered ambiguity has an unsupported predicate"
            );
            ensure!(
                *required_matches > 0 && possible_pairs.len() > *required_matches,
                "exact ordered ambiguity must encode multiple non-empty resolutions"
            );
            ensure!(
                *required_matches <= group.before.len().min(group.after.len()),
                "exact ordered ambiguity requires more matches than its endpoint sets permit"
            );
            let mut seen_pairs = HashSet::new();
            let mut pair_order = Vec::with_capacity(possible_pairs.len());
            let mut referenced_before = HashSet::new();
            let mut referenced_after = HashSet::new();
            for pair in possible_pairs {
                let before_position = *before_positions
                    .get(&pair.before_id)
                    .context("ambiguity pair references a before node outside its endpoint set")?;
                let after_position = *after_positions
                    .get(&pair.after_id)
                    .context("ambiguity pair references an after node outside its endpoint set")?;
                ensure!(
                    seen_pairs.insert((pair.before_id, pair.after_id)),
                    "exact ordered ambiguity contains a duplicate pair"
                );
                ensure!(
                    shape_equal(before, pair.before_id, after, pair.after_id),
                    "exact ordered ambiguity pair does not satisfy its predicate"
                );
                referenced_before.insert(pair.before_id);
                referenced_after.insert(pair.after_id);
                pair_order.push((before_position, after_position));
            }
            ensure!(
                pair_order.windows(2).all(|pair| pair[0] < pair[1]),
                "exact ordered ambiguity pairs are not in canonical order"
            );
            ensure!(
                referenced_before.len() == group.before.len()
                    && referenced_after.len() == group.after.len(),
                "exact ordered ambiguity contains unreferenced endpoints"
            );
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
    use std::collections::HashMap;

    use super::{
        CandidateGroup, CandidateGroups, ordered_interaction_components, partition_index_bucket,
    };

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
    }

    #[test]
    fn independent_component_partition_rejects_crossing_cuts() {
        let (monotone, before, after) = candidate_partition(&[0, 0, 1, 2], &[0, 0, 1, 2]);
        let components = ordered_interaction_components(&monotone, &before, &after).unwrap();
        assert_eq!(components.len(), 3);
        assert_eq!(components[0].groups, [0]);
        assert_eq!(components[1].groups, [1]);
        assert_eq!(components[2].groups, [2]);

        let (crossing, before, after) = candidate_partition(&[0, 1], &[1, 0]);
        let components = ordered_interaction_components(&crossing, &before, &after).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].groups, [0, 1]);
    }

    fn candidate_partition(
        before_groups: &[usize],
        after_groups: &[usize],
    ) -> (CandidateGroups, Vec<usize>, Vec<usize>) {
        let group_count = before_groups
            .iter()
            .chain(after_groups)
            .copied()
            .max()
            .unwrap()
            + 1;
        let before: Vec<_> = (0..before_groups.len()).collect();
        let after: Vec<_> = (0..after_groups.len()).collect();
        let mut groups: Vec<_> = (0..group_count)
            .map(|_| CandidateGroup {
                before: Vec::new(),
                after: Vec::new(),
                scan_complete: true,
            })
            .collect();
        let mut before_group = HashMap::new();
        let mut after_group = HashMap::new();
        for (id, group_id) in before.iter().zip(before_groups) {
            groups[*group_id].before.push(*id);
            before_group.insert(*id, *group_id);
        }
        for (id, group_id) in after.iter().zip(after_groups) {
            groups[*group_id].after.push(*id);
            after_group.insert(*id, *group_id);
        }
        (
            CandidateGroups {
                groups,
                before_group,
                after_group,
            },
            before,
            after,
        )
    }
}
