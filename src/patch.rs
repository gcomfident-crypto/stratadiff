use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use similar::{Algorithm, DiffTag, capture_diff_slices};

use crate::model::{ByteEdit, LosslessPatch, ReplayCertificate};

pub(crate) fn create_patch(before: &[u8], after: &[u8]) -> LosslessPatch {
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
        refine_changed_region(
            before, after, old_start, old_end, new_start, new_end, &mut edits,
        );
    }
    LosslessPatch {
        algorithm: "patience-lines+bounded-myers-bytes-v1".to_owned(),
        edits,
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
) {
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

    const BYTE_MYERS_BUDGET: usize = 64 * 1024;
    if trimmed_old.len() + trimmed_new.len() <= BYTE_MYERS_BUDGET {
        for operation in capture_diff_slices(Algorithm::Myers, trimmed_old, trimmed_new) {
            let (tag, old_range, new_range) = operation.as_tag_tuple();
            if tag == DiffTag::Equal {
                continue;
            }
            edits.push(ByteEdit {
                old_start: trimmed_old_start + old_range.start,
                old_end: trimmed_old_start + old_range.end,
                replacement_base64: STANDARD.encode(&trimmed_new[new_range]),
            });
        }
    } else {
        edits.push(ByteEdit {
            old_start: trimmed_old_start,
            old_end: trimmed_old_end,
            replacement_base64: STANDARD.encode(trimmed_new),
        });
    }
}

pub fn apply_patch(before: &[u8], patch: &LosslessPatch) -> Result<Vec<u8>> {
    if patch.algorithm != "patience-lines+bounded-myers-bytes-v1" {
        bail!("unsupported patch algorithm {}", patch.algorithm);
    }

    let mut cursor = 0;
    let mut output = Vec::new();
    for edit in &patch.edits {
        if edit.old_start < cursor || edit.old_end < edit.old_start || edit.old_end > before.len() {
            bail!(
                "invalid or overlapping edit range {}..{}",
                edit.old_start,
                edit.old_end
            );
        }
        output.extend_from_slice(&before[cursor..edit.old_start]);
        output.extend(STANDARD.decode(&edit.replacement_base64)?);
        cursor = edit.old_end;
    }
    output.extend_from_slice(&before[cursor..]);
    Ok(output)
}

pub(crate) fn create_certificate(
    before: &[u8],
    after: &[u8],
    patch: &LosslessPatch,
) -> Result<ReplayCertificate> {
    let reconstructed = apply_patch(before, patch)?;
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
    use proptest::prelude::*;

    use super::{apply_patch, create_patch};

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

    #[test]
    fn large_unrelated_binary_regions_take_the_bounded_path() {
        let before = vec![0x11; 100_000];
        let after = vec![0xee; 100_000];
        let patch = create_patch(&before, &after);
        assert_eq!(patch.edits.len(), 1);
        assert_eq!(apply_patch(&before, &patch).unwrap(), after);
    }
}
