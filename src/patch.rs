use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use similar::{Algorithm, DiffTag, capture_diff_slices};

use stratadiff_core::{
    PATCH_ALGORITHM,
    model::{ByteEdit, LosslessPatch, ReplayCertificate},
};
use stratadiff_verifier::{VerificationLimits, replay_patch_with_limits};

const LINE_ANCHOR_BUDGET: usize = 64 * 1024;
const BYTE_MYERS_BUDGET: usize = 64 * 1024;
const LARGE_REGION_EDIT_BUDGET: usize = 4 * 1024;
const PATCH_EDIT_BUDGET: usize = 64 * 1024;

pub(crate) fn create_patch(before: &[u8], after: &[u8]) -> LosslessPatch {
    if line_anchor_budget_exceeded(before, after) {
        return create_byte_refined_patch(before, after);
    }

    let before_lines: Vec<_> = before.split_inclusive(|byte| *byte == b'\n').collect();
    let after_lines: Vec<_> = after.split_inclusive(|byte| *byte == b'\n').collect();
    let before_offsets = line_offsets(&before_lines);
    let after_offsets = line_offsets(&after_lines);
    let mut edits = Vec::new();

    for operation in capture_diff_slices(Algorithm::Patience, &before_lines, &after_lines) {
        let (tag, old_lines, new_lines) = operation.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        let old_start = before_offsets[old_lines.start];
        let old_end = before_offsets[old_lines.end];
        let new_start = after_offsets[new_lines.start];
        let new_end = after_offsets[new_lines.end];
        if !refine_changed_region(
            before, after, old_start, old_end, new_start, new_end, &mut edits,
        ) {
            return replacement_patch(before, after);
        }
    }
    patch_with_edits(edits)
}

fn line_anchor_budget_exceeded(before: &[u8], after: &[u8]) -> bool {
    let mut line_anchors = 0;
    for source in [before, after] {
        for _ in source.split_inclusive(|byte| *byte == b'\n') {
            if line_anchors == LINE_ANCHOR_BUDGET {
                return true;
            }
            line_anchors += 1;
        }
    }
    false
}

fn create_byte_refined_patch(before: &[u8], after: &[u8]) -> LosslessPatch {
    let mut edits = Vec::new();
    if refine_changed_region(before, after, 0, before.len(), 0, after.len(), &mut edits) {
        patch_with_edits(edits)
    } else {
        replacement_patch(before, after)
    }
}

fn line_offsets(lines: &[&[u8]]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    offsets.push(0);
    for line in lines {
        offsets.push(offsets.last().expect("offset zero exists") + line.len());
    }
    offsets
}

fn refine_changed_region(
    before: &[u8],
    after: &[u8],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    edits: &mut Vec<ByteEdit>,
) -> bool {
    let old = &before[old_start..old_end];
    let new = &after[new_start..new_end];
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let trimmed_old_start = old_start + prefix;
    let trimmed_old_end = old_end - suffix;
    let trimmed_new_start = new_start + prefix;
    let trimmed_new_end = new_end - suffix;
    let trimmed_old = &before[trimmed_old_start..trimmed_old_end];
    let trimmed_new = &after[trimmed_new_start..trimmed_new_end];

    if trimmed_old.len().saturating_add(trimmed_new.len()) <= BYTE_MYERS_BUDGET {
        for operation in capture_diff_slices(Algorithm::Myers, trimmed_old, trimmed_new) {
            let (tag, old_range, new_range) = operation.as_tag_tuple();
            if tag == DiffTag::Equal {
                continue;
            }
            if edits.len() == PATCH_EDIT_BUDGET {
                return false;
            }
            edits.push(ByteEdit {
                old_start: trimmed_old_start + old_range.start,
                old_end: trimmed_old_start + old_range.end,
                replacement_base64: STANDARD.encode(&trimmed_new[new_range]),
            });
        }
    } else if trimmed_old.len() == trimmed_new.len() {
        if edits.len() == PATCH_EDIT_BUDGET {
            return false;
        }
        refine_large_equal_length_region(trimmed_old, trimmed_new, trimmed_old_start, edits);
    } else {
        if edits.len() == PATCH_EDIT_BUDGET {
            return false;
        }
        edits.push(ByteEdit {
            old_start: trimmed_old_start,
            old_end: trimmed_old_end,
            replacement_base64: STANDARD.encode(trimmed_new),
        });
    }
    true
}

