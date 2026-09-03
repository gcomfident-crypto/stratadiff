mod language;
mod matcher;
mod model;
mod patch;
mod syntax;

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};

pub use language::Language;
pub use model::{
    AmbiguityGroup, Artifact, ByteEdit, ChangeKind, Correspondence, DiffReport, LosslessPatch,
    NodeRef, ParserManifest, Position, Predicate, Relation, ReplayCertificate, Span,
    StructuralChange, Summary,
};
pub use patch::apply_patch;

use matcher::match_trees;
use patch::{create_certificate, create_patch};
use syntax::{parse, shape_equal, syntax_equal};

pub fn analyze_files(
    before_path: &Path,
    after_path: &Path,
    language: Option<Language>,
) -> Result<DiffReport> {
    let selected_language = match language {
        Some(value) => value,
        None => {
            let before_language = Language::detect(before_path)?;
            let after_language = Language::detect(after_path)?;
            if before_language != after_language {
                bail!(
                    "input languages differ ({before_language:?} and {after_language:?}); pass --language only when both files use the same grammar"
                );
            }
            before_language
        }
    };
    let before = std::fs::read(before_path)
        .with_context(|| format!("failed to read {}", before_path.display()))?;
    let after = std::fs::read(after_path)
        .with_context(|| format!("failed to read {}", after_path.display()))?;
    analyze_bytes(
        before,
        after,
        before_path.to_string_lossy().into_owned(),
        after_path.to_string_lossy().into_owned(),
        selected_language,
    )
}

pub fn analyze_bytes(
    before: Vec<u8>,
    after: Vec<u8>,
    before_label: String,
    after_label: String,
    language: Language,
) -> Result<DiffReport> {
    let parsed_before = parse(before, language)?;
    let parsed_after = parse(after, language)?;
    if parsed_before.root_kind != parsed_after.root_kind {
        bail!(
            "parser roots differ: {} versus {}",
            parsed_before.root_kind,
            parsed_after.root_kind
        );
    }

    let matched = match_trees(&parsed_before, &parsed_after);
    let patch = create_patch(&parsed_before.source, &parsed_after.source);
    let certificate = create_certificate(&parsed_before.source, &parsed_after.source, &patch)?;
    let model_forced_relations = matched
        .relations
        .iter()
        .filter(|relation| relation.correspondence == Correspondence::ModelForced)
        .count();
    let suggested_relations = matched
        .relations
        .iter()
        .filter(|relation| relation.correspondence == Correspondence::Suggested)
        .count();

    Ok(DiffReport {
        schema: "https://stratadiff.dev/schema/report-v1.json".to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        before: Artifact {
            path: before_label,
            byte_len: parsed_before.source.len(),
            blake3: blake3::hash(&parsed_before.source).to_hex().to_string(),
        },
        after: Artifact {
            path: after_label,
            byte_len: parsed_after.source.len(),
            blake3: blake3::hash(&parsed_after.source).to_hex().to_string(),
        },
        parser: ParserManifest {
            engine: "tree-sitter".to_owned(),
            runtime_version: "0.27.0".to_owned(),
            language,
            grammar_name: language.grammar_name().to_owned(),
            grammar_version: language.grammar_version().to_owned(),
            grammar_abi: language.parser_language().abi_version(),
            node_types_blake3: blake3::hash(language.node_types().as_bytes())
                .to_hex()
                .to_string(),
            coordinate_unit: "zero_based_row_utf8_byte_column".to_owned(),
            root_kind: parsed_before.root_kind.clone(),
            before_nodes: parsed_before.nodes.len(),
            after_nodes: parsed_after.nodes.len(),
            error_free: true,
        },
        summary: Summary {
            model_forced_relations,
            suggested_relations,
            ambiguity_groups: matched.ambiguities.len(),
            structural_changes: matched.changes.len(),
        },
        relations: matched.relations,
        ambiguities: matched.ambiguities,
        changes: matched.changes,
        patch,
        certificate,
    })
}

