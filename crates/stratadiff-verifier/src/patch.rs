use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use stratadiff_core::LosslessPatch;

use crate::limits::{VerificationLimits, inspect_patch};

pub fn apply_patch(before: &[u8], patch: &LosslessPatch) -> Result<Vec<u8>> {
    replay_patch_with_limits(before, patch, &VerificationLimits::default())
}

pub fn replay_patch_with_limits(
    before: &[u8],
    patch: &LosslessPatch,
    limits: &VerificationLimits,
) -> Result<Vec<u8>> {
    if before.len() > limits.max_source_bytes {
        anyhow::bail!(
            "before source bytes limit exceeded: observed {}, limit {}",
            before.len(),
            limits.max_source_bytes
        );
    }
    let stats = inspect_patch(before, patch, limits)?;
    let reserve = stats
        .replayed_bytes
        .checked_add(2)
        .context("patch replay allocation exceeds usize capacity")?;
    let mut cursor = 0;
    let mut output = Vec::new();
    output
        .try_reserve_exact(reserve)
        .context("failed to reserve bounded patch replay output")?;
    for (index, edit) in patch.edits.iter().enumerate() {
        output.extend_from_slice(&before[cursor..edit.old_start]);
        let start = output.len();
        STANDARD
            .decode_vec(&edit.replacement_base64, &mut output)
            .with_context(|| format!("patch replacement at edit {index} is not valid base64"))?;
        let decoded = output
            .len()
            .checked_sub(start)
            .context("decoded replacement length underflow")?;
        ensure!(
            decoded <= limits.max_decoded_replacement_bytes,
            "decoded replacement bytes limit exceeded at patch edit {index}"
        );
        cursor = edit.old_end;
    }
    output.extend_from_slice(&before[cursor..]);
    ensure!(
        output.len() == stats.replayed_bytes,
        "patch replay length differs from its checked preflight length"
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use stratadiff_core::{ByteEdit, LosslessPatch, PATCH_ALGORITHM};

    use super::{apply_patch, replay_patch_with_limits};
    use crate::VerificationLimits;

    #[test]
    fn replacement_limit_accepts_boundary_and_rejects_plus_one() {
        let patch = replacement("YWJj");
        let exact = VerificationLimits {
            max_source_bytes: 3,
            max_decoded_replacement_bytes: 3,
            ..VerificationLimits::default()
        };
        assert_eq!(
            replay_patch_with_limits(b"x", &patch, &exact).unwrap(),
            b"abc"
        );

        let too_small = VerificationLimits {
            max_decoded_replacement_bytes: 2,
            ..exact
        };
        let error = replay_patch_with_limits(b"x", &patch, &too_small).unwrap_err();
        assert_eq!(
            error.to_string(),
            "decoded replacement bytes limit exceeded: observed 3, limit 2"
        );
    }

    #[test]
    fn noncanonical_base64_is_rejected_before_replay() {
        let error =
            replay_patch_with_limits(b"x", &replacement("Zh=="), &VerificationLimits::default())
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("patch replacement is not canonical RFC 4648 base64")
        );
    }

    #[test]
    fn compatibility_wrapper_uses_default_limits() {
        assert_eq!(apply_patch(b"x", &replacement("eQ==")).unwrap(), b"y");
    }

    fn replacement(encoded: &str) -> LosslessPatch {
        LosslessPatch {
            algorithm: PATCH_ALGORITHM.to_owned(),
            edits: vec![ByteEdit {
                old_start: 0,
                old_end: 1,
                replacement_base64: encoded.to_owned(),
            }],
        }
    }
}
