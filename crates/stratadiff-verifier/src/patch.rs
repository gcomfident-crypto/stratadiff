use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use stratadiff_core::LosslessPatch;

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
