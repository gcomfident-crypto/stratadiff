use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::model::{
    AmbiguityAbstentionCause, AmbiguityConstraint, AmbiguityGroup, AmbiguityPair, ChangeKind,
    Correspondence, PairClaims, Predicate, Relation, StructuralChange,
};
use crate::syntax::{ParsedSyntax, shape_equal, syntax_equal};

/// Ordered alignment is quadratic in the number of active children in an order-interaction
/// component. Larger components are preserved symbolically without constructing their matrix.
const ORDERED_ALIGNMENT_COMPONENT_LIMIT: usize = 64;
const ORDERED_ALIGNMENT_CANDIDATE_SCAN_LIMIT: usize =
    ORDERED_ALIGNMENT_COMPONENT_LIMIT * ORDERED_ALIGNMENT_COMPONENT_LIMIT * 4;
const REPEATED_AMBIGUITY_REASON: &str = "repeated shape-equivalent children are intentionally unresolved; endpoint sets make no pair claims";
const OPTIONAL_AMBIGUITY_REASON: &str =
    "multiple maximum-cardinality ordered alignments have coupled candidate choices";

type ShapeKey = (Option<String>, String, [u8; 32]);

pub(crate) struct MatchResult {
    pub relations: Vec<Relation>,
    pub ambiguities: Vec<AmbiguityGroup>,
    pub changes: Vec<StructuralChange>,
}

#[derive(Clone, Copy)]
struct MappingFact {
    predicate: Predicate,
    correspondence: Correspondence,
}

struct MappingState<'a> {
    before: &'a ParsedSyntax,
    after: &'a ParsedSyntax,
    before_to_after: Vec<Option<usize>>,
    after_to_before: Vec<Option<usize>>,
    facts: Vec<Option<MappingFact>>,
    relations: Vec<Relation>,
}

impl<'a> MappingState<'a> {
    fn new(before: &'a ParsedSyntax, after: &'a ParsedSyntax) -> Self {
        Self {
            before,
            after,
            before_to_after: vec![None; before.nodes.len()],
            after_to_before: vec![None; after.nodes.len()],
            facts: vec![None; before.nodes.len()],
            relations: Vec::new(),
        }
    }

    fn map_pair(
        &mut self,
        before_id: usize,
        after_id: usize,
        predicate: Predicate,
        correspondence: Correspondence,
        evidence: Vec<String>,
    ) -> bool {
        match (
            self.before_to_after[before_id],
            self.after_to_before[after_id],
        ) {
            (Some(existing_after), Some(existing_before)) => {
                return existing_after == after_id && existing_before == before_id;
            }
            (None, None) => {}
            _ => return false,
        }
        self.before_to_after[before_id] = Some(after_id);
        self.after_to_before[after_id] = Some(before_id);
        self.facts[before_id] = Some(MappingFact {
            predicate,
            correspondence,
        });
        self.relations.push(Relation {
            before: self.before.nodes[before_id].as_ref(),
            after: self.after.nodes[after_id].as_ref(),
            predicate,
            correspondence,
            evidence,
        });
        true
    }

    fn can_map_equal_subtree(&self, before_id: usize, after_id: usize) -> bool {
        let before_node = &self.before.nodes[before_id];
        let after_node = &self.after.nodes[after_id];
        if before_node.kind != after_node.kind
            || before_node.children.len() != after_node.children.len()
        {
            return false;
        }
        if self.before_to_after[before_id].is_some_and(|mapped| mapped != after_id)
            || self.after_to_before[after_id].is_some_and(|mapped| mapped != before_id)
        {
            return false;
        }
        before_node
            .children
            .iter()
            .zip(&after_node.children)
            .all(|(before_child, after_child)| {
                self.can_map_equal_subtree(*before_child, *after_child)
            })
    }

    fn can_map_shape_pair(&self, before_id: usize, after_id: usize) -> bool {
        if self.before_to_after[before_id].is_some_and(|mapped| mapped != after_id)
            || self.after_to_before[after_id].is_some_and(|mapped| mapped != before_id)
        {
            return false;
        }
        let before_node = &self.before.nodes[before_id];
        let after_node = &self.after.nodes[after_id];
        before_node.children.len() == after_node.children.len()
            && before_node.children.iter().zip(&after_node.children).all(
                |(before_child, after_child)| self.can_map_shape_pair(*before_child, *after_child),
            )
    }

    fn map_equal_subtree(
        &mut self,
        before_id: usize,
        after_id: usize,
        root_evidence: &str,
    ) -> bool {
        if !self.can_map_equal_subtree(before_id, after_id)
            || !syntax_equal(self.before, before_id, self.after, after_id)
        {
            return false;
        }
        self.map_equal_subtree_unchecked(before_id, after_id, root_evidence, true);
        true
    }

