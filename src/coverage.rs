use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    codeowners::{CodeownerIdentity, CodeownerRuleMatch, CodeownersPolicy, CodeownersSource},
    ledger::{GithubReviewLedger, GithubReviewReceipt},
    ownership::{GithubOwnershipIndex, GithubOwnershipSnapshot},
    review::{
        CheckpointCarryBasis, CheckpointState, PathEncoding, ReviewAnalysisBudgetExceeded,
        ReviewAnalysisContext, ReviewChangeIdentity, ReviewDelta, review_git_range_with_analysis,
        review_git_resume_delta_with_analysis,
    },
};

pub const REVIEW_COVERAGE_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-coverage-v1.schema.json";
pub const MAX_REVIEW_COVERAGE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_REVIEW_COVERAGE_REQUIREMENTS: usize = 1_000;
pub const MAX_REVIEW_COVERAGE_CHECKPOINTS: usize = 64;
pub const MAX_REVIEW_COVERAGE_PROOF_ENTRIES: usize = 6_000;
pub const MAX_REVIEW_COVERAGE_FILE_VISITS: usize = 6_000;
pub const MAX_REVIEW_COVERAGE_PROOF_SOURCE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_REVIEW_COVERAGE_OWNER_CELLS: usize = 10_000;
pub const MAX_REVIEW_COVERAGE_EXPANDED_CELLS: usize = 50_000;
pub const MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS: usize = 10_000;
const PASSPORT_ATTESTATION_DOMAIN: &[u8] = b"stratadiff-review-coverage-passport-v1\0";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewCoveragePassport {
    pub schema: String,
    pub body: ReviewCoverageBody,
    pub attestation: ReviewCoverageAttestation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewCoverageBody {
    pub engine_version: String,
    pub protected_base_commit: String,
    pub merge_base_commit: String,
    pub head_commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codeowners_source: Option<CodeownersSource>,
    pub ledger: GithubReviewLedger,
    pub ownership: GithubOwnershipSnapshot,
    pub checkpoint_proofs: Vec<CheckpointCoverageProof>,
    pub files: Vec<FileCoverage>,
    pub unresolved_residue: Vec<UnresolvedCoverage>,
    pub summary: ReviewCoverageSummary,
    pub non_claims: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewCoverageAttestation {
    pub algorithm: String,
    pub key_id: String,
    pub body_sha256: String,
    pub signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckpointCoverageProof {
    pub checkpoint_commit: String,
    pub review_ids: Vec<u64>,
    pub reviewer_ids: Vec<u64>,
    pub result: CheckpointCoverageProofResult,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckpointCoverageProofResult {
    Verified {
        carried_changes: Vec<CarriedChange>,
        review_delta: Box<ReviewDelta>,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CarriedChange {
    pub change: ReviewChangeIdentity,
    pub basis: CheckpointCarryBasis,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileCoverageState {
    Covered,
    NeedsReview,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageFileScope {
    CurrentChange,
    RetiredResidue,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileCoverage {
    pub scope: CoverageFileScope,
    pub change: ReviewChangeIdentity,
    pub path: String,
    pub path_encoding: PathEncoding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_rule: Option<CodeownerRuleMatch>,
    pub owner_alternatives: Vec<OwnerCoverage>,
    pub state: FileCoverageState,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedCoverage {
    pub checkpoint_commit: String,
    pub path: String,
    pub path_encoding: PathEncoding,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerCoverage {
    pub owner: CodeownerIdentity,
    pub eligible_reviewer_ids: Vec<u64>,
    pub active_review_ids: Vec<u64>,
    pub covering_review_ids: Vec<u64>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewCoverageSummary {
    pub current_files: usize,
    pub retired_residue_files: usize,
    pub unresolved_residue: usize,
    pub total_requirements: usize,
    pub covered_files: usize,
    pub needs_review_files: usize,
    pub blocked_files: usize,
    pub active_review_receipts: usize,
    pub unique_checkpoint_proofs: usize,
    pub gate_passed: bool,
}

pub fn build_review_coverage_passport(
    repository: &Path,
    protected_base_commit: &str,
    head_commit: &str,
    ledger: GithubReviewLedger,
    ownership: GithubOwnershipSnapshot,
    receiver_signing_key: &[u8; 32],
) -> Result<ReviewCoveragePassport> {
    let body = compute_review_coverage_body(
        repository,
        protected_base_commit,
        head_commit,
        ledger,
        ownership,
    )?;
    let signing_key = SigningKey::from_bytes(receiver_signing_key);
    ensure!(
        encode_lower_hex(signing_key.verifying_key().as_bytes()) == body.ledger.receiver.public_key,
        "coverage signing key does not match the ledger receiver public key"
    );
    let encoded = serde_json::to_vec(&body)?;
    let signature = signing_key.sign(&attestation_message(&encoded));
    let passport = ReviewCoveragePassport {
        schema: REVIEW_COVERAGE_SCHEMA.to_owned(),
        attestation: ReviewCoverageAttestation {
            algorithm: "ed25519".to_owned(),
            key_id: body.ledger.receiver.key_id.clone(),
            body_sha256: hex_sha256(&encoded),
            signature: encode_lower_hex(&signature.to_bytes()),
        },
        body,
    };
    verify_passport_attestation(&passport, &passport.body.ledger.receiver.public_key)?;
    Ok(passport)
}

pub fn verify_review_coverage_passport(
    repository: &Path,
    passport: &ReviewCoveragePassport,
    trusted_receiver_public_key: &str,
) -> Result<()> {
    ensure!(
        passport.schema == REVIEW_COVERAGE_SCHEMA,
        "unsupported review coverage passport schema"
    );
    verify_passport_attestation(passport, trusted_receiver_public_key)?;
    let recomputed = compute_review_coverage_body(
        repository,
        &passport.body.protected_base_commit,
        &passport.body.head_commit,
        passport.body.ledger.clone(),
        passport.body.ownership.clone(),
    )?;
    ensure!(
        recomputed == passport.body,
        "review coverage passport does not match the exact Git objects and embedded policy facts"
    );
    Ok(())
}

fn compute_review_coverage_body(
    repository: &Path,
    protected_base_commit: &str,
    head_commit: &str,
    ledger: GithubReviewLedger,
    ownership: GithubOwnershipSnapshot,
) -> Result<ReviewCoverageBody> {
    ensure!(
        is_object_id(protected_base_commit),
        "protected base must be a full lowercase Git object ID"
    );
    ensure!(
        is_object_id(head_commit),
        "head must be a full lowercase Git object ID"
    );
    let effective_pull_request =
        ledger.reconcile_current_head(protected_base_commit, head_commit)?;
    let protected_base_commit = effective_pull_request.base_commit.as_str();
    let head_commit = effective_pull_request.head_commit.as_str();
    let ownership_index = ownership
        .index()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        ledger.provider_url == ownership.provider_url,
        "ledger and ownership snapshot provider URLs differ"
    );
    ensure!(
        ledger.repository.id == ownership.repository_id,
        "ledger and ownership snapshot repository IDs differ"
    );
    ensure!(
        ownership.base_commit == protected_base_commit,
        "ownership snapshot is not bound to the requested protected base"
    );

    let mut analysis = ReviewAnalysisContext::bounded(
        MAX_REVIEW_COVERAGE_FILE_VISITS,
        MAX_REVIEW_COVERAGE_PROOF_SOURCE_BYTES,
    );
    let current = review_git_range_with_analysis(
        repository,
        protected_base_commit,
        head_commit,
        None,
        &mut analysis,
    )?;
    ensure!(
        current.head_commit == head_commit,
        "resolved head differs from the requested exact head"
    );
    let mut requirement_count = checked_requirement_count(0, current.files.len())?;
    let policy_result = CodeownersPolicy::load(repository, protected_base_commit);
    let codeowners_source = policy_result
        .as_ref()
        .ok()
        .map(|policy| policy.source().clone());
    let policy_error = policy_result.as_ref().err().map(ToString::to_string);

    let active_receipts = ledger.active_receipts();
    let checkpoint_proofs = build_checkpoint_proofs(
        repository,
        protected_base_commit,
        head_commit,
        &active_receipts,
        &mut analysis,
    )?;
    let proof_by_checkpoint = checkpoint_proofs
        .iter()
        .map(|proof| (proof.checkpoint_commit.as_str(), proof))
        .collect::<BTreeMap<_, _>>();
    let receipt_by_reviewer = active_receipts
        .iter()
        .map(|receipt| (receipt.reviewer_id, *receipt))
        .collect::<BTreeMap<_, _>>();

    let current_identities = current
        .files
        .iter()
        .map(|file| file.change_identity())
        .collect::<BTreeSet<_>>();
    let current_paths = current
        .files
        .iter()
        .filter_map(|file| {
            file.ownership_path()
                .map(|(path, encoding)| (path.to_owned(), encoding))
        })
        .collect::<BTreeSet<_>>();
    let mut retired_paths = BTreeMap::<(String, PathEncoding), ReviewChangeIdentity>::new();
    let mut unresolved_residue = BTreeSet::new();
    for proof in &checkpoint_proofs {
        let CheckpointCoverageProofResult::Verified { review_delta, .. } = &proof.result else {
            continue;
        };
        for entry in &review_delta.entries {
            let identity = entry.file.change_identity();
            if current_identities.contains(&identity) {
                continue;
            }
            if let Some((path, encoding)) = entry.file.ownership_path()
                && !current_paths.contains(&(path.to_owned(), encoding))
            {
                let key = (path.to_owned(), encoding);
                if let std::collections::btree_map::Entry::Vacant(entry) = retired_paths.entry(key)
                {
                    requirement_count = checked_requirement_count(requirement_count, 1)?;
                    entry.insert(identity);
                }
            }
        }
        for unresolved in &review_delta.unresolved_retired_changes {
            if current_paths.contains(&(unresolved.path.clone(), unresolved.path_encoding)) {
                continue;
            }
            let unresolved = UnresolvedCoverage {
                checkpoint_commit: proof.checkpoint_commit.clone(),
                path: unresolved.path.clone(),
                path_encoding: unresolved.path_encoding,
                reason: format!("{:?}", unresolved.reason).to_ascii_lowercase(),
            };
            if !unresolved_residue.contains(&unresolved) {
                requirement_count = checked_requirement_count(requirement_count, 1)?;
                unresolved_residue.insert(unresolved);
            }
        }
    }
    let retired_residue_files = retired_paths.len();
    preflight_owner_cells(
        policy_result.as_ref().ok(),
        current
            .files
            .iter()
            .filter_map(|file| file.ownership_path())
            .chain(
                retired_paths
                    .keys()
                    .map(|(path, encoding)| (path.as_str(), *encoding)),
            ),
    )?;

    let mut resolution_cache = BTreeMap::new();
    let mut expansion_budget = CoverageExpansionBudget::default();
    let mut files = current
        .files
        .iter()
        .map(|file| {
            evaluate_file_coverage(
                file,
                policy_result.as_ref().ok(),
                policy_error.as_deref(),
                &ownership_index,
                &mut resolution_cache,
                &mut expansion_budget,
                &receipt_by_reviewer,
                &proof_by_checkpoint,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for ((path, encoding), change) in retired_paths {
        files.push(evaluate_change_coverage(
            change,
            &path,
            encoding,
            CoverageFileScope::RetiredResidue,
            policy_result.as_ref().ok(),
            policy_error.as_deref(),
            &ownership_index,
            &mut resolution_cache,
            &mut expansion_budget,
            &receipt_by_reviewer,
            &proof_by_checkpoint,
        )?);
    }
    files.sort_by(|left, right| {
        (left.path.as_str(), scope_rank(left.scope), &left.change).cmp(&(
            right.path.as_str(),
            scope_rank(right.scope),
            &right.change,
        ))
    });

    let covered_files = files
        .iter()
        .filter(|file| file.state == FileCoverageState::Covered)
        .count();
    let needs_review_files = files
        .iter()
        .filter(|file| file.state == FileCoverageState::NeedsReview)
        .count();
    let blocked_matrix_files = files
        .iter()
        .filter(|file| file.state == FileCoverageState::Blocked)
        .count();
    let unresolved_residue = unresolved_residue.into_iter().collect::<Vec<_>>();
    let blocked_files = blocked_matrix_files
        .checked_add(unresolved_residue.len())
        .context("review coverage blocked requirement count overflow")?;
    let total_requirements = files
        .len()
        .checked_add(unresolved_residue.len())
        .context("review coverage requirement count overflow")?;
    ensure!(
        total_requirements == requirement_count,
        "review coverage requirement accounting mismatch"
    );
    let summary = ReviewCoverageSummary {
        current_files: current.files.len(),
        retired_residue_files,
        unresolved_residue: unresolved_residue.len(),
        total_requirements,
        covered_files,
        needs_review_files,
        blocked_files,
        active_review_receipts: active_receipts.len(),
        unique_checkpoint_proofs: checkpoint_proofs.len(),
        gate_passed: covered_files == total_requirements,
    };

    Ok(ReviewCoverageBody {
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        protected_base_commit: protected_base_commit.to_owned(),
        merge_base_commit: current.base_commit,
        head_commit: current.head_commit,
        codeowners_source,
        ledger,
        ownership,
        checkpoint_proofs,
        files,
        unresolved_residue,
        summary,
        non_claims: vec![
            "coverage does not restore or manufacture a GitHub approval".to_owned(),
            "byte-level carry evidence does not prove semantic safety or merge safety".to_owned(),
            "GitHub webhook HMAC is receiver-verifiable; the Ed25519 signature is the receiver's attestation, not GitHub's signature".to_owned(),
            "the authoritative base and head are caller-provided exact object IDs; this passport does not authenticate their provider freshness".to_owned(),
            "the signed ledger snapshot proves body integrity, not freshness; an entire older valid ledger can be replayed unless a trusted external latest revision or root is enforced".to_owned(),
        ],
    })
}

fn checked_requirement_count(current: usize, additional: usize) -> Result<usize> {
    let observed = current
        .checked_add(additional)
        .context("review coverage requirement count overflow")?;
    ensure!(
        observed <= MAX_REVIEW_COVERAGE_REQUIREMENTS,
        "review coverage requirement limit exceeded: observed at least {observed}, limit {MAX_REVIEW_COVERAGE_REQUIREMENTS}"
    );
    Ok(observed)
}

fn build_checkpoint_proofs(
    repository: &Path,
    protected_base_commit: &str,
    head_commit: &str,
    active_receipts: &[&GithubReviewReceipt],
    analysis: &mut ReviewAnalysisContext,
) -> Result<Vec<CheckpointCoverageProof>> {
    let mut grouped = BTreeMap::<String, Vec<&GithubReviewReceipt>>::new();
    for receipt in active_receipts {
        grouped
            .entry(receipt.commit_id.clone())
            .or_default()
            .push(receipt);
    }
    ensure_checkpoint_count(grouped.len())?;

    let mut proofs = Vec::with_capacity(grouped.len());
    let mut proof_entry_count = 0;
    for (checkpoint_commit, mut receipts) in grouped {
        receipts.sort_by_key(|receipt| receipt.review_id);
        let review_ids = receipts.iter().map(|receipt| receipt.review_id).collect();
        let mut reviewer_ids = receipts
            .iter()
            .map(|receipt| receipt.reviewer_id)
            .collect::<Vec<_>>();
        reviewer_ids.sort_unstable();
        reviewer_ids.dedup();
        let result = match review_git_range_with_analysis(
            repository,
            protected_base_commit,
            head_commit,
            Some(&checkpoint_commit),
            analysis,
        )
        .and_then(|review| {
            let mut carried_changes = review
                .files
                .iter()
                .filter(|file| {
                    file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint)
                })
                .map(|file| -> Result<CarriedChange> {
                    Ok(CarriedChange {
                        change: file.change_identity(),
                        basis: file
                            .checkpoint_match_basis
                            .context("carried review file is missing its checkpoint match basis")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            carried_changes.sort_by(|left, right| left.change.cmp(&right.change));
            let review_delta =
                review_git_resume_delta_with_analysis(repository, &review, analysis)?;
            Ok((carried_changes, review_delta))
        }) {
            Ok((carried_changes, review_delta)) => CheckpointCoverageProofResult::Verified {
                carried_changes,
                review_delta: Box::new(review_delta),
            },
            Err(error) => {
                if error
                    .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                    .is_some()
                {
                    return Err(error);
                }
                CheckpointCoverageProofResult::Unavailable {
                    reason: format!("{error:#}"),
                }
            }
        };
        proof_entry_count =
            checked_proof_entry_count(proof_entry_count, retained_proof_entries(&result)?)?;
        proofs.push(CheckpointCoverageProof {
            checkpoint_commit,
            review_ids,
            reviewer_ids,
            result,
        });
    }
    Ok(proofs)
}

fn ensure_checkpoint_count(observed: usize) -> Result<()> {
    ensure!(
        observed <= MAX_REVIEW_COVERAGE_CHECKPOINTS,
        "review coverage checkpoint proof limit exceeded: observed {observed}, limit {MAX_REVIEW_COVERAGE_CHECKPOINTS}"
    );
    Ok(())
}

fn retained_proof_entries(result: &CheckpointCoverageProofResult) -> Result<usize> {
    let CheckpointCoverageProofResult::Verified {
        carried_changes,
        review_delta,
    } = result
    else {
        return Ok(0);
    };
    carried_changes
        .len()
        .checked_add(review_delta.entries.len())
        .and_then(|count| count.checked_add(review_delta.unresolved_retired_changes.len()))
        .context("review coverage proof entry count overflow")
}

fn checked_proof_entry_count(current: usize, additional: usize) -> Result<usize> {
    let observed = current
        .checked_add(additional)
        .context("review coverage proof entry count overflow")?;
    ensure!(
        observed <= MAX_REVIEW_COVERAGE_PROOF_ENTRIES,
        "review coverage proof entry limit exceeded: observed at least {observed}, limit {MAX_REVIEW_COVERAGE_PROOF_ENTRIES}"
    );
    Ok(observed)
}

fn preflight_owner_cells<'a>(
    policy: Option<&CodeownersPolicy>,
    paths: impl IntoIterator<Item = (&'a str, PathEncoding)>,
) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    let mut owner_cells = 0_usize;
    for (path, encoding) in paths {
        if encoding != PathEncoding::Utf8 {
            continue;
        }
        let Ok(Some(additional)) = policy.matching_owner_count(path) else {
            continue;
        };
        owner_cells = checked_owner_cell_count(owner_cells, additional)?;
    }
    Ok(())
}

fn checked_owner_cell_count(current: usize, additional: usize) -> Result<usize> {
    let observed = current
        .checked_add(additional)
        .context("review coverage owner cell count overflow")?;
    ensure!(
        observed <= MAX_REVIEW_COVERAGE_OWNER_CELLS,
        "review coverage owner cell limit exceeded: observed at least {observed}, limit {MAX_REVIEW_COVERAGE_OWNER_CELLS}"
    );
    Ok(observed)
}

#[derive(Default)]
struct CoverageExpansionBudget {
    expanded_cells: usize,
}

impl CoverageExpansionBudget {
    fn reserve_resolved_owner(&mut self, reviewer_count: usize) -> Result<()> {
        ensure_owner_result_count(reviewer_count, "eligible_reviewer_ids")?;
        let additional = reviewer_count
            .checked_mul(4)
            .context("review coverage expanded cell count overflow")?;
        self.reserve(additional)
    }

    fn reserve_resolution_blocker(&mut self) -> Result<()> {
        self.reserve(1)
    }

    fn reserve(&mut self, additional: usize) -> Result<()> {
        let observed = self
            .expanded_cells
            .checked_add(additional)
            .context("review coverage expanded cell count overflow")?;
        ensure!(
            observed <= MAX_REVIEW_COVERAGE_EXPANDED_CELLS,
            "review coverage expanded cell limit exceeded: observed at least {observed}, limit {MAX_REVIEW_COVERAGE_EXPANDED_CELLS}"
        );
        self.expanded_cells = observed;
        Ok(())
    }
}

fn ensure_owner_result_count(observed: usize, field: &str) -> Result<()> {
    ensure!(
        observed <= MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS,
        "review coverage {field} item limit exceeded: observed {observed}, limit {MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS}"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn evaluate_file_coverage(
    file: &crate::review::ReviewFile,
    policy: Option<&CodeownersPolicy>,
    policy_error: Option<&str>,
    ownership: &GithubOwnershipIndex<'_>,
    resolution_cache: &mut BTreeMap<CodeownerIdentity, Result<Vec<u64>, String>>,
    expansion_budget: &mut CoverageExpansionBudget,
    receipt_by_reviewer: &BTreeMap<u64, &GithubReviewReceipt>,
    proof_by_checkpoint: &BTreeMap<&str, &CheckpointCoverageProof>,
) -> Result<FileCoverage> {
    let change = file.change_identity();
    let Some((path, path_encoding)) = file.ownership_path() else {
        return Ok(blocked_file(
            change,
            "<unknown>",
            PathEncoding::Utf8,
            CoverageFileScope::CurrentChange,
            None,
            "Git change has no path",
        ));
    };
    evaluate_change_coverage(
        change,
        path,
        path_encoding,
        CoverageFileScope::CurrentChange,
        policy,
        policy_error,
        ownership,
        resolution_cache,
        expansion_budget,
        receipt_by_reviewer,
        proof_by_checkpoint,
    )
}

#[derive(Clone, Copy)]
enum CoverageProofMode {
    CurrentIdentity,
    RetiredPath,
}

#[allow(clippy::too_many_arguments)]
fn evaluate_change_coverage(
    change: ReviewChangeIdentity,
    path: &str,
    path_encoding: PathEncoding,
    scope: CoverageFileScope,
    policy: Option<&CodeownersPolicy>,
    policy_error: Option<&str>,
    ownership: &GithubOwnershipIndex<'_>,
    resolution_cache: &mut BTreeMap<CodeownerIdentity, Result<Vec<u64>, String>>,
    expansion_budget: &mut CoverageExpansionBudget,
    receipt_by_reviewer: &BTreeMap<u64, &GithubReviewReceipt>,
    proof_by_checkpoint: &BTreeMap<&str, &CheckpointCoverageProof>,
) -> Result<FileCoverage> {
    let proof_mode = match scope {
        CoverageFileScope::CurrentChange => CoverageProofMode::CurrentIdentity,
        CoverageFileScope::RetiredResidue => CoverageProofMode::RetiredPath,
    };
    if path_encoding != PathEncoding::Utf8 {
        return Ok(blocked_file(
            change,
            path,
            path_encoding,
            scope,
            None,
            "non-UTF-8 Git paths cannot be matched against CODEOWNERS",
        ));
    }
    let Some(policy) = policy else {
        return Ok(blocked_file(
            change,
            path,
            path_encoding,
            scope,
            None,
            policy_error.unwrap_or("CODEOWNERS policy is unavailable"),
        ));
    };
    let resolution = match policy.resolve_utf8_path(path) {
        Ok(resolution) => resolution,
        Err(error) => {
            return Ok(blocked_file(
                change,
                path,
                path_encoding,
                scope,
                None,
                &error.to_string(),
            ));
        }
    };
    let Some(rule) = resolution.matching_rule else {
        return Ok(blocked_file(
            change,
            path,
            path_encoding,
            scope,
            None,
            "no CODEOWNERS rule matches this review requirement",
        ));
    };
    if rule.owner_alternatives.is_empty() {
        return Ok(blocked_file(
            change,
            path,
            path_encoding,
            scope,
            Some(rule),
            "the winning CODEOWNERS rule explicitly leaves this file unowned",
        ));
    }

    let mut alternatives = Vec::with_capacity(rule.owner_alternatives.len());
    let mut any_covering_receipt = false;
    let mut any_blocker = false;
    for owner in &rule.owner_alternatives {
        let resolution = resolution_cache
            .entry(owner.clone())
            .or_insert_with(|| ownership.resolve(owner).map_err(|error| error.to_string()));
        let mut alternative = match resolution {
            Ok(reviewer_ids) => {
                expansion_budget.reserve_resolved_owner(reviewer_ids.len())?;
                let mut alternative = OwnerCoverage {
                    owner: owner.clone(),
                    eligible_reviewer_ids: reviewer_ids.clone(),
                    active_review_ids: Vec::with_capacity(reviewer_ids.len()),
                    covering_review_ids: Vec::with_capacity(reviewer_ids.len()),
                    blockers: Vec::with_capacity(reviewer_ids.len()),
                };
                for reviewer_id in &alternative.eligible_reviewer_ids {
                    let Some(receipt) = receipt_by_reviewer.get(reviewer_id) else {
                        continue;
                    };
                    let Some(user) = ownership.user(*reviewer_id) else {
                        alternative.blockers.push(format!(
                            "ownership snapshot is missing reviewer ID {reviewer_id}"
                        ));
                        continue;
                    };
                    if !user.login.eq_ignore_ascii_case(&receipt.reviewer_login) {
                        alternative.blockers.push(format!(
                            "review receipt {} login {} conflicts with ownership identity {}",
                            receipt.review_id, receipt.reviewer_login, user.login
                        ));
                        continue;
                    }
                    alternative.active_review_ids.push(receipt.review_id);
                    let Some(proof) = proof_by_checkpoint.get(receipt.commit_id.as_str()) else {
                        alternative.blockers.push(format!(
                            "review receipt {} has no checkpoint proof",
                            receipt.review_id
                        ));
                        continue;
                    };
                    match proof_covers(&proof.result, proof_mode, &change, path) {
                        Ok(true) => {
                            alternative.covering_review_ids.push(receipt.review_id);
                            any_covering_receipt = true;
                        }
                        Ok(false) => {}
                        Err(reason) => {
                            alternative.blockers.push(format!(
                                "review receipt {} checkpoint proof is unavailable: {reason}",
                                receipt.review_id
                            ));
                        }
                    }
                }
                alternative
            }
            Err(error) => {
                expansion_budget.reserve_resolution_blocker()?;
                OwnerCoverage {
                    owner: owner.clone(),
                    eligible_reviewer_ids: Vec::new(),
                    active_review_ids: Vec::new(),
                    covering_review_ids: Vec::new(),
                    blockers: vec![error.clone()],
                }
            }
        };
        alternative.active_review_ids.sort_unstable();
        alternative.covering_review_ids.sort_unstable();
        ensure_owner_result_count(
            alternative.eligible_reviewer_ids.len(),
            "eligible_reviewer_ids",
        )?;
        ensure_owner_result_count(alternative.active_review_ids.len(), "active_review_ids")?;
        ensure_owner_result_count(alternative.covering_review_ids.len(), "covering_review_ids")?;
        ensure_owner_result_count(alternative.blockers.len(), "blockers")?;
        if !alternative.blockers.is_empty() {
            any_blocker = true;
        }
        alternatives.push(alternative);
    }

    let (state, reason) = if any_covering_receipt {
        (
            FileCoverageState::Covered,
            "at least one authorized owner has an active receipt whose exact checkpoint proof carries this complete Git change identity".to_owned(),
        )
    } else if any_blocker {
        (
            FileCoverageState::Blocked,
            "ownership or checkpoint evidence is incomplete; coverage fails closed".to_owned(),
        )
    } else {
        (
            FileCoverageState::NeedsReview,
            "no authorized owner has an active review receipt carrying this complete Git change identity".to_owned(),
        )
    };
    Ok(FileCoverage {
        scope,
        change,
        path: path.to_owned(),
        path_encoding,
        matching_rule: Some(rule),
        owner_alternatives: alternatives,
        state,
        reason,
    })
}

fn proof_covers<'a>(
    result: &'a CheckpointCoverageProofResult,
    mode: CoverageProofMode,
    change: &ReviewChangeIdentity,
    path: &str,
) -> Result<bool, &'a str> {
    match result {
        CheckpointCoverageProofResult::Unavailable { reason } => Err(reason),
        CheckpointCoverageProofResult::Verified {
            carried_changes, ..
        } if matches!(mode, CoverageProofMode::CurrentIdentity) => Ok(carried_changes
            .iter()
            .any(|carried| &carried.change == change)),
        CheckpointCoverageProofResult::Verified { review_delta, .. } => {
            let displayable_residue = review_delta.entries.iter().any(|entry| {
                entry
                    .file
                    .ownership_path()
                    .is_some_and(|(entry_path, _)| entry_path == path)
            });
            let unresolved_residue = review_delta
                .unresolved_retired_changes
                .iter()
                .any(|entry| entry.path == path);
            Ok(!displayable_residue && !unresolved_residue)
        }
    }
}

fn scope_rank(scope: CoverageFileScope) -> u8 {
    match scope {
        CoverageFileScope::CurrentChange => 0,
        CoverageFileScope::RetiredResidue => 1,
    }
}

fn blocked_file(
    change: ReviewChangeIdentity,
    path: &str,
    path_encoding: PathEncoding,
    scope: CoverageFileScope,
    matching_rule: Option<CodeownerRuleMatch>,
    reason: &str,
) -> FileCoverage {
    FileCoverage {
        scope,
        change,
        path: path.to_owned(),
        path_encoding,
        matching_rule,
        owner_alternatives: Vec::new(),
        state: FileCoverageState::Blocked,
        reason: reason.to_owned(),
    }
}

fn verify_passport_attestation(
    passport: &ReviewCoveragePassport,
    trusted_receiver_public_key: &str,
) -> Result<()> {
    ensure!(
        passport.attestation.algorithm == "ed25519",
        "unsupported review coverage attestation algorithm"
    );
    ensure!(
        passport.attestation.key_id == passport.body.ledger.receiver.key_id,
        "review coverage attestation key ID differs from the ledger receiver"
    );
    ensure!(
        trusted_receiver_public_key == passport.body.ledger.receiver.public_key,
        "review coverage passport was not signed by the trusted receiver key"
    );
    let body = serde_json::to_vec(&passport.body)?;
    ensure!(
        passport.attestation.body_sha256 == hex_sha256(&body),
        "review coverage passport body digest mismatch"
    );
    let public_key = decode_lower_hex::<32>(trusted_receiver_public_key)
        .context("trusted receiver Ed25519 public key is invalid")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .context("trusted receiver Ed25519 public key is not valid")?;
    let signature_bytes = decode_lower_hex::<64>(&passport.attestation.signature)
        .context("review coverage Ed25519 signature is invalid")?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(&attestation_message(&body), &signature)
        .context("review coverage passport signature verification failed")
}

fn attestation_message(body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(PASSPORT_ATTESTATION_DOMAIN.len() + body.len());
    message.extend_from_slice(PASSPORT_ATTESTATION_DOMAIN);
    message.extend_from_slice(body);
    message
}

fn decode_lower_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    ensure!(value.len() == N * 2, "hex value has the wrong length");
    let mut decoded = [0_u8; N];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => bail!("hex value must use lowercase hexadecimal"),
    }
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn hex_sha256(bytes: &[u8]) -> String {
    encode_lower_hex(&Sha256::digest(bytes))
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use super::*;

    fn git(repository: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn global_requirement_budget_accepts_the_exact_limit() {
        let current = checked_requirement_count(0, 400).unwrap();
        let current_and_retired = checked_requirement_count(current, 350).unwrap();
        let total = checked_requirement_count(current_and_retired, 250).unwrap();

        assert_eq!(total, MAX_REVIEW_COVERAGE_REQUIREMENTS);
    }

    #[test]
    fn global_requirement_budget_rejects_one_over_the_limit() {
        let at_limit = checked_requirement_count(0, MAX_REVIEW_COVERAGE_REQUIREMENTS).unwrap();
        let error = checked_requirement_count(at_limit, 1).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("observed at least 1001, limit 1000")
        );
    }

    #[test]
    fn proof_work_budgets_accept_the_exact_limits() {
        ensure_checkpoint_count(MAX_REVIEW_COVERAGE_CHECKPOINTS).unwrap();
        assert_eq!(
            checked_proof_entry_count(0, MAX_REVIEW_COVERAGE_PROOF_ENTRIES).unwrap(),
            MAX_REVIEW_COVERAGE_PROOF_ENTRIES
        );
    }

    #[test]
    fn proof_work_budgets_reject_one_over_the_limits() {
        let checkpoint_error =
            ensure_checkpoint_count(MAX_REVIEW_COVERAGE_CHECKPOINTS + 1).unwrap_err();
        assert!(
            checkpoint_error
                .to_string()
                .contains("observed 65, limit 64")
        );

        let entry_error =
            checked_proof_entry_count(MAX_REVIEW_COVERAGE_PROOF_ENTRIES, 1).unwrap_err();
        assert!(
            entry_error
                .to_string()
                .contains("observed at least 6001, limit 6000")
        );
    }

    #[test]
    fn owner_cell_budget_accepts_exact_limit_and_rejects_one_more() {
        let at_limit = checked_owner_cell_count(0, MAX_REVIEW_COVERAGE_OWNER_CELLS).unwrap();
        assert_eq!(at_limit, MAX_REVIEW_COVERAGE_OWNER_CELLS);

        let error = checked_owner_cell_count(at_limit, 1).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("owner cell limit exceeded: observed at least 10001, limit 10000")
        );
    }

    #[test]
    fn expanded_owner_budget_accepts_exact_limit_and_rejects_one_more_owner() {
        let mut budget = CoverageExpansionBudget::default();
        budget.reserve_resolved_owner(10_000).unwrap();
        budget.reserve_resolved_owner(2_500).unwrap();
        assert_eq!(budget.expanded_cells, MAX_REVIEW_COVERAGE_EXPANDED_CELLS);

        let error = budget.reserve_resolved_owner(1).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("expanded cell limit exceeded: observed at least 50004, limit 50000")
        );
    }

    #[test]
    fn owner_result_budget_rejects_one_over_the_limit() {
        ensure_owner_result_count(MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS, "blockers").unwrap();
        let error =
            ensure_owner_result_count(MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS + 1, "blockers")
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("blockers item limit exceeded: observed 10001, limit 10000")
        );
    }

    #[test]
    fn checkpoint_limit_fails_before_git_analysis() {
        let receipts = (0..=MAX_REVIEW_COVERAGE_CHECKPOINTS)
            .map(|index| GithubReviewReceipt {
                review_id: index as u64 + 1,
                review_node_id: format!("PRR_{}", index + 1),
                reviewer_id: index as u64 + 1,
                reviewer_node_id: format!("U_{}", index + 1),
                reviewer_login: format!("reviewer-{}", index + 1),
                state: crate::ledger::CompletedReviewState::Approved,
                commit_id: format!("{index:040x}"),
                submitted_at: "2026-09-05T01:02:03Z".to_owned(),
                html_url: format!("https://example.test/reviews/{}", index + 1),
                author_association: "MEMBER".to_owned(),
                source_delivery_id: format!("delivery-{}", index + 1),
                payload_sha256: "0".repeat(64),
                receipt_sha256: "0".repeat(64),
            })
            .collect::<Vec<_>>();
        let receipt_refs = receipts.iter().collect::<Vec<_>>();
        let mut analysis = ReviewAnalysisContext::bounded(
            MAX_REVIEW_COVERAGE_FILE_VISITS,
            MAX_REVIEW_COVERAGE_PROOF_SOURCE_BYTES,
        );
        let error = build_checkpoint_proofs(
            Path::new("/path-that-must-not-be-read"),
            &"a".repeat(40),
            &"b".repeat(40),
            &receipt_refs,
            &mut analysis,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("checkpoint proof limit exceeded")
        );
    }

    #[test]
    fn checkpoint_budget_error_is_not_recorded_as_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.name", "StrataDiff Test"]);
        git(root, &["config", "user.email", "stratadiff@example.test"]);
        fs::write(root.join("reviewed.txt"), "before\n").unwrap();
        git(root, &["add", "--all"]);
        git(root, &["commit", "--quiet", "-m", "base"]);
        let base = git(root, &["rev-parse", "HEAD"]);
        fs::write(root.join("reviewed.txt"), "after\n").unwrap();
        git(root, &["add", "--all"]);
        git(root, &["commit", "--quiet", "-m", "checkpoint"]);
        let checkpoint = git(root, &["rev-parse", "HEAD"]);
        let receipt = GithubReviewReceipt {
            review_id: 1,
            review_node_id: "PRR_1".to_owned(),
            reviewer_id: 1,
            reviewer_node_id: "U_1".to_owned(),
            reviewer_login: "reviewer".to_owned(),
            state: crate::ledger::CompletedReviewState::Approved,
            commit_id: checkpoint.clone(),
            submitted_at: "2026-09-05T01:02:03Z".to_owned(),
            html_url: "https://example.test/reviews/1".to_owned(),
            author_association: "MEMBER".to_owned(),
            source_delivery_id: "delivery-1".to_owned(),
            payload_sha256: "0".repeat(64),
            receipt_sha256: "0".repeat(64),
        };
        let mut analysis = ReviewAnalysisContext::bounded(0, 1024 * 1024);

        let error = build_checkpoint_proofs(root, &base, &checkpoint, &[&receipt], &mut analysis)
            .unwrap_err();

        assert!(
            error
                .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                .is_some()
        );
        assert!(error.to_string().contains("diff query budget exceeded"));
    }
}
