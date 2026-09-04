use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use stratadiff_core::{AmbiguityConstraint, DiffReport, LosslessPatch, PATCH_ALGORITHM};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerificationLimits {
    pub max_report_bytes: usize,
    pub max_source_bytes: usize,
    pub max_relations: usize,
    pub max_ambiguity_groups: usize,
    pub max_ambiguity_endpoints: usize,
    pub max_ambiguity_pairs: usize,
    pub max_changes: usize,
    pub max_patch_edits: usize,
    pub max_decoded_replacement_bytes: usize,
    pub max_syntax_nodes: usize,
    pub max_syntax_depth: usize,
    pub max_parse_callbacks: usize,
    pub max_verification_work: usize,
}

impl Default for VerificationLimits {
    fn default() -> Self {
        Self {
            max_report_bytes: 64 * 1024 * 1024,
            max_source_bytes: 16 * 1024 * 1024,
            max_relations: 250_000,
            max_ambiguity_groups: 50_000,
            max_ambiguity_endpoints: 500_000,
            max_ambiguity_pairs: 1_000_000,
            max_changes: 250_000,
            max_patch_edits: 250_000,
            max_decoded_replacement_bytes: 32 * 1024 * 1024,
            max_syntax_nodes: 1_000_000,
            max_syntax_depth: 512,
            max_parse_callbacks: 4_000_000,
            max_verification_work: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerificationStats {
    pub report_bytes: usize,
    pub before_source_bytes: usize,
    pub after_source_bytes: usize,
    pub relations: usize,
    pub ambiguity_groups: usize,
    pub ambiguity_endpoints: usize,
    pub ambiguity_pairs: usize,
    pub changes: usize,
    pub patch_edits: usize,
    pub decoded_replacement_bytes: usize,
    pub syntax_nodes: usize,
    pub verification_work: usize,
}

#[derive(Debug)]
pub(crate) struct WorkBudget {
    limit: usize,
    used: usize,
}

impl WorkBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit, used: 0 }
    }

    pub(crate) fn charge(&mut self, amount: usize, context: &str) -> Result<()> {
        let next = self
            .used
            .checked_add(amount)
            .with_context(|| format!("verification work exceeds usize capacity while {context}"))?;
        if next > self.limit {
            bail!(
                "verification work limit exceeded: observed {next}, limit {}, while {context}",
                self.limit
            );
        }
        self.used = next;
        Ok(())
    }

    pub(crate) fn charge_product(
        &mut self,
        left: usize,
        right: usize,
        context: &str,
    ) -> Result<()> {
        let amount = left
            .checked_mul(right)
            .with_context(|| format!("verification work exceeds usize capacity while {context}"))?;
        self.charge(amount, context)
    }

    pub(crate) fn charge_n_log_n(&mut self, len: usize, context: &str) -> Result<()> {
        let levels = if len <= 1 {
            1
        } else {
            usize::BITS as usize - (len - 1).leading_zeros() as usize
        };
        self.charge_product(len, levels, context)
    }

