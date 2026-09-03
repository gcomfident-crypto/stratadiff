pub mod diffbenchmark;
pub mod diffbenchmark_eval;
mod language;
mod matcher;
mod model;
mod patch;
mod syntax;
mod verifier;

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
use syntax::parse;
use verifier::REPORT_SCHEMA;

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
        schema: REPORT_SCHEMA.to_owned(),
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
    verifier::verify_report(report, before, after)
}
