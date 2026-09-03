use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::model::{
    AmbiguityGroup, ChangeKind, Correspondence, Predicate, Relation, StructuralChange,
};
use crate::syntax::{ParsedSyntax, shape_equal, syntax_equal};

/// Ordered alignment is quadratic in the number of active children. Larger regions are preserved
/// as symbolic ambiguity buckets without constructing their candidate matrix.
const ORDERED_ALIGNMENT_COMPONENT_LIMIT: usize = 64;

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
                    predicate: Predicate::ShapeEqual,
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
    reason: String,
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

    let active_before: Vec<_> = before_ids
        .iter()
        .copied()
        .filter(|id| verified.before_class.contains_key(id))
        .collect();
    let active_after: Vec<_> = after_ids
        .iter()
        .copied()
        .filter(|id| verified.after_class.contains_key(id))
        .collect();

    if !within_alignment_limit(active_before.len(), active_after.len()) {
        return oversized_alignment_ambiguities(&verified);
    }

    let mut candidates = vec![vec![false; active_after.len()]; active_before.len()];
    for (before_index, before_id) in active_before.iter().copied().enumerate() {
        for (after_index, after_id) in active_after.iter().copied().enumerate() {
            if verified.before_class[&before_id] == verified.after_class[&after_id]
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
    let mut ambiguous_by_class: BTreeMap<usize, (BTreeSet<usize>, BTreeSet<usize>)> =
        BTreeMap::new();
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
            let class_id = verified.before_class[&before_id];
            let shape_class = &verified.classes[class_id];
            let signature_is_unique = shape_class.before.len() == 1 && shape_class.after.len() == 1;
            let forced = candidate_is_forced(
                &candidates,
                optimum,
                before_index,
                after_index,
                signature_is_unique,
            );
            if forced {
                outcome.forced.push((before_id, after_id));
            } else {
                let endpoints = ambiguous_by_class.entry(class_id).or_default();
                if signature_is_unique {
                    endpoints.0.insert(before_id);
                    endpoints.1.insert(after_id);
                } else {
                    endpoints.0.extend(shape_class.before.iter().copied());
                    endpoints.1.extend(shape_class.after.iter().copied());
                }
            }
        }
    }

    outcome.forced.sort_unstable();
    for (class_id, (before, after)) in ambiguous_by_class {
        let shape_class = &verified.classes[class_id];
        let repeated = shape_class.before.len() > 1 || shape_class.after.len() > 1;
        outcome.ambiguities.push(AlignmentAmbiguity {
            before: before.into_iter().collect(),
            after: after.into_iter().collect(),
            reason: if repeated {
                "repeated shape-equivalent children are not treated as identities even when source order selects one optimal alignment"
                    .to_owned()
            } else {
                "the shape-equivalent pair is absent from at least one optimal ordered alignment"
                    .to_owned()
            },
        });
    }
    outcome
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
    before_class: HashMap<usize, usize>,
    after_class: HashMap<usize, usize>,
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

    let mut before_class = HashMap::new();
    let mut after_class = HashMap::new();
    for (class_id, shape_class) in classes.iter().enumerate() {
        for before_id in &shape_class.before {
            before_class.insert(*before_id, class_id);
        }
        for after_id in &shape_class.after {
            after_class.insert(*after_id, class_id);
        }
    }
    VerifiedShapeClasses {
        classes,
        before_class,
        after_class,
    }
}

fn within_alignment_limit(before: usize, after: usize) -> bool {
    before <= ORDERED_ALIGNMENT_COMPONENT_LIMIT && after <= ORDERED_ALIGNMENT_COMPONENT_LIMIT
}

fn oversized_alignment_ambiguities(verified: &VerifiedShapeClasses) -> AlignmentOutcome {
    AlignmentOutcome {
        forced: Vec::new(),
        ambiguities: verified
            .classes
            .iter()
            .map(|shape_class| AlignmentAmbiguity {
                before: shape_class.before.clone(),
                after: shape_class.after.clone(),
                reason: format!(
                    "ordered alignment candidate region exceeds the {ORDERED_ALIGNMENT_COMPONENT_LIMIT}-child per-side cap; candidates are preserved symbolically"
                ),
            })
            .collect(),
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
    signature_is_unique: bool,
) -> bool {
    signature_is_unique && alignment_score(candidates, Some((before_index, after_index))) < optimum
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
        alignment_prefix_scores, alignment_score, alignment_suffix_scores, candidate_is_forced,
        candidate_occurs_in_optimum, oversized_alignment_ambiguities, verified_shape_classes,
        within_alignment_limit,
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
    fn duplicate_signature_guard_preserves_identity_ambiguity() {
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
    fn raw_shape_hash_collision_is_partitioned_by_recursive_equality() {
        let before = colliding_shapes("identifier", "string");
        let after = colliding_shapes("string", "identifier");
        let verified = verified_shape_classes(&before, &after, &[0, 2], &[0, 2]);

        assert_eq!(verified.classes.len(), 2);
        assert_eq!(verified.classes[0].before, [0]);
        assert_eq!(verified.classes[0].after, [2]);
        assert_eq!(verified.classes[1].before, [2]);
        assert_eq!(verified.classes[1].after, [0]);
        assert_eq!(verified.before_class[&0], verified.after_class[&2]);
        assert_ne!(verified.before_class[&0], verified.after_class[&0]);

        let oversized = oversized_alignment_ambiguities(&verified);
        assert_eq!(oversized.ambiguities.len(), 2);
        assert_eq!(oversized.ambiguities[0].before, [0]);
        assert_eq!(oversized.ambiguities[0].after, [2]);
        assert_eq!(oversized.ambiguities[1].before, [2]);
        assert_eq!(oversized.ambiguities[1].after, [0]);
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