    pub(crate) fn used(&self) -> usize {
        self.used
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PatchStats {
    pub decoded_replacement_bytes: usize,
    pub replayed_bytes: usize,
}

pub(crate) fn check_limit(label: &str, observed: usize, limit: usize) -> Result<()> {
    if observed > limit {
        bail!("{label} limit exceeded: observed {observed}, limit {limit}");
    }
    Ok(())
}

pub(crate) fn checked_add(label: &str, left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .with_context(|| format!("{label} exceeds usize capacity"))
}

pub(crate) fn measure_report_bytes(
    report: &DiffReport,
    limits: &VerificationLimits,
) -> Result<usize> {
    let mut writer = LimitWriter {
        written: 0,
        limit: limits.max_report_bytes,
    };
    serde_json::to_writer(&mut writer, report).context("failed to measure serialized report")?;
    Ok(writer.written)
}

pub(crate) fn preflight_report(
    report: &DiffReport,
    report_bytes: usize,
    before: &[u8],
    after: &[u8],
    limits: &VerificationLimits,
) -> Result<VerificationStats> {
    check_limit("report bytes", report_bytes, limits.max_report_bytes)?;
    check_limit("before source bytes", before.len(), limits.max_source_bytes)?;
    check_limit("after source bytes", after.len(), limits.max_source_bytes)?;
    check_limit("relations", report.relations.len(), limits.max_relations)?;
    check_limit(
        "ambiguity groups",
        report.ambiguities.len(),
        limits.max_ambiguity_groups,
    )?;
    check_limit("changes", report.changes.len(), limits.max_changes)?;

    let mut ambiguity_endpoints = 0;
    let mut ambiguity_pairs = 0;
    for ambiguity in &report.ambiguities {
        ambiguity_endpoints = checked_add(
            "ambiguity endpoint count",
            ambiguity_endpoints,
            ambiguity.before.len(),
        )?;
        ambiguity_endpoints = checked_add(
            "ambiguity endpoint count",
            ambiguity_endpoints,
            ambiguity.after.len(),
        )?;
        if let AmbiguityConstraint::ExactOrderedAlignment { possible_pairs, .. } =
            &ambiguity.constraint
        {
            ambiguity_pairs = checked_add(
                "ambiguity possible-pair count",
                ambiguity_pairs,
                possible_pairs.len(),
            )?;
        }
    }
    check_limit(
        "ambiguity endpoints",
        ambiguity_endpoints,
        limits.max_ambiguity_endpoints,
    )?;
    check_limit(
        "ambiguity possible pairs",
        ambiguity_pairs,
        limits.max_ambiguity_pairs,
    )?;

    let patch = inspect_patch(before, &report.patch, limits)?;
    let syntax_nodes = checked_add(
        "syntax node count",
        report.parser.before_nodes,
        report.parser.after_nodes,
    )?;
    check_limit("syntax nodes", syntax_nodes, limits.max_syntax_nodes)?;

    Ok(VerificationStats {
        report_bytes,
        before_source_bytes: before.len(),
        after_source_bytes: after.len(),
        relations: report.relations.len(),
        ambiguity_groups: report.ambiguities.len(),
        ambiguity_endpoints,
        ambiguity_pairs,
        changes: report.changes.len(),
        patch_edits: report.patch.edits.len(),
        decoded_replacement_bytes: patch.decoded_replacement_bytes,
        syntax_nodes,
        verification_work: 0,
    })
}

pub(crate) fn inspect_patch(
    before: &[u8],
    patch: &LosslessPatch,
    limits: &VerificationLimits,
) -> Result<PatchStats> {
    if patch.algorithm != PATCH_ALGORITHM {
        bail!("unsupported patch algorithm {}", patch.algorithm);
    }
    check_limit("patch edits", patch.edits.len(), limits.max_patch_edits)?;

    let mut previous_end = 0;
    let mut removed_bytes = 0;
    let mut decoded_replacement_bytes = 0;
    for (index, edit) in patch.edits.iter().enumerate() {
        if edit.old_start < previous_end
            || edit.old_end < edit.old_start
            || edit.old_end > before.len()
        {
            bail!(
                "invalid or overlapping edit range {}..{} at patch edit {index}",
                edit.old_start,
                edit.old_end
            );
        }
        removed_bytes = checked_add(
            "removed patch byte count",
            removed_bytes,
            edit.old_end - edit.old_start,
        )?;
        let decoded = canonical_base64_decoded_len(&edit.replacement_base64).map_err(|error| {
            anyhow::anyhow!("invalid replacement at patch edit {index}: {error}")
        })?;
        decoded_replacement_bytes = checked_add(
            "decoded replacement byte count",
            decoded_replacement_bytes,
            decoded,
        )?;
        check_limit(
            "decoded replacement bytes",
            decoded_replacement_bytes,
            limits.max_decoded_replacement_bytes,
        )?;
        previous_end = edit.old_end;
    }

    let preserved_bytes = before
        .len()
        .checked_sub(removed_bytes)
        .context("removed patch byte count exceeds the before source length")?;
    let replayed_bytes = checked_add(
        "replayed output byte count",
        preserved_bytes,
        decoded_replacement_bytes,
    )?;
    check_limit(
        "replayed output bytes",
        replayed_bytes,
        limits.max_source_bytes,
    )?;

    Ok(PatchStats {
        decoded_replacement_bytes,
        replayed_bytes,
    })
}

fn canonical_base64_decoded_len(encoded: &str) -> Result<usize> {
    let bytes = encoded.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        bail!("patch replacement is not canonical RFC 4648 base64");
    }
    if bytes.is_empty() {
        return Ok(0);
    }

    let padding = if bytes.ends_with(b"==") {
        2
    } else if bytes.ends_with(b"=") {
        1
    } else {
        0
    };
    let data_len = bytes.len() - padding;
    if bytes[..data_len]
        .iter()
        .any(|byte| base64_value(*byte).is_none())
        || bytes[data_len..].iter().any(|byte| *byte != b'=')
    {
        bail!("patch replacement is not canonical RFC 4648 base64");
    }
    let final_value = base64_value(bytes[data_len - 1])
        .context("patch replacement is not canonical RFC 4648 base64")?;
    if (padding == 1 && final_value & 0b11 != 0) || (padding == 2 && final_value & 0b1111 != 0) {
        bail!("patch replacement is not canonical RFC 4648 base64");
    }

    let full_bytes = (bytes.len() / 4)
        .checked_mul(3)
        .context("decoded replacement byte count exceeds usize capacity")?;
    full_bytes
        .checked_sub(padding)
        .context("invalid RFC 4648 base64 padding")
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

struct LimitWriter {
    written: usize,
    limit: usize,
}

impl Write for LimitWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self
            .written
            .checked_add(buffer.len())
            .ok_or_else(|| io::Error::other("report byte count exceeds usize capacity"))?;
        if next > self.limit {
            return Err(io::Error::other(format!(
                "report bytes limit exceeded: observed at least {next}, limit {}",
                self.limit
            )));
        }
        self.written = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use stratadiff_core::{
        AmbiguityConstraint, AmbiguityGroup, AmbiguityPair, Artifact, ByteEdit, ChangeKind,
        Correspondence, DiffReport, Language, LosslessPatch, NodeRef, PATCH_ALGORITHM,
        ParserManifest, Position, Predicate, Relation, ReplayCertificate, Span, StructuralChange,
        Summary,
    };

    use super::{VerificationLimits, WorkBudget, checked_add, preflight_report};

    #[test]
    fn every_preflight_limit_accepts_its_boundary() {
        let report = sample_report();
        let limits = exact_limits();
        let stats = preflight_report(&report, 1, b"x", b"y", &limits).unwrap();

        assert_eq!(stats.report_bytes, 1);
        assert_eq!(stats.before_source_bytes, 1);
        assert_eq!(stats.after_source_bytes, 1);
        assert_eq!(stats.relations, 1);
        assert_eq!(stats.ambiguity_groups, 1);
        assert_eq!(stats.ambiguity_endpoints, 2);
        assert_eq!(stats.ambiguity_pairs, 1);
        assert_eq!(stats.changes, 1);
        assert_eq!(stats.patch_edits, 1);
        assert_eq!(stats.decoded_replacement_bytes, 1);
        assert_eq!(stats.syntax_nodes, 2);
        assert_eq!(stats.verification_work, 0);
    }

    #[test]
    fn every_preflight_limit_rejects_boundary_plus_one() {
        assert_limit_error(
            |limits| limits.max_report_bytes = 0,
            "report bytes limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_source_bytes = 0,
            "before source bytes limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_relations = 0,
            "relations limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_ambiguity_groups = 0,
            "ambiguity groups limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_ambiguity_endpoints = 1,
            "ambiguity endpoints limit exceeded: observed 2, limit 1",
        );
        assert_limit_error(
            |limits| limits.max_ambiguity_pairs = 0,
            "ambiguity possible pairs limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_changes = 0,
            "changes limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_patch_edits = 0,
            "patch edits limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_decoded_replacement_bytes = 0,
            "decoded replacement bytes limit exceeded: observed 1, limit 0",
        );
        assert_limit_error(
            |limits| limits.max_syntax_nodes = 1,
            "syntax nodes limit exceeded: observed 2, limit 1",
        );
    }

    #[test]
    fn runtime_work_budget_checks_addition_and_reports_context() {
        let mut budget = WorkBudget::new(8);
        budget.charge(3, "first operation").unwrap();
        budget.charge_product(1, 5, "bounded DP cells").unwrap();
        assert_eq!(budget.used(), 8);

        let error = budget.charge(1, "recursive equality").unwrap_err();
        assert_eq!(
            error.to_string(),
            "verification work limit exceeded: observed 9, limit 8, while recursive equality"
        );
    }

    #[test]
    fn runtime_work_budget_rejects_arithmetic_overflow() {
        let mut addition = WorkBudget::new(usize::MAX);
        addition.charge(usize::MAX, "first operation").unwrap();
        let error = addition.charge(1, "overflowing addition").unwrap_err();
        assert_eq!(
            error.to_string(),
            "verification work exceeds usize capacity while overflowing addition"
        );

        let mut multiplication = WorkBudget::new(usize::MAX);
        let error = multiplication
            .charge_product(usize::MAX, 2, "overflowing multiplication")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "verification work exceeds usize capacity while overflowing multiplication"
        );
    }