fn refine_large_equal_length_region(
    old: &[u8],
    new: &[u8],
    old_start: usize,
    edits: &mut Vec<ByteEdit>,
) {
    let available = PATCH_EDIT_BUDGET - edits.len();
    let edit_budget = available.min(LARGE_REGION_EDIT_BUDGET);
    if edit_budget == 0 {
        return;
    }

    let mut cursor = 0;
    let mut emitted = 0;
    while cursor < old.len() {
        while cursor < old.len() && old[cursor] == new[cursor] {
            cursor += 1;
        }
        if cursor == old.len() {
            return;
        }

        let mismatch_start = cursor;
        while cursor < old.len() && old[cursor] != new[cursor] {
            cursor += 1;
        }

        if emitted + 1 == edit_budget {
            edits.push(ByteEdit {
                old_start: old_start + mismatch_start,
                old_end: old_start + old.len(),
                replacement_base64: STANDARD.encode(&new[mismatch_start..]),
            });
            return;
        }

        edits.push(ByteEdit {
            old_start: old_start + mismatch_start,
            old_end: old_start + cursor,
            replacement_base64: STANDARD.encode(&new[mismatch_start..cursor]),
        });
        emitted += 1;
    }
}

fn replacement_patch(before: &[u8], after: &[u8]) -> LosslessPatch {
    patch_with_edits(if before == after {
        Vec::new()
    } else {
        vec![ByteEdit {
            old_start: 0,
            old_end: before.len(),
            replacement_base64: STANDARD.encode(after),
        }]
    })
}

fn patch_with_edits(edits: Vec<ByteEdit>) -> LosslessPatch {
    LosslessPatch {
        algorithm: PATCH_ALGORITHM.to_owned(),
        edits,
    }
}