    fn map_equal_subtree_unchecked(
        &mut self,
        before_id: usize,
        after_id: usize,
        root_evidence: &str,
        is_root: bool,
    ) {
        let before_node = &self.before.nodes[before_id];
        let after_node = &self.after.nodes[after_id];
        if self.before_to_after[before_id].is_none() {
            let before_bytes =
                &self.before.source[before_node.span.start_byte..before_node.span.end_byte];
            let after_bytes =
                &self.after.source[after_node.span.start_byte..after_node.span.end_byte];
            let predicate =
                if before_node.byte_hash == after_node.byte_hash && before_bytes == after_bytes {
                    Predicate::ByteEqual
                } else {
                    Predicate::SyntaxEqual
                };
            let evidence = if is_root {
                vec![
                    root_evidence.to_owned(),
                    "recursive_syntax_equality_check".to_owned(),
                ]
            } else {
                vec!["isomorphic_path_under_exact_anchor".to_owned()]
            };
            self.map_pair(
                before_id,
                after_id,
                predicate,
                Correspondence::ModelForced,
                evidence,
            );
        }
        let children: Vec<_> = before_node
            .children
            .iter()
            .copied()
            .zip(after_node.children.iter().copied())
            .collect();
        for (before_child, after_child) in children {
            self.map_equal_subtree_unchecked(before_child, after_child, root_evidence, false);
        }
    }
}

pub(crate) fn match_trees(before: &ParsedSyntax, after: &ParsedSyntax) -> MatchResult {
    let mut state = MappingState::new(before, after);
    state.map_pair(
        before.root,
        after.root,
        Predicate::InputPair,
        Correspondence::InputPair,
        vec!["caller_supplied_file_pair".to_owned()],
    );

    add_global_exact_anchors(&mut state);
    let (ambiguities, ambiguous_before, ambiguous_after) = add_local_relations(&mut state);
    let changes = derive_changes(&state, &ambiguous_before, &ambiguous_after);
    state.relations.sort_by_key(|relation| relation.before.id);

    MatchResult {
        relations: state.relations,
        ambiguities,
        changes,
    }
}

fn add_global_exact_anchors(state: &mut MappingState<'_>) {
    let mut before_index: HashMap<(String, [u8; 32]), Vec<usize>> = HashMap::new();
    let mut after_index: HashMap<(String, [u8; 32]), Vec<usize>> = HashMap::new();
    for node in state
        .before
        .nodes
        .iter()
        .filter(|node| node.id != state.before.root && node.subtree_size >= 3)
    {
        before_index
            .entry((node.kind.clone(), node.syntax_hash))
            .or_default()
            .push(node.id);
    }
    for node in state
        .after
        .nodes
        .iter()
        .filter(|node| node.id != state.after.root && node.subtree_size >= 3)
    {
        after_index
            .entry((node.kind.clone(), node.syntax_hash))
            .or_default()
            .push(node.id);
    }

    let mut candidates = Vec::new();
    for (key, before_ids) in before_index {
        let Some(after_ids) = after_index.get(&key) else {
            continue;
        };
        if before_ids.len() == 1 && after_ids.len() == 1 {
            candidates.push((
                state.before.nodes[before_ids[0]].subtree_size,
                before_ids[0],
                after_ids[0],
            ));
        }
    }
    candidates.sort_by_key(|(size, before_id, after_id)| {
        (std::cmp::Reverse(*size), *before_id, *after_id)
    });
    for (_, before_id, after_id) in candidates {
        if state.before_to_after[before_id] == Some(after_id) {
            continue;
        }
        state.map_equal_subtree(
            before_id,
            after_id,
            "globally_unique_identical_syntax_subtree",
        );
    }
}

