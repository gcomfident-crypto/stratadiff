use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{
    AmbiguityGroup, ChangeKind, Correspondence, Predicate, Relation, StructuralChange,
};
use crate::syntax::{ParsedSyntax, shape_equal, syntax_equal};

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
            let predicate = if before_node.byte_hash == after_node.byte_hash {
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
    add_local_relations(&mut state);
    let (ambiguities, ambiguous_before, ambiguous_after) = collect_ambiguities(&state);
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

fn add_local_relations(state: &mut MappingState<'_>) {
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

        let before_children = unmatched_children_before(state, before_parent);
        let after_children = unmatched_children_after(state, after_parent);
        let shape_pairs = unique_pairs(state, &before_children, &after_children, |node| {
            (node.field.clone(), node.kind.clone(), node.shape_hash)
        });
        for (before_id, after_id) in shape_pairs {
            if shape_equal(state.before, before_id, state.after, after_id) {
                state.map_pair(
                    before_id,
                    after_id,
                    Predicate::ShapeEqual,
                    Correspondence::Suggested,
                    vec![
                        "unique_shape_under_mapped_parent".to_owned(),
                        "not_an_identity_claim".to_owned(),
                    ],
                );
            }
        }
    }
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

fn collect_ambiguities(
    state: &MappingState<'_>,
) -> (Vec<AmbiguityGroup>, HashSet<usize>, HashSet<usize>) {
    let mut groups = Vec::new();
    let mut ambiguous_before = HashSet::new();
    let mut ambiguous_after = HashSet::new();

    for (before_parent, after_parent) in state
        .before_to_after
        .iter()
        .enumerate()
        .filter_map(|(before_id, after_id)| after_id.map(|value| (before_id, value)))
    {
        let before_children = unmatched_children_before(state, before_parent);
        let after_children = unmatched_children_after(state, after_parent);
        let mut before_buckets: BTreeMap<(Option<String>, String, [u8; 32]), Vec<usize>> =
            BTreeMap::new();
        let mut after_buckets: BTreeMap<(Option<String>, String, [u8; 32]), Vec<usize>> =
            BTreeMap::new();
        for id in before_children {
            let node = &state.before.nodes[id];
            before_buckets
                .entry((node.field.clone(), node.kind.clone(), node.shape_hash))
                .or_default()
                .push(id);
        }
        for id in after_children {
            let node = &state.after.nodes[id];
            after_buckets
                .entry((node.field.clone(), node.kind.clone(), node.shape_hash))
                .or_default()
                .push(id);
        }
        for (key, before_ids) in before_buckets {
            let Some(after_ids) = after_buckets.get(&key) else {
                continue;
            };
            if before_ids.len() == 1 && after_ids.len() == 1 {
                continue;
            }
            ambiguous_before.extend(before_ids.iter().copied());
            ambiguous_after.extend(after_ids.iter().copied());
            groups.push(AmbiguityGroup {
                parent_before: before_parent,
                parent_after: after_parent,
                predicate: Predicate::ShapeEqual,
                before: before_ids
                    .iter()
                    .map(|id| state.before.nodes[*id].as_ref())
                    .collect(),
                after: after_ids
                    .iter()
                    .map(|id| state.after.nodes[*id].as_ref())
                    .collect(),
                reason: "multiple shape-equivalent children admit more than one correspondence"
                    .to_owned(),
            });
        }
    }
    groups.sort_by_key(|group| (group.parent_before, group.parent_after));
    (groups, ambiguous_before, ambiguous_after)
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
        if fact.correspondence == Correspondence::Suggested
            && fact.predicate == Predicate::ShapeEqual
            && !syntax_equal(state.before, before_id, state.after, after_id)
            && !before_parent.is_some_and(|parent| {
                state.facts[parent].is_some_and(|parent_fact| {
                    parent_fact.correspondence == Correspondence::Suggested
                })
            })
        {
            changes.push(StructuralChange {
                kind: ChangeKind::SuggestedUpdate,
                before: Some(state.before.nodes[before_id].as_ref()),
                after: Some(state.after.nodes[after_id].as_ref()),
                detail: "unique local shape match; reported as a suggestion, not an identity fact"
                    .to_owned(),
            });
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