pub(crate) fn create_certificate(
    before: &[u8],
    after: &[u8],
    patch: &LosslessPatch,
    limits: &VerificationLimits,
) -> Result<ReplayCertificate> {
    let reconstructed = replay_patch_with_limits(before, patch, limits)?;
    if reconstructed != after {
        bail!("internal invariant failed: generated patch did not reconstruct the target bytes");
    }
    Ok(ReplayCertificate {
        before_blake3: blake3::hash(before).to_hex().to_string(),
        after_blake3: blake3::hash(after).to_hex().to_string(),
        reconstructed_blake3: blake3::hash(&reconstructed).to_hex().to_string(),
        before_len: before.len(),
        after_len: after.len(),
        patch_verified: true,
    })
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use proptest::prelude::*;
    use stratadiff_core::LosslessPatch;
    use stratadiff_verifier::apply_patch;

    use super::{
        BYTE_MYERS_BUDGET, LARGE_REGION_EDIT_BUDGET, LINE_ANCHOR_BUDGET, PATCH_EDIT_BUDGET,
        create_patch, line_anchor_budget_exceeded,
    };

    proptest! {
        #[test]
        fn arbitrary_bytes_replay_exactly(
            before in proptest::collection::vec(any::<u8>(), 0..256),
            after in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let patch = create_patch(&before, &after);
            prop_assert_eq!(apply_patch(&before, &patch).unwrap(), after);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn arbitrary_large_bytes_produce_canonical_ordered_replayable_edits(
            before in proptest::collection::vec(any::<u8>(), 48_000..96_000),
            first_delta in 1u8..=u8::MAX,
            second_delta in 1u8..=u8::MAX,
        ) {
            let mut after = before.clone();
            let first = before.len() / 8;
            let second = before.len() * 7 / 8;
            after[first] ^= first_delta;
            after[second] ^= second_delta;

            let patch = create_patch(&before, &after);
            prop_assert_eq!(&patch, &create_patch(&before, &after));
            prop_assert!(patch.edits.len() <= PATCH_EDIT_BUDGET);
            let mut previous_end = 0;
            for edit in &patch.edits {
                prop_assert!(previous_end <= edit.old_start);
                prop_assert!(edit.old_start <= edit.old_end);
                prop_assert!(edit.old_end <= before.len());
                let replacement = STANDARD.decode(&edit.replacement_base64).unwrap();
                prop_assert_eq!(
                    STANDARD.encode(replacement),
                    edit.replacement_base64.as_str()
                );
                previous_end = edit.old_end;
            }
            prop_assert_eq!(apply_patch(&before, &patch).unwrap(), after);
        }
    }

    #[test]
    fn large_unrelated_binary_regions_take_the_bounded_path() {
        let before = vec![0x11; 100_000];
        let after = vec![0xee; 100_000];
        let patch = create_patch(&before, &after);
        assert_eq!(patch.edits.len(), 1);
        assert_eq!(apply_patch(&before, &patch).unwrap(), after);
    }

    #[test]
    fn sparse_edits_in_a_long_single_line_remain_local() {
        let before = vec![b'a'; 100_000];
        let mut after = before.clone();
        after[100] = b'b';
        after[90_000] = b'c';

        let patch = create_patch(&before, &after);

        assert_eq!(patch.edits.len(), 2);
        assert_eq!(patch.edits[0].old_start, 100);
        assert_eq!(patch.edits[0].old_end, 101);
        assert_eq!(patch.edits[1].old_start, 90_000);
        assert_eq!(patch.edits[1].old_end, 90_001);
        assert_eq!(apply_patch(&before, &patch).unwrap(), after);
    }

    #[test]
    fn pathological_alternating_changes_have_bounded_patch_size() {
        let before = vec![0; 1_000_000];
        let after: Vec<_> = (0..before.len())
            .map(|index| if index % 2 == 0 { 1 } else { 0 })
            .collect();

        let patch = create_patch(&before, &after);

        assert_eq!(patch.edits.len(), LARGE_REGION_EDIT_BUDGET);
        assert_patch_invariants(&before, &after, &patch);
    }

    #[test]
    fn byte_refinement_budget_boundary_replays_exactly() {
        for (old_len, new_len) in [
            (BYTE_MYERS_BUDGET / 2, BYTE_MYERS_BUDGET / 2),
            (BYTE_MYERS_BUDGET / 2, BYTE_MYERS_BUDGET / 2 + 1),
        ] {
            let before = vec![0x11; old_len];
            let after = vec![0xee; new_len];
            let patch = create_patch(&before, &after);

            assert_patch_invariants(&before, &after, &patch);
        }
    }

    #[test]
    fn large_region_edit_budget_boundaries_are_exact() {
        for mismatch_runs in [
            LARGE_REGION_EDIT_BUDGET - 1,
            LARGE_REGION_EDIT_BUDGET,
            LARGE_REGION_EDIT_BUDGET + 1,
        ] {
            let len = (mismatch_runs - 1) * 9 + 1;
            let before = vec![0; len];
            let mut after = before.clone();
            for run in 0..mismatch_runs {
                after[run * 9] = 1;
            }

            let patch = create_patch(&before, &after);

            assert_eq!(
                patch.edits.len(),
                mismatch_runs.min(LARGE_REGION_EDIT_BUDGET)
            );
            if mismatch_runs > LARGE_REGION_EDIT_BUDGET {
                let tail = patch.edits.last().unwrap();
                assert_eq!(tail.old_start, (LARGE_REGION_EDIT_BUDGET - 1) * 9);
                assert_eq!(tail.old_end, len);
            }
            assert_patch_invariants(&before, &after, &patch);
        }
    }

    #[test]
    fn newline_dense_input_bypasses_line_anchor_materialization() {
        let exact_left = vec![b'\n'; LINE_ANCHOR_BUDGET / 2];
        let exact_right = vec![b'\n'; LINE_ANCHOR_BUDGET / 2];
        assert!(!line_anchor_budget_exceeded(&exact_left, &exact_right));

        let before = vec![b'\n'; LINE_ANCHOR_BUDGET + 1];
        let mut after = before.clone();
        let changed = LINE_ANCHOR_BUDGET / 2;
        after[changed] = 0xff;

        assert!(line_anchor_budget_exceeded(&before, &after));
        let patch = create_patch(&before, &after);

        assert_eq!(patch.edits.len(), 1);
        assert_eq!(patch.edits[0].old_start, changed);
        assert_eq!(patch.edits[0].old_end, changed + 1);
        assert_patch_invariants(&before, &after, &patch);
    }

    fn assert_patch_invariants(before: &[u8], after: &[u8], patch: &LosslessPatch) {
        assert!(patch.edits.len() <= PATCH_EDIT_BUDGET);
        let mut previous_end = 0;
        for edit in &patch.edits {
            assert!(previous_end <= edit.old_start);
            assert!(edit.old_start <= edit.old_end);
            assert!(edit.old_end <= before.len());
            let replacement = STANDARD.decode(&edit.replacement_base64).unwrap();
            assert_eq!(
                STANDARD.encode(replacement),
                edit.replacement_base64.as_str()
            );
            previous_end = edit.old_end;
        }
        assert_eq!(apply_patch(before, patch).unwrap(), after);
    }
}