pub fn verify_report(report: &DiffReport, before: &[u8], after: &[u8]) -> Result<()> {
    verify_artifact("before", &report.before, before)?;
    verify_artifact("after", &report.after, after)?;
    if !report.certificate.patch_verified {
        bail!("report does not carry a successful replay certificate");
    }
    if report.certificate.before_len != before.len()
        || report.certificate.after_len != after.len()
        || report.certificate.before_blake3 != report.before.blake3
        || report.certificate.after_blake3 != report.after.blake3
    {
        bail!("certificate metadata does not match the input artifacts");
    }
    let reconstructed = apply_patch(before, &report.patch)?;
    if reconstructed != after {
        bail!("patch replay differs from the supplied after snapshot");
    }
    let reconstructed_hash = blake3::hash(&reconstructed).to_hex().to_string();
    if reconstructed_hash != report.certificate.reconstructed_blake3
        || reconstructed_hash != report.certificate.after_blake3
    {
        bail!("reconstructed bytes do not match the certificate hashes");
    }

    let parsed_before = parse(before.to_vec(), report.parser.language)?;
    let parsed_after = parse(after.to_vec(), report.parser.language)?;
    if report.parser.engine != "tree-sitter"
        || report.parser.runtime_version != "0.27.0"
        || report.parser.grammar_name != report.parser.language.grammar_name()
        || report.parser.grammar_version != report.parser.language.grammar_version()
        || report.parser.grammar_abi != report.parser.language.parser_language().abi_version()
        || report.parser.node_types_blake3
            != blake3::hash(report.parser.language.node_types().as_bytes())
                .to_hex()
                .to_string()
        || report.parser.coordinate_unit != "zero_based_row_utf8_byte_column"
        || report.parser.root_kind != parsed_before.root_kind
        || report.parser.root_kind != parsed_after.root_kind
        || report.parser.before_nodes != parsed_before.nodes.len()
        || report.parser.after_nodes != parsed_after.nodes.len()
        || !report.parser.error_free
    {
        bail!("parser manifest does not match a fresh parse");
    }
    let mut seen_before = HashSet::new();
    let mut seen_after = HashSet::new();
    for relation in &report.relations {
        let before_node = parsed_before
            .nodes
            .get(relation.before.id)
            .context("relation references an unknown before node")?;
        let after_node = parsed_after
            .nodes
            .get(relation.after.id)
            .context("relation references an unknown after node")?;
        if before_node.as_ref() != relation.before || after_node.as_ref() != relation.after {
            bail!("relation node metadata does not match a fresh parse");
        }
        if !seen_before.insert(relation.before.id) || !seen_after.insert(relation.after.id) {
            bail!("relations violate one-to-one correspondence");
        }
        let predicate_holds = match relation.predicate {
            Predicate::InputPair => {
                relation.before.id == parsed_before.root && relation.after.id == parsed_after.root
            }
            Predicate::ByteEqual => {
                before_node.byte_hash == after_node.byte_hash
                    && parsed_before.source[before_node.span.start_byte..before_node.span.end_byte]
                        == parsed_after.source[after_node.span.start_byte..after_node.span.end_byte]
            }
            Predicate::SyntaxEqual => syntax_equal(
                &parsed_before,
                relation.before.id,
                &parsed_after,
                relation.after.id,
            ),
            Predicate::ShapeEqual => shape_equal(
                &parsed_before,
                relation.before.id,
                &parsed_after,
                relation.after.id,
            ),
        };
        if !predicate_holds {
            bail!(
                "certified predicate {:?} is false for relation {} -> {}",
                relation.predicate,
                relation.before.id,
                relation.after.id
            );
        }
    }

    let matched = match_trees(&parsed_before, &parsed_after);
    if report.relations != matched.relations
        || report.ambiguities != matched.ambiguities
        || report.changes != matched.changes
    {
        bail!("structural claims do not match a deterministic fresh analysis");
    }
    let expected_summary = Summary {
        model_forced_relations: matched
            .relations
            .iter()
            .filter(|relation| relation.correspondence == Correspondence::ModelForced)
            .count(),
        suggested_relations: matched
            .relations
            .iter()
            .filter(|relation| relation.correspondence == Correspondence::Suggested)
            .count(),
        ambiguity_groups: matched.ambiguities.len(),
        structural_changes: matched.changes.len(),
    };
    if report.summary != expected_summary {
        bail!("summary does not match the verified structural claims");
    }
    Ok(())
}

fn verify_artifact(side: &str, artifact: &Artifact, bytes: &[u8]) -> Result<()> {
    if artifact.byte_len != bytes.len() {
        bail!("{side} byte length does not match the report");
    }
    if artifact.blake3 != blake3::hash(bytes).to_hex().to_string() {
        bail!("{side} BLAKE3 digest does not match the report");
    }
    Ok(())
}