fn add_local_relations(
    state: &mut MappingState<'_>,
) -> (Vec<AmbiguityGroup>, HashSet<usize>, HashSet<usize>) {
    let mut ambiguities = Vec::new();
    let mut ambiguous_before = HashSet::new();
    let mut ambiguous_after = HashSet::new();
    let mut cursor = 0;
    while cursor < state.relations.len() {
        let before_parent = state.relations[cursor].before.id;
        let after_parent = state.relations[cursor].after.id;
        cursor += 1;

        let before_children = unmatched_children_before(state, before_parent);
        let after_children = unmatched_children_after(state, after_parent);
        let exact_pairs = unique_pairs(state, &before_children, &after_children, |node| {
            (node.field.clone(), node.kind.clone(), node.syntax_hash)
        });
        for (before_id, after_id) in exact_pairs {
            state.map_equal_subtree(
                before_id,
                after_id,
                "unique_identical_child_under_mapped_parent",
            );
        }

        for (before_children, after_children) in
            ordered_alignment_regions(state, before_parent, after_parent)
        {
            let alignment = bounded_ordered_stable_core(state, &before_children, &after_children);
            for (before_id, after_id) in alignment.forced {
                let mapped = state.map_pair(
                    before_id,
                    after_id,
                    Predicate::ShapeEqual,
                    Correspondence::ModelForced,
                    vec![
                        "bounded_ordered_child_alignment_v1".to_owned(),
                        "pair_present_in_every_optimal_alignment".to_owned(),
                        "recursive_shape_equality_check".to_owned(),
                        "not_a_historical_identity_claim".to_owned(),
                    ],
                );
                debug_assert!(mapped, "stable-core pairs must be mutually compatible");
            }
            for ambiguity in alignment.ambiguities {
                ambiguous_before.extend(ambiguity.before.iter().copied());
                ambiguous_after.extend(ambiguity.after.iter().copied());
                ambiguities.push(AmbiguityGroup {
                    parent_before: before_parent,
                    parent_after: after_parent,
                    before: ambiguity
                        .before
                        .iter()
                        .map(|id| state.before.nodes[*id].as_ref())
                        .collect(),
                    after: ambiguity
                        .after
                        .iter()
                        .map(|id| state.after.nodes[*id].as_ref())
                        .collect(),
                    constraint: ambiguity.constraint,
                    reason: ambiguity.reason,
                });
            }
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
    (ambiguities, ambiguous_before, ambiguous_after)
}

fn ordered_alignment_regions(
    state: &MappingState<'_>,
    before_parent: usize,
    after_parent: usize,
) -> Vec<(Vec<usize>, Vec<usize>)> {
    let before_children = &state.before.nodes[before_parent].children;
    let after_children = &state.after.nodes[after_parent].children;
    let after_positions: HashMap<_, _> = after_children
        .iter()
        .enumerate()
        .map(|(position, id)| (*id, position))
        .collect();
    let exact_anchors: Vec<_> = before_children
        .iter()
        .enumerate()
        .filter_map(|(before_position, before_id)| {
            let after_id = state.before_to_after[*before_id]?;
            let after_position = after_positions.get(&after_id).copied()?;
            let fact = state.facts[*before_id]?;
            if fact.correspondence == Correspondence::ModelForced
                && matches!(
                    fact.predicate,
                    Predicate::ByteEqual | Predicate::SyntaxEqual
                )
            {
                Some((before_position, after_position))
            } else {
                None
            }
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
        let before = before_children[before_start..before_end]
            .iter()
            .copied()
            .filter(|id| state.before_to_after[*id].is_none())
            .collect();
        let after = after_children[after_start..after_end]
            .iter()
            .copied()
            .filter(|id| state.after_to_before[*id].is_none())
            .collect();
        regions.push((before, after));
        before_start = before_end.saturating_add(1);
        after_start = after_end.saturating_add(1);
    }
    regions
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

fn bounded_ordered_stable_core(
    state: &MappingState<'_>,
    before_ids: &[usize],
    after_ids: &[usize],
) -> AlignmentOutcome {
    let verified = verified_shape_classes(state.before, state.after, before_ids, after_ids);
    if verified.classes.is_empty() {
        return AlignmentOutcome::default();
    }
    let candidates = verified_candidate_groups(state, &verified);
    if candidates.groups.is_empty() {
        return AlignmentOutcome::default();
    }

    let mut outcome = AlignmentOutcome::default();
    for component in ordered_interaction_components(&candidates, before_ids, after_ids) {
        let component_outcome =
            if within_alignment_limit(component.before.len(), component.after.len()) {
                bounded_component_stable_core(state, &candidates, &component)
            } else {
                oversized_alignment_ambiguities(&candidates, &component)
            };
        outcome.forced.extend(component_outcome.forced);
        outcome.ambiguities.extend(component_outcome.ambiguities);
    }
    outcome.forced.sort_unstable();
    outcome
}

fn bounded_component_stable_core(
    state: &MappingState<'_>,
    candidate_groups: &CandidateGroups,
    component: &AlignmentComponent,
) -> AlignmentOutcome {
    let active_before = &component.before;
    let active_after = &component.after;

    let mut candidates = vec![vec![false; active_after.len()]; active_before.len()];
    for (before_index, before_id) in active_before.iter().copied().enumerate() {
        for (after_index, after_id) in active_after.iter().copied().enumerate() {
            if candidate_groups.before_group[&before_id] == candidate_groups.after_group[&after_id]
                && shape_equal(state.before, before_id, state.after, after_id)
                && state.can_map_shape_pair(before_id, after_id)
            {
                candidates[before_index][after_index] = true;
            }
        }
    }

    let prefix = alignment_prefix_scores(&candidates, None);
    let suffix = alignment_suffix_scores(&candidates);
    let optimum = prefix[active_before.len()][active_after.len()];
    if optimum == 0 {
        return AlignmentOutcome::default();
    }

    let mut outcome = AlignmentOutcome::default();
    let mut ambiguous_before = BTreeSet::new();
    let mut ambiguous_after = BTreeSet::new();
    let mut possible_pairs = Vec::new();
    let mut has_duplicate_symmetry = false;
    for (before_index, row) in candidates.iter().enumerate() {
        for (after_index, _) in row.iter().enumerate() {
            if !candidate_occurs_in_optimum(
                &candidates,
                &prefix,
                &suffix,
                optimum,
                before_index,
                after_index,
            ) {
                continue;
            }
            let before_id = active_before[before_index];
            let after_id = active_after[after_index];
            let group_id = candidate_groups.before_group[&before_id];
            let candidate_group = &candidate_groups.groups[group_id];
            let candidate_group_is_singleton =
                candidate_group.before.len() == 1 && candidate_group.after.len() == 1;
            let forced = candidate_is_forced(
                &candidates,
                optimum,
                before_index,
                after_index,
                candidate_group_is_singleton,
            );
            if forced {
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
            debug_assert!(required_matches > 0);
            debug_assert!(possible_pairs.len() > required_matches);
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
    outcome
}

fn ordered_interaction_components(
    candidates: &CandidateGroups,
    before_ids: &[usize],
    after_ids: &[usize],
) -> Vec<AlignmentComponent> {
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
        return Vec::new();
    }

    let mut before_last = vec![0; candidates.groups.len()];
    for (position, id) in active_before.iter().enumerate() {
        before_last[candidates.before_group[id]] = position;
    }
    let mut after_last = vec![0; candidates.groups.len()];
    let mut after_counts = vec![0; candidates.groups.len()];
    for (position, id) in active_after.iter().enumerate() {
        let group_id = candidates.after_group[id];
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

    // A cut is independent only when complete connected candidate groups occupy prefixes on both
    // sides. Every candidate then has both endpoints on one side, so choices cannot interact.
    for (position, id) in active_before.iter().enumerate() {
        let group_id = candidates.before_group[id];
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

    debug_assert_eq!(before_start, active_before.len());
    debug_assert_eq!(after_start, active_after.len());
    debug_assert!(component_groups.is_empty());
    components
}

fn shape_key(node: &crate::syntax::SyntaxNode) -> ShapeKey {
    (node.field.clone(), node.kind.clone(), node.shape_hash)
}

fn shape_buckets(syntax: &ParsedSyntax, ids: &[usize]) -> BTreeMap<ShapeKey, Vec<usize>> {
    let mut buckets = BTreeMap::new();
    for id in ids {
        buckets
            .entry(shape_key(&syntax.nodes[*id]))
            .or_insert_with(Vec::new)
            .push(*id);
    }
    buckets
}

struct ShapeClass {
    before: Vec<usize>,
    after: Vec<usize>,
}

struct VerifiedShapeClasses {
    classes: Vec<ShapeClass>,
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
    let before_buckets = shape_buckets(before, before_ids);
    let after_buckets = shape_buckets(after, after_ids);
    let mut classes = Vec::new();

    for (key, before_bucket) in before_buckets {
        let Some(after_bucket) = after_buckets.get(&key) else {
            continue;
        };
        let mut key_classes: Vec<ShapeClass> = Vec::new();
        for before_id in before_bucket {
            if let Some(shape_class) = key_classes
                .iter_mut()
                .find(|shape_class| shape_equal(before, before_id, before, shape_class.before[0]))
            {
                shape_class.before.push(before_id);
            } else {
                key_classes.push(ShapeClass {
                    before: vec![before_id],
                    after: Vec::new(),
                });
            }
        }
        for after_id in after_bucket {
            if let Some(shape_class) = key_classes
                .iter_mut()
                .find(|shape_class| shape_equal(before, shape_class.before[0], after, *after_id))
            {
                shape_class.after.push(*after_id);
            }
        }
        classes.extend(
            key_classes
                .into_iter()
                .filter(|shape_class| !shape_class.after.is_empty()),
        );
    }

    VerifiedShapeClasses { classes }
}

fn verified_candidate_groups(
    state: &MappingState<'_>,
    verified: &VerifiedShapeClasses,
) -> CandidateGroups {
    let mut groups = Vec::new();
    for shape_class in &verified.classes {
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
                if state.can_map_shape_pair(before_id, after_id) {
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

fn within_alignment_limit(before: usize, after: usize) -> bool {
    before <= ORDERED_ALIGNMENT_COMPONENT_LIMIT && after <= ORDERED_ALIGNMENT_COMPONENT_LIMIT
}

fn oversized_alignment_ambiguities(
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
    AlignmentOutcome {
        forced: Vec::new(),
        ambiguities: vec![AlignmentAmbiguity {
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
        }],
    }
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

fn candidate_occurs_in_optimum(
    candidates: &[Vec<bool>],
    prefix: &[Vec<usize>],
    suffix: &[Vec<usize>],
    optimum: usize,
    before_index: usize,
    after_index: usize,
) -> bool {
    candidates[before_index][after_index]
        && prefix[before_index][after_index] + 1 + suffix[before_index + 1][after_index + 1]
            == optimum
}

fn candidate_is_forced(
    candidates: &[Vec<bool>],
    optimum: usize,
    before_index: usize,
    after_index: usize,
    candidate_group_is_singleton: bool,
) -> bool {
    candidate_group_is_singleton
        && alignment_score(candidates, Some((before_index, after_index))) < optimum
}

fn unique_pairs<K: Ord>(
    state: &MappingState<'_>,
    before_ids: &[usize],
    after_ids: &[usize],
    key: impl Fn(&crate::syntax::SyntaxNode) -> K,
) -> Vec<(usize, usize)> {
    let mut before_buckets: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    let mut after_buckets: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for id in before_ids {
        before_buckets
            .entry(key(&state.before.nodes[*id]))
            .or_default()
            .push(*id);
    }
    for id in after_ids {
        after_buckets
            .entry(key(&state.after.nodes[*id]))
            .or_default()
            .push(*id);
    }
    before_buckets
        .into_iter()
        .filter_map(|(key, before_bucket)| {
            let after_bucket = after_buckets.get(&key)?;
            if before_bucket.len() == 1 && after_bucket.len() == 1 {
                Some((before_bucket[0], after_bucket[0]))
            } else {
                None
            }
        })
        .collect()
}

fn unmatched_children_before(state: &MappingState<'_>, parent: usize) -> Vec<usize> {
    state.before.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|id| state.before_to_after[*id].is_none())
        .collect()
}

fn unmatched_children_after(state: &MappingState<'_>, parent: usize) -> Vec<usize> {
    state.after.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|id| state.after_to_before[*id].is_none())
        .collect()
}

fn derive_changes(
    state: &MappingState<'_>,
    ambiguous_before: &HashSet<usize>,
    ambiguous_after: &HashSet<usize>,
) -> Vec<StructuralChange> {
    let mut changes = Vec::new();

    if syntax_equal(
        state.before,
        state.before.root,
        state.after,
        state.after.root,
    ) && state.before.source != state.after.source
    {
        changes.push(StructuralChange {
            kind: ChangeKind::FormattingOnly,
            before: Some(state.before.nodes[state.before.root].as_ref()),
            after: Some(state.after.nodes[state.after.root].as_ref()),
            detail: "syntax is identical after discarding trivia; bytes differ".to_owned(),
        });
    }

    for (before_parent, after_parent) in state
        .before_to_after
        .iter()
        .enumerate()
        .filter_map(|(before_id, after_id)| after_id.map(|value| (before_id, value)))
    {
        let after_positions: HashMap<usize, usize> = state.after.nodes[after_parent]
            .children
            .iter()
            .enumerate()
            .map(|(position, child)| (*child, position))
            .collect();
        let mapped_positions: Vec<_> = state.before.nodes[before_parent]
            .children
            .iter()
            .filter_map(|before_child| {
                let after_child = state.before_to_after[*before_child]?;
                if state.after.nodes[after_child].parent != Some(after_parent) {
                    return None;
                }
                after_positions.get(&after_child).copied()
            })
            .collect();
        if mapped_positions.windows(2).any(|pair| pair[0] > pair[1]) {
            changes.push(StructuralChange {
                kind: ChangeKind::ChildOrderChanged,
                before: Some(state.before.nodes[before_parent].as_ref()),
                after: Some(state.after.nodes[after_parent].as_ref()),
                detail: "the relative order of mapped direct children changed; no historical move identity is asserted"
                    .to_owned(),
            });
        }
    }

    for (before_id, after_id) in state
        .before_to_after
        .iter()
        .enumerate()
        .filter_map(|(before_id, after_id)| after_id.map(|value| (before_id, value)))
        .filter(|(before_id, _)| *before_id != state.before.root)
    {
        let fact = state.facts[before_id].expect("every mapped node has a mapping fact");
        let before_parent = state.before.nodes[before_id].parent;
        let after_parent = state.after.nodes[after_id].parent;
        if fact.correspondence == Correspondence::ModelForced
            && matches!(
                fact.predicate,
                Predicate::ByteEqual | Predicate::SyntaxEqual
            )
            && before_parent.is_some()
            && after_parent.is_some()
            && state.before_to_after[before_parent.expect("checked")].is_some()
            && state.before_to_after[before_parent.expect("checked")] != after_parent
        {
            changes.push(StructuralChange {
                kind: ChangeKind::EquivalentRelocation,
                before: Some(state.before.nodes[before_id].as_ref()),
                after: Some(state.after.nodes[after_id].as_ref()),
                detail: "an exact syntax subtree occurs under a different mapped parent".to_owned(),
            });
        }
        if fact.predicate == Predicate::ShapeEqual
            && !syntax_equal(state.before, before_id, state.after, after_id)
            && !before_parent.is_some_and(|parent| {
                state.facts[parent]
                    .is_some_and(|parent_fact| parent_fact.predicate == Predicate::ShapeEqual)
            })
        {
            match fact.correspondence {
                Correspondence::ModelForced => changes.push(StructuralChange {
                    kind: ChangeKind::ModelForcedUpdate,
                    before: Some(state.before.nodes[before_id].as_ref()),
                    after: Some(state.after.nodes[after_id].as_ref()),
                    detail: "the pair occurs in every optimal ordered alignment and has equal shape but different syntax; no historical identity is asserted"
                        .to_owned(),
                }),
                Correspondence::Suggested => changes.push(StructuralChange {
                    kind: ChangeKind::SuggestedUpdate,
                    before: Some(state.before.nodes[before_id].as_ref()),
                    after: Some(state.after.nodes[after_id].as_ref()),
                    detail:
                        "local shape match; reported as a suggestion, not an identity fact"
                            .to_owned(),
                }),
                Correspondence::InputPair => {}
            }
        }
    }

    for node in state.before.nodes.iter().skip(1) {
        if state.before_to_after[node.id].is_none()
            && !ambiguous_before.contains(&node.id)
            && node
                .parent
                .is_some_and(|parent| state.before_to_after[parent].is_some())
        {
            changes.push(StructuralChange {
                kind: ChangeKind::Delete,
                before: Some(node.as_ref()),
                after: None,
                detail: "maximal unmatched subtree in the before snapshot".to_owned(),
            });
        }
    }
    for node in state.after.nodes.iter().skip(1) {
        if state.after_to_before[node.id].is_none()
            && !ambiguous_after.contains(&node.id)
            && node
                .parent
                .is_some_and(|parent| state.after_to_before[parent].is_some())
        {
            changes.push(StructuralChange {
                kind: ChangeKind::Insert,
                before: None,
                after: Some(node.as_ref()),
                detail: "maximal unmatched subtree in the after snapshot".to_owned(),
            });
        }
    }
    changes.sort_by_key(|change| {
        (
            change.before.as_ref().map_or(usize::MAX, |node| node.id),
            change.after.as_ref().map_or(usize::MAX, |node| node.id),
            change.kind as u8,
        )
    });
    changes
}

#[cfg(test)]
mod tests {
    use crate::model::{Position, Span};
    use crate::syntax::{ParsedSyntax, SyntaxNode};

    use super::{
        AlignmentComponent, CandidateGroup, CandidateGroups, DisjointSets, alignment_prefix_scores,
        alignment_score, alignment_suffix_scores, candidate_is_forced, candidate_occurs_in_optimum,
        ordered_interaction_components, verified_shape_classes, within_alignment_limit,
    };

    #[test]
    fn unique_candidate_is_in_the_stable_core() {
        let candidates = vec![vec![true]];
        let prefix = alignment_prefix_scores(&candidates, None);
        let suffix = alignment_suffix_scores(&candidates);
        let optimum = alignment_score(&candidates, None);

        assert!(candidate_occurs_in_optimum(
            &candidates,
            &prefix,
            &suffix,
            optimum,
            0,
            0
        ));
        assert!(candidate_is_forced(&candidates, optimum, 0, 0, true));
    }

    #[test]
    fn crossing_candidates_are_optional_across_optimal_alignments() {
        let candidates = vec![vec![false, true], vec![true, false]];
        let prefix = alignment_prefix_scores(&candidates, None);
        let suffix = alignment_suffix_scores(&candidates);
        let optimum = alignment_score(&candidates, None);

        assert_eq!(optimum, 1);
        for (before_index, after_index) in [(0, 1), (1, 0)] {
            assert!(candidate_occurs_in_optimum(
                &candidates,
                &prefix,
                &suffix,
                optimum,
                before_index,
                after_index
            ));
            assert!(!candidate_is_forced(
                &candidates,
                optimum,
                before_index,
                after_index,
                true
            ));
        }
    }

    #[test]
    fn duplicate_candidate_group_guard_preserves_identity_ambiguity() {
        let candidates = vec![vec![true, true], vec![true, true]];
        let optimum = alignment_score(&candidates, None);

        assert_eq!(optimum, 2);
        assert!(alignment_score(&candidates, Some((0, 0))) < optimum);
        assert!(!candidate_is_forced(&candidates, optimum, 0, 0, false));
        assert!(!candidate_is_forced(&candidates, optimum, 1, 1, false));
    }

    #[test]
    fn alignment_matrix_is_bounded_per_side() {
        assert!(within_alignment_limit(64, 64));
        assert!(!within_alignment_limit(65, 1));
        assert!(!within_alignment_limit(1, 65));
    }

    #[test]
    fn order_interaction_components_split_only_at_compatible_prefixes() {
        let (monotone, before, after) = candidate_partition(&[0, 0, 1, 2, 2], &[0, 0, 1, 2, 2]);
        let components = ordered_interaction_components(&monotone, &before, &after);
        assert_eq!(components.len(), 3);
        assert_eq!(components[0].groups, [0]);
        assert_eq!(components[0].before.len(), 2);
        assert_eq!(components[1].groups, [1]);
        assert_eq!(components[2].groups, [2]);

        let (crossing, before, after) = candidate_partition(&[0, 1], &[1, 0]);
        let components = ordered_interaction_components(&crossing, &before, &after);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].groups, [0, 1]);

        let (interleaved, before, after) = candidate_partition(&[0, 1, 0, 2], &[0, 0, 1, 2]);
        let components = ordered_interaction_components(&interleaved, &before, &after);
        assert_eq!(components.len(), 2);
        assert_eq!(components[0].groups, [0, 1]);
        assert_eq!(components[0].before.len(), 3);
        assert_eq!(components[0].after.len(), 3);
        assert_eq!(components[1].groups, [2]);
    }

    #[test]
    fn component_optima_equal_the_full_ordered_optimum() {
        for bits in 1_u16..(1 << 9) {
            let mut candidates = vec![vec![false; 3]; 3];
            for index in 0..9 {
                candidates[index / 3][index % 3] = bits & (1 << index) != 0;
            }
            let (graph, before, after) = candidate_graph(&candidates);
            let components = ordered_interaction_components(&graph, &before, &after);
            let full_optimum = alignment_score(&candidates, None);
            let component_optimum: usize = components
                .iter()
                .map(|component| alignment_score(&component_matrix(component, &candidates), None))
                .sum();
            assert_eq!(
                component_optimum, full_optimum,
                "wrong decomposition for candidate bits {bits:09b}"
            );

            for (before_index, row) in candidates.iter().enumerate() {
                for (after_index, is_candidate) in row.iter().copied().enumerate() {
                    if !is_candidate {
                        continue;
                    }
                    let globally_forced =
                        alignment_score(&candidates, Some((before_index, after_index)))
                            < full_optimum;
                    let after_id = 100 + after_index;
                    let component = components
                        .iter()
                        .find(|component| {
                            component.before.contains(&before_index)
                                && component.after.contains(&after_id)
                        })
                        .unwrap();
                    let local_before = component
                        .before
                        .iter()
                        .position(|id| *id == before_index)
                        .unwrap();
                    let local_after = component
                        .after
                        .iter()
                        .position(|id| *id == after_id)
                        .unwrap();
                    let component_candidates = component_matrix(component, &candidates);
                    let local_optimum = alignment_score(&component_candidates, None);
                    let locally_forced =
                        alignment_score(&component_candidates, Some((local_before, local_after)))
                            < local_optimum;
                    assert_eq!(
                        locally_forced, globally_forced,
                        "wrong forcedness for candidate bits {bits:09b} at ({before_index}, {after_index})"
                    );
                }
            }
        }
    }

    #[test]
    fn raw_shape_hash_collision_is_partitioned_by_recursive_equality() {
        let before = colliding_shapes("identifier", "string");
        let after = colliding_shapes("string", "identifier");
        let verified = verified_shape_classes(&before, &after, &[0, 2], &[0, 2]);

        assert_eq!(verified.classes.len(), 2);
        assert_eq!(verified.classes[0].before, [0]);
        assert_eq!(verified.classes[0].after, [2]);
        assert_eq!(verified.classes[1].before, [2]);
        assert_eq!(verified.classes[1].after, [0]);
    }

    #[test]
    fn dynamic_program_matches_all_three_by_three_alignments() {
        for bits in 0_u16..(1 << 9) {
            let mut candidates = vec![vec![false; 3]; 3];
            for index in 0..9 {
                candidates[index / 3][index % 3] = bits & (1 << index) != 0;
            }
            let alignments = maximum_alignments(&candidates);
            let optimum = alignment_score(&candidates, None);
            let prefix = alignment_prefix_scores(&candidates, None);
            let suffix = alignment_suffix_scores(&candidates);

            assert_eq!(
                optimum,
                alignments[0].len(),
                "wrong optimum for candidate bits {bits:09b}"
            );
            for before_index in 0..3 {
                for after_index in 0..3 {
                    let appears = alignments
                        .iter()
                        .any(|alignment| alignment.contains(&(before_index, after_index)));
                    let forced = alignments
                        .iter()
                        .all(|alignment| alignment.contains(&(before_index, after_index)));
                    assert_eq!(
                        candidate_occurs_in_optimum(
                            &candidates,
                            &prefix,
                            &suffix,
                            optimum,
                            before_index,
                            after_index,
                        ),
                        appears,
                        "wrong possible edge for candidate bits {bits:09b}"
                    );
                    assert_eq!(
                        candidate_is_forced(&candidates, optimum, before_index, after_index, true,),
                        forced,
                        "wrong forced edge for candidate bits {bits:09b}"
                    );
                }
            }
        }
    }

    #[test]
    fn residual_pair_constraints_encode_every_three_by_three_optimum() {
        for bits in 0_u16..(1 << 9) {
            let mut candidates = vec![vec![false; 3]; 3];
            for index in 0..9 {
                candidates[index / 3][index % 3] = bits & (1 << index) != 0;
            }
            let alignments = maximum_alignments(&candidates);
            let optimum = alignments[0].len();
            let forced: std::collections::BTreeSet<_> = (0..3)
                .flat_map(|before_index| (0..3).map(move |after_index| (before_index, after_index)))
                .filter(|pair| alignments.iter().all(|alignment| alignment.contains(pair)))
                .collect();
            let possible: Vec<_> = (0..3)
                .flat_map(|before_index| (0..3).map(move |after_index| (before_index, after_index)))
                .filter(|pair| {
                    !forced.contains(pair)
                        && alignments.iter().any(|alignment| alignment.contains(pair))
                })
                .collect();
            let required_matches = optimum - forced.len();
            let actual: std::collections::BTreeSet<Vec<_>> = alignments
                .iter()
                .map(|alignment| {
                    alignment
                        .iter()
                        .copied()
                        .filter(|pair| !forced.contains(pair))
                        .collect()
                })
                .collect();
            let encoded = constrained_resolutions(&possible, required_matches);
            assert_eq!(
                encoded, actual,
                "residual constraint mismatch for candidate bits {bits:09b}"
            );
            if actual.len() > 1 {
                assert!(possible.len() > required_matches);
            }
        }
    }

    fn maximum_alignments(candidates: &[Vec<bool>]) -> Vec<Vec<(usize, usize)>> {
        let mut alignments = Vec::new();
        enumerate_alignments(candidates, 0, 0, &mut Vec::new(), &mut alignments);
        let optimum = alignments.iter().map(Vec::len).max().unwrap();
        alignments.retain(|alignment| alignment.len() == optimum);
        alignments
    }

    fn enumerate_alignments(
        candidates: &[Vec<bool>],
        before_index: usize,
        after_start: usize,
        current: &mut Vec<(usize, usize)>,
        alignments: &mut Vec<Vec<(usize, usize)>>,
    ) {
        if before_index == candidates.len() {
            alignments.push(current.clone());
            return;
        }
        enumerate_alignments(
            candidates,
            before_index + 1,
            after_start,
            current,
            alignments,
        );
        for after_index in after_start..candidates[before_index].len() {
            if candidates[before_index][after_index] {
                current.push((before_index, after_index));
                enumerate_alignments(
                    candidates,
                    before_index + 1,
                    after_index + 1,
                    current,
                    alignments,
                );
                current.pop();
            }
        }
    }

    fn constrained_resolutions(
        possible: &[(usize, usize)],
        required_matches: usize,
    ) -> std::collections::BTreeSet<Vec<(usize, usize)>> {
        let mut resolutions = std::collections::BTreeSet::new();
        for bits in 0_usize..(1 << possible.len()) {
            let selected: Vec<_> = possible
                .iter()
                .enumerate()
                .filter(|(index, _)| bits & (1 << index) != 0)
                .map(|(_, pair)| *pair)
                .collect();
            if selected.len() == required_matches
                && selected
                    .windows(2)
                    .all(|pairs| pairs[0].0 < pairs[1].0 && pairs[0].1 < pairs[1].1)
            {
                resolutions.insert(selected);
            }
        }
        resolutions
    }

    fn colliding_shapes(first_child_kind: &str, second_child_kind: &str) -> ParsedSyntax {
        let collision_hash = [7; 32];
        ParsedSyntax {
            source: Vec::new(),
            nodes: vec![
                syntax_node(0, "container", None, vec![1], collision_hash),
                syntax_node(1, first_child_kind, Some(0), Vec::new(), [1; 32]),
                syntax_node(2, "container", None, vec![3], collision_hash),
                syntax_node(3, second_child_kind, Some(2), Vec::new(), [2; 32]),
            ],
            root: 0,
            root_kind: "container".to_owned(),
        }
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
        let after: Vec<_> = (100..100 + after_groups.len()).collect();
        let mut groups: Vec<_> = (0..group_count)
            .map(|_| CandidateGroup {
                before: Vec::new(),
                after: Vec::new(),
                scan_complete: true,
            })
            .collect();
        let mut before_group = std::collections::HashMap::new();
        let mut after_group = std::collections::HashMap::new();
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

    fn candidate_graph(candidates: &[Vec<bool>]) -> (CandidateGroups, Vec<usize>, Vec<usize>) {
        let before: Vec<_> = (0..candidates.len()).collect();
        let after_len = candidates[0].len();
        let after: Vec<_> = (100..100 + after_len).collect();
        let mut sets = DisjointSets::new(before.len() + after.len());
        let mut incident = vec![false; before.len() + after.len()];
        for (before_index, row) in candidates.iter().enumerate() {
            for (after_index, is_candidate) in row.iter().copied().enumerate() {
                if is_candidate {
                    let after_vertex = before.len() + after_index;
                    sets.union(before_index, after_vertex);
                    incident[before_index] = true;
                    incident[after_vertex] = true;
                }
            }
        }

        let mut connected: std::collections::BTreeMap<usize, CandidateGroup> =
            std::collections::BTreeMap::new();
        for (before_index, before_id) in before.iter().copied().enumerate() {
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
        for (after_index, after_id) in after.iter().copied().enumerate() {
            let vertex = before.len() + after_index;
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
        let groups: Vec<_> = connected.into_values().collect();
        let mut before_group = std::collections::HashMap::new();
        let mut after_group = std::collections::HashMap::new();
        for (group_id, group) in groups.iter().enumerate() {
            for before_id in &group.before {
                before_group.insert(*before_id, group_id);
            }
            for after_id in &group.after {
                after_group.insert(*after_id, group_id);
            }
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

    fn component_matrix(
        component: &AlignmentComponent,
        candidates: &[Vec<bool>],
    ) -> Vec<Vec<bool>> {
        component
            .before
            .iter()
            .map(|before_id| {
                component
                    .after
                    .iter()
                    .map(|after_id| candidates[*before_id][*after_id - 100])
                    .collect()
            })
            .collect()
    }

    fn syntax_node(
        id: usize,
        kind: &str,
        parent: Option<usize>,
        children: Vec<usize>,
        shape_hash: [u8; 32],
    ) -> SyntaxNode {
        SyntaxNode {
            id,
            kind: kind.to_owned(),
            named: true,
            extra: false,
            missing: false,
            field: None,
            span: Span {
                start_byte: 0,
                end_byte: 0,
                start: Position { row: 0, column: 0 },
                end: Position { row: 0, column: 0 },
            },
            parent,
            children,
            subtree_size: 1,
            byte_hash: [0; 32],
            syntax_hash: [0; 32],
            shape_hash,
        }
    }
}