    #[test]
    fn count_overflow_has_a_specific_diagnostic() {
        let error = checked_add("ambiguity endpoint count", usize::MAX, 1).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ambiguity endpoint count exceeds usize capacity"
        );
    }

    fn assert_limit_error(update: impl FnOnce(&mut VerificationLimits), expected: &str) {
        let report = sample_report();
        let mut limits = exact_limits();
        update(&mut limits);
        let error = preflight_report(&report, 1, b"x", b"y", &limits).unwrap_err();
        assert_eq!(error.to_string(), expected);
    }

    fn exact_limits() -> VerificationLimits {
        VerificationLimits {
            max_report_bytes: 1,
            max_source_bytes: 1,
            max_relations: 1,
            max_ambiguity_groups: 1,
            max_ambiguity_endpoints: 2,
            max_ambiguity_pairs: 1,
            max_changes: 1,
            max_patch_edits: 1,
            max_decoded_replacement_bytes: 1,
            max_syntax_nodes: 2,
            max_verification_work: 9,
            ..VerificationLimits::default()
        }
    }

    fn sample_report() -> DiffReport {
        DiffReport {
            schema: "schema".to_owned(),
            engine_version: "engine".to_owned(),
            before: Artifact {
                path: "before.py".to_owned(),
                byte_len: 1,
                blake3: "0".repeat(64),
            },
            after: Artifact {
                path: "after.py".to_owned(),
                byte_len: 1,
                blake3: "0".repeat(64),
            },
            parser: ParserManifest {
                engine: "tree-sitter".to_owned(),
                runtime_version: "runtime".to_owned(),
                language: Language::Python,
                grammar_name: "grammar".to_owned(),
                grammar_version: "version".to_owned(),
                grammar_abi: 0,
                node_types_blake3: "0".repeat(64),
                coordinate_unit: "unit".to_owned(),
                root_kind: "module".to_owned(),
                before_nodes: 1,
                after_nodes: 1,
                error_free: true,
            },
            relations: vec![Relation {
                before: node(0),
                after: node(0),
                predicate: Predicate::InputPair,
                correspondence: Correspondence::InputPair,
                evidence: vec!["evidence".to_owned()],
            }],
            ambiguities: vec![AmbiguityGroup {
                parent_before: 0,
                parent_after: 0,
                before: vec![node(1)],
                after: vec![node(1)],
                constraint: AmbiguityConstraint::ExactOrderedAlignment {
                    predicate: Predicate::ShapeEqual,
                    required_matches: 1,
                    possible_pairs: vec![AmbiguityPair {
                        before_id: 1,
                        after_id: 1,
                    }],
                },
                reason: "test".to_owned(),
            }],
            changes: vec![StructuralChange {
                kind: ChangeKind::FormattingOnly,
                before: Some(node(0)),
                after: Some(node(0)),
                detail: "test".to_owned(),
            }],
            patch: LosslessPatch {
                algorithm: PATCH_ALGORITHM.to_owned(),
                edits: vec![ByteEdit {
                    old_start: 0,
                    old_end: 1,
                    replacement_base64: "eQ==".to_owned(),
                }],
            },
            certificate: ReplayCertificate {
                before_blake3: "0".repeat(64),
                after_blake3: "0".repeat(64),
                reconstructed_blake3: "0".repeat(64),
                before_len: 1,
                after_len: 1,
                patch_verified: true,
            },
            summary: Summary {
                model_forced_relations: 0,
                suggested_relations: 0,
                ambiguity_groups: 1,
                structural_changes: 1,
            },
        }
    }

    fn node(id: usize) -> NodeRef {
        NodeRef {
            id,
            kind: "identifier".to_owned(),
            named: true,
            extra: false,
            missing: false,
            field: None,
            span: Span {
                start_byte: 0,
                end_byte: 1,
                start: Position { row: 0, column: 0 },
                end: Position { row: 0, column: 1 },
            },
            subtree_size: 1,
            syntax_hash: "0".repeat(64),
            shape_hash: "0".repeat(64),
        }
    }
}
