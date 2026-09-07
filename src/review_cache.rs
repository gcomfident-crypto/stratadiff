use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    ffi::OsStr,
    io::{self, Read},
    path::Path,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Number, Value, json};
use sha2::{Digest, Sha256};

pub const REVIEW_CONTEXT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/tools/review-transition/review-context-v1.schema.json";
pub const REVIEW_RECEIPT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/tools/review-transition/review-receipt-v1.schema.json";
pub const REVIEW_INPUT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/tools/review-transition/review-input-v1.schema.json";
pub const REVIEW_CACHE_PAYLOAD_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-cache-payload-v1.schema.json";
pub const REVIEW_CACHE_RESULT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-cache-result-v1.schema.json";
pub const REVIEW_CACHE_REVIEWER_INPUT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-cache-reviewer-input-v1.schema.json";
pub const REVIEW_CACHE_REVIEWER_MANIFEST_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-cache-reviewer-manifest-v1.schema.json";
pub const REVIEW_CACHE_CROSS_ITEM_AGGREGATOR_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-cache-cross-item-aggregator-v1.json";

const CONTEXT_SCHEMA_BYTES: &str =
    include_str!("../tools/review-transition/review-context-v1.schema.json");
const RECEIPT_SCHEMA_BYTES: &str =
    include_str!("../tools/review-transition/review-receipt-v1.schema.json");
const INPUT_SCHEMA_BYTES: &str =
    include_str!("../tools/review-transition/review-input-v1.schema.json");
const PAYLOAD_SCHEMA_BYTES: &str = include_str!("../schema/review-cache-payload-v1.schema.json");
const RESULT_SCHEMA_BYTES: &str = include_str!("../schema/review-cache-result-v1.schema.json");
const CROSS_ITEM_AGGREGATOR_BYTES: &str =
    include_str!("../schema/review-cache-cross-item-aggregator-v1.json");
const RECEIPT_SIGNATURE_DOMAIN: &str = "stratadiff.review-receipt";
const RECEIPT_SIGNATURE_PREIMAGE_VERSION: &str = "1";
pub const MAX_REVIEW_CACHE_JSON_BYTES: usize = 96 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDENTITIES: usize = 10_000;
const MAX_GIT_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_SELECTED_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_REPOSITORY_MANIFEST_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCacheDecision {
    Skip,
    Residue,
    Full,
    Blocked,
}

impl ReviewCacheDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Residue => "residue",
            Self::Full => "full",
            Self::Blocked => "blocked",
        }
    }
}

pub struct ReviewCacheReceiptBundle<'a> {
    pub receipt: &'a [u8],
    pub prior_input: &'a [u8],
    pub prior_payload: &'a [u8],
    pub prior_result: &'a [u8],
    pub trusted_key_id: &'a str,
    pub trusted_public_key: &'a str,
    pub trust_domain: &'a str,
    pub trust_policy_sha256: &'a str,
}

pub struct ReviewCachePreflight<'a> {
    pub repository: &'a Path,
    pub checkpoint: &'a str,
    pub generated_at: &'a str,
    pub current_context: &'a [u8],
    pub receipt: Option<ReviewCacheReceiptBundle<'a>>,
}

pub struct ReviewCacheContextBuild<'a> {
    pub repository: &'a Path,
    pub requested_base: &'a str,
    pub checkpoint: &'a str,
    pub head: &'a str,
    pub provider_host: &'a str,
    pub owner: &'a str,
    pub name: &'a str,
    pub repository_id: &'a str,
    pub pull_request_node_id: &'a str,
    pub pull_request_number: u64,
    pub base_ref: &'a str,
    pub head_ref: &'a str,
    pub observed_at: &'a str,
    pub canonical_metadata_sha256: &'a str,
    pub review_input_scope: &'a str,
    pub reviewer_manifest: &'a [u8],
}

pub struct ReviewCacheArtifacts {
    pub decision: ReviewCacheDecision,
    pub cached_outcome: Option<String>,
    pub review_input: Value,
    pub review_input_bytes: Vec<u8>,
    pub selected_payload: Value,
    pub selected_payload_bytes: Vec<u8>,
    pub reviewer_input: Value,
    pub reviewer_input_bytes: Vec<u8>,
    pub receipt_notice: Option<String>,
}

pub struct ReviewCacheReceiptIssue<'a> {
    pub repository: &'a Path,
    pub expected_base: &'a str,
    pub expected_head: &'a str,
    pub current_context: &'a [u8],
    pub review_input: &'a [u8],
    pub selected_payload: &'a [u8],
    pub result: &'a [u8],
    pub prior_receipt: Option<ReviewCacheReceiptBundle<'a>>,
    pub receipt_id: &'a str,
    pub issued_at: &'a str,
    pub issuer_id: &'a str,
    pub trust_domain: &'a str,
    pub trust_policy_sha256: &'a str,
    pub key_id: &'a str,
    pub signing_key: &'a str,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct ExactGitChangeIdentity {
    status: ChangeStatus,
    similarity_percent: Option<u8>,
    before_path_base64: Option<String>,
    after_path_base64: Option<String>,
    before_mode: Option<String>,
    after_mode: Option<String>,
    before_object_id: Option<String>,
    after_object_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum ChangeStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
}

#[derive(Clone)]
struct Snapshot {
    commit_oid: String,
    tree_oid: String,
}

struct Transition {
    q: Snapshot,
    a: Snapshot,
    b: Snapshot,
    c: Snapshot,
    d: Snapshot,
}

struct PayloadSelection<'a> {
    current: &'a BTreeMap<String, ExactGitChangeIdentity>,
    retired: &'a BTreeMap<String, ExactGitChangeIdentity>,
    base_drift: Option<&'a (String, Value)>,
    retained_non_current: &'a BTreeMap<String, Value>,
}

struct ContextFacts {
    value: Value,
    body_sha256: String,
    compatibility_sha256: String,
    historical_dispositions_sha256: String,
    repository_closure_sha256: String,
    closure_status: String,
    open_reason: Option<String>,
    input_scope: String,
    composition_kind: String,
    semantic_errors: Vec<String>,
}

#[derive(Clone, Copy)]
enum ReceiptAbsentReason {
    NotProvided,
    UntrustedSource,
    InvalidSignature,
    IncompleteCoverage,
    DigestMismatch,
}

impl ReceiptAbsentReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotProvided => "not_provided",
            Self::UntrustedSource => "untrusted_source",
            Self::InvalidSignature => "invalid_signature",
            Self::IncompleteCoverage => "incomplete_coverage",
            Self::DigestMismatch => "digest_mismatch",
        }
    }
}

struct ReceiptFailure {
    reason: ReceiptAbsentReason,
    message: String,
}

struct VerifiedReceipt {
    reference: Value,
    body: Value,
    prior_current: BTreeMap<String, ExactGitChangeIdentity>,
    prior_item_outcomes: BTreeMap<String, String>,
    prior_outcome: String,
    blocking_non_current: BTreeMap<String, Value>,
    reviewer_input_projection_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewOutcome {
    Passed,
    Advisory,
    ChangesRequested,
    Failed,
}

impl ReviewOutcome {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "passed" => Ok(Self::Passed),
            "advisory" => Ok(Self::Advisory),
            "changes_requested" => Ok(Self::ChangesRequested),
            "failed" => Ok(Self::Failed),
            _ => bail!("unsupported review outcome {value:?}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Advisory => "advisory",
            Self::ChangesRequested => "changes_requested",
            Self::Failed => "failed",
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::Advisory => 1,
            Self::ChangesRequested => 2,
            Self::Failed => 3,
        }
    }
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(Number::from(value))))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(Number::from(value))))
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                let number = Number::from_f64(value)
                    .ok_or_else(|| E::custom("JSON number is not finite"))?;
                Ok(StrictValue(Value::Number(number)))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_string(value.to_owned())
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }

            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                StrictValue::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate JSON object key {key:?}"
                        )));
                    }
                    let value = object.next_value::<StrictValue>()?;
                    values.insert(key, value.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

pub fn build_review_cache_context(request: ReviewCacheContextBuild<'_>) -> Result<Vec<u8>> {
    ensure!(
        request.reviewer_manifest.len() <= MAX_REVIEW_CACHE_JSON_BYTES,
        "reviewer manifest exceeds the {MAX_REVIEW_CACHE_JSON_BYTES} byte limit"
    );
    let manifest = parse_strict_json(request.reviewer_manifest, "reviewer manifest")?;
    let manifest_object = manifest
        .as_object()
        .context("reviewer manifest is not an object")?;
    let expected_keys = BTreeSet::from([
        "schema",
        "contract_version",
        "dependency_closure",
        "reviewer",
        "historical_dispositions",
    ]);
    let actual_keys = manifest_object
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        actual_keys == expected_keys,
        "reviewer manifest fields differ from the v1 contract"
    );
    ensure!(
        string_at(&manifest, &["schema"])? == REVIEW_CACHE_REVIEWER_MANIFEST_SCHEMA
            && string_at(&manifest, &["contract_version"])? == "1.0.0",
        "reviewer manifest schema or contract version is unsupported"
    );
    ensure!(
        git_text(request.repository, &["rev-parse", "--show-object-format"])? == "sha1",
        "review cache context currently requires a SHA-1 Git repository"
    );
    let transition = resolve_transition(
        request.repository,
        request.requested_base,
        request.checkpoint,
        request.head,
    )?;
    let repository_closure = build_repository_closure(request.repository, &transition)?;
    let mut reviewer = manifest["reviewer"].clone();
    let tools = reviewer["tools"]
        .as_array_mut()
        .context("reviewer manifest tools are not an array")?;
    tools.sort_by(|left, right| {
        canonical_json_bytes(left)
            .expect("JSON values are canonically serializable")
            .cmp(&canonical_json_bytes(right).expect("JSON values are canonically serializable"))
    });
    ensure!(
        canonically_strictly_ordered(&reviewer["tools"])?,
        "reviewer manifest contains duplicate tools"
    );
    let mut historical_dispositions = manifest["historical_dispositions"].clone();
    let dispositions = historical_dispositions
        .as_array_mut()
        .context("reviewer manifest historical dispositions are not an array")?;
    dispositions.sort_by(|left, right| {
        canonical_json_bytes(left)
            .expect("JSON values are canonically serializable")
            .cmp(&canonical_json_bytes(right).expect("JSON values are canonically serializable"))
    });
    ensure!(
        canonically_strictly_ordered(&historical_dispositions)?,
        "reviewer manifest contains duplicate historical dispositions"
    );
    let body = json!({
        "reuse_policy": "exact_git_change_identity_only_v1",
        "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
        "review_input_scope": request.review_input_scope,
        "dependency_closure": manifest["dependency_closure"],
        "reviewer": reviewer,
        "repository": {
            "provider": "github",
            "host": request.provider_host,
            "owner": request.owner,
            "name": request.name,
            "repository_id": request.repository_id,
            "object_format": "sha1",
        },
        "repository_closure": repository_closure,
        "pull_request": {
            "node_id": request.pull_request_node_id,
            "number": request.pull_request_number,
            "base_ref": request.base_ref,
            "head_ref": request.head_ref,
            "base_oid": transition.q.commit_oid,
            "head_oid": transition.d.commit_oid,
            "observed_at": request.observed_at,
            "canonical_metadata_sha256": request.canonical_metadata_sha256,
        },
        "historical_dispositions": historical_dispositions,
    });
    let compatibility = json!({
        "reuse_policy": body["reuse_policy"],
        "change_identity_schema": body["change_identity_schema"],
        "review_input_scope": body["review_input_scope"],
        "dependency_closure": body["dependency_closure"],
        "reviewer": body["reviewer"],
    });
    let context = json!({
        "schema": REVIEW_CONTEXT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "body_sha256": sha256_value(&body)?,
        "compatibility_sha256": sha256_value(&compatibility)?,
        "body": body,
    });
    let parsed = parse_context(&canonical_json_bytes(&context)?)?;
    let mut errors = parsed.semantic_errors.clone();
    validate_context_git_closure(&parsed, &transition, &mut errors);
    ensure!(
        errors.is_empty(),
        "generated review context is invalid: {}",
        errors.join("; ")
    );
    canonical_json_bytes(&context)
}

fn build_repository_closure(repository: &Path, transition: &Transition) -> Result<Value> {
    let root_commits = [
        transition.q.commit_oid.clone(),
        transition.b.commit_oid.clone(),
        transition.d.commit_oid.clone(),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect::<Vec<_>>();
    let root_trees = [
        transition.q.tree_oid.clone(),
        transition.b.tree_oid.clone(),
        transition.d.tree_oid.clone(),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect::<Vec<_>>();
    let mut rev_list_arguments = vec!["rev-list", "--objects", "--no-object-names"];
    rev_list_arguments.extend(root_commits.iter().map(String::as_str));
    let object_output = git_output_bounded(
        repository,
        &rev_list_arguments,
        MAX_REPOSITORY_MANIFEST_BYTES,
    )?
    .stdout;
    let mut objects = BTreeSet::new();
    for line in object_output.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let object_id = std::str::from_utf8(line).context("Git object list is not UTF-8")?;
        ensure!(
            is_sha1(object_id),
            "Git object list contains an invalid object ID"
        );
        objects.insert(object_id.to_owned());
    }
    ensure!(!objects.is_empty(), "Git repository closure is empty");
    ensure!(
        objects.len() <= 10_000_000,
        "Git repository closure exceeds 10000000 objects"
    );

    let mut paths = BTreeSet::new();
    for root in &root_commits {
        let output = git_output_bounded(
            repository,
            &["ls-tree", "-r", "-z", "--name-only", root],
            MAX_REPOSITORY_MANIFEST_BYTES,
        )?
        .stdout;
        for path in output.split(|byte| *byte == 0) {
            if path.is_empty() {
                continue;
            }
            ensure!(path.len() <= 4096, "Git repository path exceeds 4096 bytes");
            paths.insert(STANDARD.encode(path));
        }
    }
    ensure!(
        paths.len() <= 1_000_000,
        "Git repository closure exceeds 1000000 paths"
    );
    let object_manifest = objects.into_iter().collect::<Vec<_>>();
    let path_manifest = paths.into_iter().collect::<Vec<_>>();
    let closure_basis = json!({
        "kind": "declared_git_object_and_path_closure_v1",
        "complete": true,
        "root_commit_oids": root_commits,
        "root_tree_oids": root_trees,
        "object_count": object_manifest.len(),
        "path_count": path_manifest.len(),
        "object_manifest_sha256": sha256_value(&json!(object_manifest))?,
        "path_manifest_sha256": sha256_value(&json!(path_manifest))?,
    });
    let mut closure = closure_basis.clone();
    closure["canonical_closure_sha256"] = Value::String(sha256_value(&closure_basis)?);
    Ok(closure)
}

pub fn review_cache_preflight(request: ReviewCachePreflight<'_>) -> Result<ReviewCacheArtifacts> {
    ensure!(
        request.current_context.len() <= MAX_REVIEW_CACHE_JSON_BYTES,
        "review context exceeds the {MAX_REVIEW_CACHE_JSON_BYTES} byte limit"
    );
    let context = parse_context(request.current_context)?;
    let base = string_at(&context.value, &["body", "pull_request", "base_oid"])?;
    let head = string_at(&context.value, &["body", "pull_request", "head_oid"])?;
    let transition = resolve_transition(request.repository, base, request.checkpoint, head)?;

    let mut context_errors = context.semantic_errors.clone();
    validate_context_git_closure(&context, &transition, &mut context_errors);

    let current_changes = match discover_changes(
        request.repository,
        &transition.c.commit_oid,
        &transition.d.commit_oid,
    ) {
        Ok(changes) => changes,
        Err(error) => {
            return build_blocked(
                request.generated_at,
                &context,
                &transition,
                request.receipt.as_ref(),
                "source_closure_incomplete",
                format!("current Git input is unavailable: {error:#}"),
            );
        }
    };
    if let Err(error) = verify_change_objects(request.repository, &current_changes) {
        return build_blocked(
            request.generated_at,
            &context,
            &transition,
            request.receipt.as_ref(),
            "source_closure_incomplete",
            format!("current Git source closure is incomplete: {error:#}"),
        );
    }
    if !context_errors.is_empty() {
        return build_blocked(
            request.generated_at,
            &context,
            &transition,
            request.receipt.as_ref(),
            "invalid_receipt_or_context",
            context_errors.join("; "),
        );
    }

    let current = changes_by_digest(current_changes)?;
    let receipt_outcome = request
        .receipt
        .as_ref()
        .map(|bundle| verify_receipt(bundle, request.repository, &context, &transition));
    let (verified_receipt, absent_reason, receipt_notice) = match receipt_outcome {
        None => (None, ReceiptAbsentReason::NotProvided, None),
        Some(Ok(receipt)) => (Some(receipt), ReceiptAbsentReason::NotProvided, None),
        Some(Err(failure)) => (
            None,
            failure.reason,
            Some(format!(
                "prior receipt was not eligible for reuse: {}",
                failure.message
            )),
        ),
    };

    let repository = context.value["body"]["repository"].clone();
    let pull_request = current_pull_request_reference(&context.value);
    let current_ids = current.keys().cloned().collect::<Vec<_>>();
    let mut carried_ids = BTreeSet::new();
    let mut retired = BTreeMap::new();
    let mut retained_non_current = BTreeMap::new();
    let context_comparison;
    let receipt_reference;
    let carry_eligible;
    let base_changed = transition.a.commit_oid != transition.c.commit_oid
        || transition.a.tree_oid != transition.c.tree_oid;

    if let Some(receipt) = &verified_receipt {
        receipt_reference = receipt.reference.clone();
        let current_metadata = string_at(
            &context.value,
            &["body", "pull_request", "canonical_metadata_sha256"],
        )?;
        let receipt_metadata = string_at(&receipt.body, &["pull_request", "metadata_sha256"])?;
        let receipt_compatibility =
            string_at(&receipt.body, &["review_context_compatibility_sha256"])?;
        let receipt_history = string_at(&receipt.body, &["historical_dispositions_sha256"])?;
        let receipt_repository_closure =
            string_at(&receipt.body, &["review_repository_closure_sha256"])?;
        let compatibility_equal = receipt_compatibility == context.compatibility_sha256;
        let history_equal = receipt_history == context.historical_dispositions_sha256;
        let metadata_equal = receipt_metadata == current_metadata;
        let repository_scope_equal = context.input_scope == "selected_payload_only"
            || (receipt_repository_closure == context.repository_closure_sha256
                && transition.b.commit_oid == transition.d.commit_oid
                && transition.a.tree_oid == transition.c.tree_oid
                && transition.b.tree_oid == transition.d.tree_oid);
        carry_eligible = context.closure_status == "closed"
            && compatibility_equal
            && history_equal
            && metadata_equal
            && repository_scope_equal;
        context_comparison = if carry_eligible {
            exact_context_comparison(&context, &receipt.body)?
        } else if context.closure_status == "open" {
            unavailable_context_comparison(&context)
        } else {
            mismatch_context_comparison(&context, &receipt.body)?
        };

        let holistic_projection_equal =
            if carry_eligible && context.composition_kind == "holistic" && !base_changed {
                let candidate = match build_payload(
                    request.repository,
                    ReviewCacheDecision::Full,
                    &context,
                    &transition,
                    PayloadSelection {
                        current: &current,
                        retired: &BTreeMap::new(),
                        base_drift: None,
                        retained_non_current: &BTreeMap::new(),
                    },
                ) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        return build_blocked(
                            request.generated_at,
                            &context,
                            &transition,
                            request.receipt.as_ref(),
                            "resource_limit",
                            format!("reviewer-visible input could not be materialized: {error:#}"),
                        );
                    }
                };
                sha256_value(&reviewer_input_projection(&candidate, &context))?
                    == receipt.reviewer_input_projection_sha256
            } else {
                false
            };
        if carry_eligible && context.composition_kind == "itemwise_closed_v1" {
            for digest in current.keys() {
                if receipt.prior_current.contains_key(digest)
                    && receipt
                        .prior_item_outcomes
                        .get(digest)
                        .is_some_and(|outcome| matches!(outcome.as_str(), "passed" | "advisory"))
                {
                    carried_ids.insert(digest.clone());
                }
            }
        } else if carry_eligible
            && holistic_projection_equal
            && current.len() == receipt.prior_current.len()
            && current
                .keys()
                .all(|digest| receipt.prior_current.contains_key(digest))
        {
            carried_ids.extend(current.keys().cloned());
        }
        for (digest, identity) in &receipt.prior_current {
            if !current.contains_key(digest) {
                retired.insert(digest.clone(), identity.clone());
            }
        }
        retained_non_current = receipt.blocking_non_current.clone();
    } else {
        receipt_reference = absent_receipt_reference(absent_reason);
        context_comparison = unavailable_context_comparison(&context);
        carry_eligible = false;
    }

    let base_drift = if verified_receipt.is_some() && base_changed {
        match base_drift_obligation(request.repository, &transition) {
            Ok(obligation) => Some(obligation),
            Err(error) => {
                return build_blocked(
                    request.generated_at,
                    &context,
                    &transition,
                    request.receipt.as_ref(),
                    "source_closure_incomplete",
                    format!("base-drift review input is unavailable: {error:#}"),
                );
            }
        }
    } else {
        None
    };

    for identity in retained_non_current.keys() {
        if current.contains_key(identity) {
            carried_ids.remove(identity);
        }
    }
    retained_non_current.retain(|identity, _| {
        !current.contains_key(identity)
            && !retired.contains_key(identity)
            && base_drift
                .as_ref()
                .is_none_or(|(obligation, _)| obligation != identity)
    });

    let selected_current = current
        .iter()
        .filter(|(digest, _)| !carried_ids.contains(*digest))
        .map(|(digest, identity)| (digest.clone(), identity.clone()))
        .collect::<BTreeMap<_, _>>();
    let obligation_count = selected_current
        .len()
        .checked_add(retired.len())
        .and_then(|count| count.checked_add(usize::from(base_drift.is_some())))
        .and_then(|count| count.checked_add(retained_non_current.len()))
        .context("review cache obligation count overflow")?;
    if obligation_count > MAX_IDENTITIES {
        return build_blocked(
            request.generated_at,
            &context,
            &transition,
            request.receipt.as_ref(),
            "resource_limit",
            format!(
                "review obligation limit exceeded: observed {obligation_count}, limit {MAX_IDENTITIES}"
            ),
        );
    }

    let decision = if carry_eligible && obligation_count == 0 {
        ReviewCacheDecision::Skip
    } else if carry_eligible && (!carried_ids.is_empty() || !retained_non_current.is_empty()) {
        ReviewCacheDecision::Residue
    } else {
        ReviewCacheDecision::Full
    };
    let payload = match build_payload(
        request.repository,
        decision,
        &context,
        &transition,
        PayloadSelection {
            current: &selected_current,
            retired: &retired,
            base_drift: base_drift.as_ref(),
            retained_non_current: &retained_non_current,
        },
    ) {
        Ok(payload) => payload,
        Err(error) => {
            return build_blocked(
                request.generated_at,
                &context,
                &transition,
                request.receipt.as_ref(),
                "resource_limit",
                format!("selected review payload could not be materialized: {error:#}"),
            );
        }
    };
    let payload_bytes = canonical_json_bytes(&payload)?;
    if payload_bytes.len() > MAX_REVIEW_CACHE_JSON_BYTES {
        return build_blocked(
            request.generated_at,
            &context,
            &transition,
            request.receipt.as_ref(),
            "resource_limit",
            format!("selected review payload exceeds the {MAX_REVIEW_CACHE_JSON_BYTES} byte limit"),
        );
    }
    let payload_sha256 = sha256_bytes(&payload_bytes);
    let reviewer_input = reviewer_input_projection(&payload, &context);
    let reviewer_input_bytes = canonical_json_bytes(&reviewer_input)?;
    let reviewer_input_projection_sha256 = sha256_bytes(&reviewer_input_bytes);
    let selected_ids = payload_item_ids(&payload)?;

    let review_required = selected_ids
        .iter()
        .map(|digest| {
            let reason = if selected_current.contains_key(digest) {
                "new_or_changed_git_identity"
            } else if base_drift
                .as_ref()
                .is_some_and(|(base_digest, _)| base_digest == digest)
            {
                "base_drift"
            } else if retained_non_current.contains_key(digest) {
                "retained_blocking_obligation"
            } else {
                "retired_checkpoint_change"
            };
            json!({
                "obligation_sha256": digest,
                "reason": reason,
                "evidence_sha256": digest,
            })
        })
        .collect::<Vec<_>>();
    let carried = carried_ids
        .iter()
        .map(|digest| {
            let prior_outcome = verified_receipt
                .as_ref()
                .and_then(|receipt| receipt.prior_item_outcomes.get(digest))
                .expect("every carried identity has a verified prior obligation outcome");
            json!({
                "current_identity_sha256": digest,
                "receipt_identity_sha256": digest,
                "basis": "exact_git_change_identity",
                "prior_outcome": prior_outcome,
            })
        })
        .collect::<Vec<_>>();
    let unresolved_retired = retired.keys().cloned().collect::<Vec<_>>();
    let base_drift_ids = base_drift
        .as_ref()
        .map(|(digest, _)| vec![digest.clone()])
        .unwrap_or_default();
    let retained_non_current_ids = retained_non_current.keys().cloned().collect::<Vec<_>>();
    let selected_current_ids = selected_current.keys().cloned().collect::<Vec<_>>();
    let (reason, payload_mode) = match decision {
        ReviewCacheDecision::Skip => ("all_obligations_exactly_covered", "empty"),
        ReviewCacheDecision::Residue => ("exact_carries_removed_from_review_input", "residue"),
        ReviewCacheDecision::Full if verified_receipt.is_none() => ("receipt_unavailable", "full"),
        ReviewCacheDecision::Full if context.closure_status == "open" => ("context_open", "full"),
        ReviewCacheDecision::Full if !carry_eligible => ("context_mismatch", "full"),
        ReviewCacheDecision::Full => ("no_exact_carry", "full"),
        ReviewCacheDecision::Blocked => unreachable!("blocked decisions are built separately"),
    };
    let review_input = json!({
        "schema": REVIEW_INPUT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "generated_at": request.generated_at,
        "reuse_policy": "exact_git_change_identity_only_v1",
        "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
        "repository": repository,
        "pull_request": pull_request,
        "transition": transition_json(&transition),
        "current_context": context_reference(&context),
        "resolution": {
            "decision": decision.as_str(),
            "reason": reason,
            "prior_receipt": receipt_reference,
            "context_comparison": context_comparison,
            "selected_payload": {
                "mode": payload_mode,
                "schema_uri": REVIEW_CACHE_PAYLOAD_SCHEMA,
                "canonical_payload_sha256": payload_sha256,
                "reviewer_input_projection_sha256": reviewer_input_projection_sha256,
                "byte_length": payload_bytes.len(),
                "selected_identity_sha256": selected_ids,
            },
            "accounting": {
                "coverage_complete": true,
                "full_current_identity_sha256": current_ids,
                "carried": carried,
                "selected_current_identity_sha256": selected_current_ids,
                "review_required": review_required,
                "unresolved_retired_obligation_sha256": unresolved_retired,
                "base_drift_obligation_sha256": base_drift_ids,
                "retained_non_current_obligation_sha256": retained_non_current_ids,
                "blocking_reasons": [],
            },
        },
    });
    validate_against_schema(&review_input, INPUT_SCHEMA_BYTES, "review input")?;
    let review_input_bytes = canonical_json_bytes(&review_input)?;
    Ok(ReviewCacheArtifacts {
        decision,
        cached_outcome: verified_receipt
            .as_ref()
            .map(|receipt| receipt.prior_outcome.clone()),
        review_input,
        review_input_bytes,
        selected_payload: payload,
        selected_payload_bytes: payload_bytes,
        reviewer_input,
        reviewer_input_bytes,
        receipt_notice,
    })
}

pub fn issue_review_cache_receipt(request: ReviewCacheReceiptIssue<'_>) -> Result<Vec<u8>> {
    for (bytes, label) in [
        (request.current_context, "review context"),
        (request.review_input, "review input"),
        (request.selected_payload, "selected payload"),
        (request.result, "review result"),
    ] {
        ensure!(
            bytes.len() <= MAX_REVIEW_CACHE_JSON_BYTES,
            "{label} exceeds the {MAX_REVIEW_CACHE_JSON_BYTES} byte limit"
        );
    }
    let context = parse_context(request.current_context)?;
    ensure!(
        context.semantic_errors.is_empty(),
        "review context is not valid: {}",
        context.semantic_errors.join("; ")
    );
    let review_input = parse_strict_json(request.review_input, "review input")?;
    validate_against_schema(&review_input, INPUT_SCHEMA_BYTES, "review input")?;
    let payload = parse_strict_json(request.selected_payload, "selected payload")?;
    validate_against_schema(&payload, PAYLOAD_SCHEMA_BYTES, "selected payload")?;
    let result = parse_strict_json(request.result, "review result")?;
    validate_against_schema(&result, RESULT_SCHEMA_BYTES, "review result")?;
    let result_schema = string_at(&result, &["schema"])?;

    let decision = string_at(&review_input, &["resolution", "decision"])?;
    ensure!(
        matches!(decision, "full" | "residue"),
        "a receipt can be issued only after a full or residue reviewer execution"
    );
    ensure!(
        decision != "residue" || context.composition_kind == "itemwise_closed_v1",
        "partial review execution requires an itemwise-closed composition contract"
    );
    ensure!(
        string_at(&payload, &["mode"])? == decision,
        "selected payload mode differs from the review route"
    );
    let payload_ids = payload_item_ids(&payload)?;
    validate_payload_items(&payload, Some(request.repository))?;
    validate_review_input_accounting(&review_input, &payload, &payload_ids)?;

    let review_input_bytes = canonical_json_bytes(&review_input)?;
    let payload_bytes = canonical_json_bytes(&payload)?;
    let result_bytes = canonical_json_bytes(&result)?;
    let reviewer_input_projection_sha256 =
        sha256_value(&reviewer_input_projection(&payload, &context))?;
    validate_result_manifest(
        &result,
        &sha256_bytes(&review_input_bytes),
        &sha256_bytes(&payload_bytes),
        &reviewer_input_projection_sha256,
        &payload_ids,
    )?;
    let selected_item_outcomes = result_item_outcomes(&result, &payload_ids)?;
    ensure!(
        string_at(
            &review_input,
            &["resolution", "selected_payload", "canonical_payload_sha256",],
        )? == sha256_bytes(&payload_bytes)
            && usize_at(
                &review_input,
                &["resolution", "selected_payload", "byte_length"],
            )? == payload_bytes.len(),
        "review input does not bind the supplied selected payload"
    );
    ensure!(
        string_at(
            &review_input,
            &[
                "resolution",
                "selected_payload",
                "reviewer_input_projection_sha256",
            ],
        )? == reviewer_input_projection_sha256,
        "review input does not bind the reviewer-visible input projection"
    );

    let q = string_at(&review_input, &["transition", "Q", "commit_oid"])?;
    let b = string_at(&review_input, &["transition", "B", "commit_oid"])?;
    let d = string_at(&review_input, &["transition", "D", "commit_oid"])?;
    let transition = resolve_transition(request.repository, q, b, d)?;
    ensure!(
        request.expected_base == transition.q.commit_oid
            && request.expected_head == transition.d.commit_oid,
        "adapter-observed live base tip or head differs from the routed review snapshots"
    );
    ensure!(
        review_input["transition"] == transition_json(&transition),
        "review input transition does not match the complete local Git history"
    );
    ensure!(
        string_at(&context.value, &["body", "pull_request", "base_oid"])? == q
            && string_at(&context.value, &["body", "pull_request", "head_oid"])? == d,
        "review context does not bind the routed Git snapshots"
    );
    let mut context_errors = context.semantic_errors.clone();
    validate_context_git_closure(&context, &transition, &mut context_errors);
    ensure!(
        context_errors.is_empty(),
        "review context is not valid for the routed Git history: {}",
        context_errors.join("; ")
    );
    ensure!(
        review_input["repository"] == context.value["body"]["repository"]
            && review_input["pull_request"] == current_pull_request_reference(&context.value)
            && payload["repository"] == review_input["repository"]
            && payload["pull_request"] == review_input["pull_request"]
            && string_at(&payload, &["base_commit"])? == transition.c.commit_oid
            && string_at(&payload, &["head_commit"])? == transition.d.commit_oid,
        "context, route, and selected payload identities differ"
    );
    ensure!(
        string_at(&review_input, &["current_context", "body_sha256"])? == context.body_sha256
            && string_at(&review_input, &["current_context", "compatibility_sha256"],)?
                == context.compatibility_sha256
            && string_at(&review_input, &["current_context", "closure_status"])?
                == context.closure_status,
        "review input does not bind the supplied current context"
    );

    let current = changes_by_digest(discover_changes(
        request.repository,
        &transition.c.commit_oid,
        &transition.d.commit_oid,
    )?)?;
    let current_ids = current.keys().cloned().collect::<Vec<_>>();
    ensure!(
        string_array_at(
            &review_input,
            &["resolution", "accounting", "full_current_identity_sha256"],
        )? == current_ids,
        "review input current identities do not match the local Git range"
    );
    let checkpoint_changes = changes_by_digest(discover_changes(
        request.repository,
        &transition.a.commit_oid,
        &transition.b.commit_oid,
    )?)?;
    let expected_base_drift = (transition.a.commit_oid != transition.c.commit_oid
        || transition.a.tree_oid != transition.c.tree_oid)
        .then(|| base_drift_obligation(request.repository, &transition))
        .transpose()?;
    let mut selected_retained_non_current = BTreeMap::new();
    for item in payload["items"]
        .as_array()
        .expect("payload schema requires an item array")
    {
        let kind = string_at(item, &["kind"])?;
        let obligation = string_at(item, &["obligation_sha256"])?;
        match kind {
            "current_change" => {
                let identity: ExactGitChangeIdentity =
                    serde_json::from_value(item["identity"].clone())?;
                ensure!(
                    current.get(obligation) == Some(&identity),
                    "selected current payload identity does not match the local Git range"
                );
            }
            "retired_checkpoint_change" => {
                let identity: ExactGitChangeIdentity =
                    serde_json::from_value(item["identity"].clone())?;
                ensure!(
                    checkpoint_changes.get(obligation) == Some(&identity),
                    "retired payload identity does not match the checkpoint Git range"
                );
            }
            "base_drift" => {
                let (expected_digest, expected_evidence) = expected_base_drift
                    .as_ref()
                    .context("selected payload declares base drift for an unchanged base")?;
                ensure!(
                    obligation == expected_digest && item["evidence"] == *expected_evidence,
                    "selected base-drift obligation does not match the local Git transition"
                );
            }
            "retained_non_current_obligation" => {
                ensure!(
                    selected_retained_non_current
                        .insert(obligation.to_owned(), item.clone())
                        .is_none(),
                    "selected payload contains a duplicate retained non-current obligation"
                );
            }
            _ => bail!("selected payload contains an unsupported item kind"),
        }
    }

    let carried_outcomes = carried_outcomes(&review_input)?;
    let selected_current = string_array_at(
        &review_input,
        &[
            "resolution",
            "accounting",
            "selected_current_identity_sha256",
        ],
    )?
    .into_iter()
    .collect::<BTreeSet<_>>();
    let verified_prior = if decision == "residue" {
        let bundle = request
            .prior_receipt
            .as_ref()
            .context("a residue receipt requires the complete trusted prior receipt bundle")?;
        let receipt = verify_receipt(bundle, request.repository, &context, &transition).map_err(
            |failure| {
                anyhow::anyhow!(
                    "prior receipt is not eligible for residue issuance: {}",
                    failure.message
                )
            },
        )?;
        let current_metadata = string_at(
            &context.value,
            &["body", "pull_request", "canonical_metadata_sha256"],
        )?;
        let repository_scope_equal = context.input_scope == "selected_payload_only"
            || (string_at(&receipt.body, &["review_repository_closure_sha256"])?
                == context.repository_closure_sha256
                && transition.b.commit_oid == transition.d.commit_oid
                && transition.a.tree_oid == transition.c.tree_oid
                && transition.b.tree_oid == transition.d.tree_oid);
        ensure!(
            context.closure_status == "closed"
                && string_at(&receipt.body, &["review_context_compatibility_sha256"],)?
                    == context.compatibility_sha256
                && string_at(&receipt.body, &["historical_dispositions_sha256"])?
                    == context.historical_dispositions_sha256
                && string_at(&receipt.body, &["pull_request", "metadata_sha256"])?
                    == current_metadata
                && repository_scope_equal,
            "prior receipt context is not eligible for residue issuance"
        );
        ensure!(
            review_input["resolution"]["prior_receipt"] == receipt.reference
                && review_input["resolution"]["context_comparison"]
                    == exact_context_comparison(&context, &receipt.body)?,
            "review input does not bind the verified prior receipt and exact context comparison"
        );
        ensure!(
            !carried_outcomes.is_empty() || !selected_retained_non_current.is_empty(),
            "a residue route must carry a prior current identity or re-review a retained non-current blocker"
        );
        for (identity, outcome) in &carried_outcomes {
            ensure!(
                matches!(outcome.as_str(), "passed" | "advisory")
                    && receipt.prior_item_outcomes.get(identity) == Some(outcome)
                    && receipt.prior_current.get(identity) == current.get(identity),
                "review input carry is not backed by the verified prior receipt and exact current Git identity"
            );
        }
        let selected_non_retained = payload["items"]
            .as_array()
            .expect("payload schema requires an item array")
            .iter()
            .filter(|item| item["kind"] != "retained_non_current_obligation")
            .map(|item| {
                string_at(item, &["obligation_sha256"])
                    .expect("payload schema requires an obligation digest")
            })
            .collect::<BTreeSet<_>>();
        let expected_retained_non_current = receipt
            .blocking_non_current
            .iter()
            .filter(|(obligation, _)| !selected_non_retained.contains(obligation.as_str()))
            .map(|(obligation, item)| (obligation.clone(), item.clone()))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            selected_retained_non_current == expected_retained_non_current,
            "residue payload does not exactly re-review every retained non-current blocker from the verified prior receipt"
        );
        Some(receipt)
    } else {
        ensure!(
            carried_outcomes.is_empty(),
            "a full route cannot carry prior identities"
        );
        ensure!(
            selected_retained_non_current.is_empty(),
            "a full route cannot claim retained obligations from an unverified prior receipt"
        );
        None
    };

    let mut current_identity_results = Vec::with_capacity(current_ids.len());
    for identity in &current_ids {
        if selected_current.contains(identity) {
            let outcome = selected_item_outcomes
                .get(identity)
                .context("selected current identity has no reviewer outcome")?;
            current_identity_results.push(json!({
                "identity_sha256": identity,
                "outcome": outcome,
                "lineage": {
                    "kind": "selected_execution",
                    "obligation_sha256": identity,
                },
            }));
        } else {
            let outcome = carried_outcomes
                .get(identity)
                .context("current identity is neither selected nor carried")?;
            let prior = verified_prior
                .as_ref()
                .context("carried current identity has no verified prior receipt")?;
            current_identity_results.push(json!({
                "identity_sha256": identity,
                "outcome": outcome,
                "lineage": {
                    "kind": "prior_receipt_exact_carry",
                    "prior_receipt_body_sha256": prior.reference["body_sha256"],
                    "prior_identity_sha256": identity,
                },
            }));
        }
    }
    let selected_non_current_blocking_outcomes = selected_item_outcomes
        .iter()
        .filter(|(obligation, _)| !selected_current.contains(*obligation))
        .map(|(_, outcome)| outcome.as_str())
        .filter(|outcome| matches!(*outcome, "changes_requested" | "failed"));
    let retained_non_current_blocking_outcome =
        reduce_review_outcomes(selected_non_current_blocking_outcomes)?;
    let effective_outcome = reduce_review_outcomes(
        std::iter::once(string_at(&result, &["outcome"])?)
            .chain(carried_outcomes.values().map(String::as_str)),
    )?;

    let body = json!({
        "receipt_id": request.receipt_id,
        "issued_at": request.issued_at,
        "issuer": {
            "source_kind": "signed_review_execution",
            "issuer_id": request.issuer_id,
            "trust_domain": request.trust_domain,
            "trust_policy_sha256": request.trust_policy_sha256,
        },
        "repository": review_input["repository"],
        "pull_request": {
            "node_id": review_input["pull_request"]["node_id"],
            "number": review_input["pull_request"]["number"],
            "requested_base_oid": transition.q.commit_oid,
            "reviewed_merge_base_oid": transition.c.commit_oid,
            "reviewed_head_oid": transition.d.commit_oid,
            "metadata_sha256": review_input["pull_request"]["metadata_sha256"],
        },
        "review_context_body_sha256": context.body_sha256,
        "review_context_compatibility_sha256": context.compatibility_sha256,
        "review_repository_closure_sha256": context.repository_closure_sha256,
        "historical_dispositions_sha256": context.historical_dispositions_sha256,
        "review_composition": context.value["body"]["reviewer"]["composition"],
        "prior_input": {
            "schema_uri": REVIEW_INPUT_SCHEMA,
            "canonical_input_sha256": sha256_bytes(&review_input_bytes),
            "selected_payload_sha256": sha256_bytes(&payload_bytes),
            "byte_length": review_input_bytes.len(),
        },
        "prior_result": {
            "schema_uri": result_schema,
            "canonical_result_sha256": sha256_bytes(&result_bytes),
            "selected_outcome": result["outcome"],
            "retained_non_current_blocking_outcome": retained_non_current_blocking_outcome.as_str(),
            "effective_outcome": effective_outcome.as_str(),
            "verdict_scope": "reviewer_visible_input_projection",
            "reviewer_input_projection_sha256": reviewer_input_projection_sha256,
            "byte_length": result_bytes.len(),
        },
        "coverage": {
            "complete": true,
            "covered_input_sha256": sha256_bytes(&review_input_bytes),
            "covered_identity_sha256": current_ids,
            "current_identity_results": current_identity_results,
            "omitted_identity_sha256": [],
            "unresolved_obligation_sha256": [],
        },
        "reuse_constraints": {
            "policy": "exact_git_change_identity_only_v1",
            "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
            "future_reuse_requires_identical_context": true,
            "future_reuse_requires_exact_git_identity": true,
            "partial_reuse_requires_itemwise_closed_composition": true,
            "four_way_replay_authorizes_verdict_or_input_skip": false,
        },
    });
    let body_sha256 = sha256_value(&body)?;
    let signing_key_bytes = decode_hex_array::<32>(request.signing_key)
        .context("review cache Ed25519 signing key is invalid")?;
    let signing_key = SigningKey::from_bytes(&signing_key_bytes);
    let signing_preimage = receipt_signature_preimage(&body_sha256)?;
    let receipt = json!({
        "schema": REVIEW_RECEIPT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "body": body,
        "attestation": {
            "algorithm": "ed25519",
            "key_id": request.key_id,
            "signature_domain": RECEIPT_SIGNATURE_DOMAIN,
            "signature_preimage_version": RECEIPT_SIGNATURE_PREIMAGE_VERSION,
            "body_sha256": body_sha256,
            "signature": encode_hex(&signing_key.sign(&signing_preimage).to_bytes()),
        },
    });
    validate_against_schema(&receipt, RECEIPT_SCHEMA_BYTES, "review receipt")?;
    canonical_json_bytes(&receipt)
}

fn parse_context(bytes: &[u8]) -> Result<ContextFacts> {
    let value = parse_strict_json(bytes, "review context")?;
    validate_against_schema(&value, CONTEXT_SCHEMA_BYTES, "review context")?;
    let body = &value["body"];
    let body_sha256 = sha256_value(body)?;
    let compatibility = json!({
        "reuse_policy": body["reuse_policy"],
        "change_identity_schema": body["change_identity_schema"],
        "review_input_scope": body["review_input_scope"],
        "dependency_closure": body["dependency_closure"],
        "reviewer": body["reviewer"],
    });
    let compatibility_sha256 = sha256_value(&compatibility)?;
    let historical_dispositions_sha256 = sha256_value(&body["historical_dispositions"])?;
    let mut semantic_errors = Vec::new();
    if string_at(&value, &["body_sha256"])? != body_sha256 {
        semantic_errors.push("review context body digest mismatch".to_owned());
    }
    if string_at(&value, &["compatibility_sha256"])? != compatibility_sha256 {
        semantic_errors.push("review context compatibility digest mismatch".to_owned());
    }
    if !canonically_strictly_ordered(&body["reviewer"]["tools"])? {
        semantic_errors.push("reviewer tools are not in strict canonical order".to_owned());
    }
    if !canonically_strictly_ordered(&body["historical_dispositions"])? {
        semantic_errors
            .push("historical dispositions are not in strict canonical order".to_owned());
    }
    let closure_status = string_at(body, &["dependency_closure", "status"])?.to_owned();
    let open_reason = (closure_status == "open").then(|| {
        string_at(body, &["dependency_closure", "reason"])
            .expect("open dependency closure has a schema-validated reason")
            .to_owned()
    });
    let model_kind = string_at(body, &["reviewer", "model", "kind"])?;
    if model_kind == "hosted_model_unpinned"
        && (closure_status != "open" || open_reason.as_deref() != Some("hosted_model_unpinned"))
    {
        semantic_errors.push(
            "an unpinned hosted model requires an open hosted-model dependency closure".to_owned(),
        );
    }
    let composition_kind = string_at(body, &["reviewer", "composition", "kind"])?;
    if composition_kind == "itemwise_closed_v1"
        && body["reviewer"]["composition"]["cross_item_aggregator"]
            != cross_item_aggregator_binding()?
    {
        semantic_errors.push(
            "itemwise review composition does not bind the built-in conservative outcome reduction"
                .to_owned(),
        );
    }
    Ok(ContextFacts {
        repository_closure_sha256: string_at(
            body,
            &["repository_closure", "canonical_closure_sha256"],
        )?
        .to_owned(),
        input_scope: string_at(body, &["review_input_scope"])?.to_owned(),
        composition_kind: composition_kind.to_owned(),
        value,
        body_sha256,
        compatibility_sha256,
        historical_dispositions_sha256,
        closure_status,
        open_reason,
        semantic_errors,
    })
}

fn validate_context_git_closure(
    context: &ContextFacts,
    transition: &Transition,
    errors: &mut Vec<String>,
) {
    let closure = &context.value["body"]["repository_closure"];
    let root_commits = closure["root_commit_oids"]
        .as_array()
        .expect("context schema requires root commit array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let root_trees = closure["root_tree_oids"]
        .as_array()
        .expect("context schema requires root tree array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for snapshot in [&transition.q, &transition.b, &transition.d] {
        if !root_commits.contains(snapshot.commit_oid.as_str()) {
            errors.push(format!(
                "repository closure does not bind root commit {}",
                snapshot.commit_oid
            ));
        }
    }
    for snapshot in [&transition.q, &transition.b, &transition.d] {
        if !root_trees.contains(snapshot.tree_oid.as_str()) {
            errors.push(format!(
                "repository closure does not bind transition tree {}",
                snapshot.tree_oid
            ));
        }
    }
}

fn verify_receipt(
    bundle: &ReviewCacheReceiptBundle<'_>,
    repository: &Path,
    context: &ContextFacts,
    transition: &Transition,
) -> std::result::Result<VerifiedReceipt, ReceiptFailure> {
    verify_receipt_inner(bundle, repository, context, transition).map_err(|error| ReceiptFailure {
        reason: error.0,
        message: error.1,
    })
}

fn verify_receipt_inner(
    bundle: &ReviewCacheReceiptBundle<'_>,
    repository: &Path,
    context: &ContextFacts,
    transition: &Transition,
) -> std::result::Result<VerifiedReceipt, (ReceiptAbsentReason, String)> {
    for (bytes, label) in [
        (bundle.receipt, "receipt"),
        (bundle.prior_input, "prior input"),
        (bundle.prior_payload, "prior selected payload"),
        (bundle.prior_result, "prior result"),
    ] {
        if bytes.len() > MAX_REVIEW_CACHE_JSON_BYTES {
            return Err((
                ReceiptAbsentReason::DigestMismatch,
                format!("{label} exceeds the {MAX_REVIEW_CACHE_JSON_BYTES} byte limit"),
            ));
        }
    }
    let receipt = parse_strict_json(bundle.receipt, "review receipt")
        .and_then(|value| {
            validate_against_schema(&value, RECEIPT_SCHEMA_BYTES, "review receipt")?;
            Ok(value)
        })
        .map_err(|error| (ReceiptAbsentReason::DigestMismatch, format!("{error:#}")))?;
    let body = &receipt["body"];
    let attestation = &receipt["attestation"];
    if string_at(body, &["issuer", "trust_domain"]).map_err(digest_failure)? != bundle.trust_domain
        || string_at(body, &["issuer", "trust_policy_sha256"]).map_err(digest_failure)?
            != bundle.trust_policy_sha256
        || string_at(attestation, &["key_id"]).map_err(digest_failure)? != bundle.trusted_key_id
    {
        return Err((
            ReceiptAbsentReason::UntrustedSource,
            "receipt issuer or trust policy is not trusted".to_owned(),
        ));
    }
    if string_at(attestation, &["signature_domain"]).map_err(digest_failure)?
        != RECEIPT_SIGNATURE_DOMAIN
        || string_at(attestation, &["signature_preimage_version"]).map_err(digest_failure)?
            != RECEIPT_SIGNATURE_PREIMAGE_VERSION
    {
        return Err((
            ReceiptAbsentReason::InvalidSignature,
            "receipt signature domain or preimage version is unsupported".to_owned(),
        ));
    }
    let body_bytes = canonical_json_bytes(body).map_err(digest_failure)?;
    let body_sha256 = sha256_bytes(&body_bytes);
    if string_at(attestation, &["body_sha256"]).map_err(digest_failure)? != body_sha256 {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "receipt body digest does not match its canonical body".to_owned(),
        ));
    }
    let public_key = decode_hex_array::<32>(bundle.trusted_public_key).map_err(|error| {
        (
            ReceiptAbsentReason::UntrustedSource,
            format!("trusted Ed25519 public key is invalid: {error:#}"),
        )
    })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|error| {
        (
            ReceiptAbsentReason::UntrustedSource,
            format!("trusted Ed25519 public key is invalid: {error}"),
        )
    })?;
    let signature =
        decode_hex_array::<64>(string_at(attestation, &["signature"]).map_err(digest_failure)?)
            .map_err(|error| {
                (
                    ReceiptAbsentReason::InvalidSignature,
                    format!("receipt signature is invalid: {error:#}"),
                )
            })?;
    let signing_preimage = receipt_signature_preimage(&body_sha256).map_err(digest_failure)?;
    verifying_key
        .verify(&signing_preimage, &Signature::from_bytes(&signature))
        .map_err(|_| {
            (
                ReceiptAbsentReason::InvalidSignature,
                "receipt signature verification failed".to_owned(),
            )
        })?;

    if body["repository"] != context.value["body"]["repository"]
        || body["pull_request"]["node_id"] != context.value["body"]["pull_request"]["node_id"]
        || body["pull_request"]["number"] != context.value["body"]["pull_request"]["number"]
        || body["review_composition"] != context.value["body"]["reviewer"]["composition"]
        || string_at(body, &["pull_request", "reviewed_merge_base_oid"]).map_err(digest_failure)?
            != transition.a.commit_oid
        || string_at(body, &["pull_request", "reviewed_head_oid"]).map_err(digest_failure)?
            != transition.b.commit_oid
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "receipt repository, pull request, or reviewed Git snapshot differs".to_owned(),
        ));
    }
    if !body["coverage"]["complete"].as_bool().unwrap_or(false)
        || !body["coverage"]["omitted_identity_sha256"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !body["coverage"]["unresolved_obligation_sha256"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "receipt coverage is incomplete".to_owned(),
        ));
    }

    let prior_input = parse_strict_json(bundle.prior_input, "prior review input")
        .and_then(|value| {
            validate_against_schema(&value, INPUT_SCHEMA_BYTES, "prior review input")?;
            Ok(value)
        })
        .map_err(digest_failure)?;
    let prior_payload = parse_strict_json(bundle.prior_payload, "prior selected payload")
        .and_then(|value| {
            validate_against_schema(&value, PAYLOAD_SCHEMA_BYTES, "prior selected payload")?;
            Ok(value)
        })
        .map_err(digest_failure)?;
    let prior_result = parse_strict_json(bundle.prior_result, "prior review result")
        .and_then(|value| {
            validate_against_schema(&value, RESULT_SCHEMA_BYTES, "prior review result")?;
            Ok(value)
        })
        .map_err(digest_failure)?;
    validate_payload_items(&prior_payload, Some(repository)).map_err(digest_failure)?;
    let prior_input_bytes = canonical_json_bytes(&prior_input).map_err(digest_failure)?;
    let prior_payload_bytes = canonical_json_bytes(&prior_payload).map_err(digest_failure)?;
    let prior_result_bytes = canonical_json_bytes(&prior_result).map_err(digest_failure)?;
    let prior_input_sha256 = sha256_bytes(&prior_input_bytes);
    let prior_payload_sha256 = sha256_bytes(&prior_payload_bytes);
    let prior_result_sha256 = sha256_bytes(&prior_result_bytes);
    if string_at(body, &["prior_input", "canonical_input_sha256"]).map_err(digest_failure)?
        != prior_input_sha256
        || string_at(body, &["coverage", "covered_input_sha256"]).map_err(digest_failure)?
            != prior_input_sha256
        || string_at(body, &["prior_input", "selected_payload_sha256"]).map_err(digest_failure)?
            != prior_payload_sha256
        || string_at(body, &["prior_result", "canonical_result_sha256"]).map_err(digest_failure)?
            != prior_result_sha256
        || usize_at(body, &["prior_input", "byte_length"]).map_err(digest_failure)?
            != prior_input_bytes.len()
        || usize_at(body, &["prior_result", "byte_length"]).map_err(digest_failure)?
            != prior_result_bytes.len()
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "receipt does not bind the supplied prior artifacts".to_owned(),
        ));
    }
    if string_at(body, &["prior_input", "schema_uri"]).map_err(digest_failure)?
        != string_at(&prior_input, &["schema"]).map_err(digest_failure)?
        || string_at(body, &["prior_result", "schema_uri"]).map_err(digest_failure)?
            != string_at(&prior_result, &["schema"]).map_err(digest_failure)?
        || string_at(
            &prior_input,
            &["resolution", "selected_payload", "canonical_payload_sha256"],
        )
        .map_err(digest_failure)?
            != prior_payload_sha256
        || usize_at(
            &prior_input,
            &["resolution", "selected_payload", "byte_length"],
        )
        .map_err(digest_failure)?
            != prior_payload_bytes.len()
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "prior input or result artifact binding differs".to_owned(),
        ));
    }
    let prior_transition = resolve_transition(
        repository,
        string_at(&prior_input, &["transition", "Q", "commit_oid"]).map_err(digest_failure)?,
        string_at(&prior_input, &["transition", "B", "commit_oid"]).map_err(digest_failure)?,
        string_at(&prior_input, &["transition", "D", "commit_oid"]).map_err(digest_failure)?,
    )
    .map_err(digest_failure)?;
    if prior_input["transition"] != transition_json(&prior_transition)
        || prior_transition.c.commit_oid != transition.a.commit_oid
        || prior_transition.d.commit_oid != transition.b.commit_oid
        || string_at(body, &["pull_request", "requested_base_oid"]).map_err(digest_failure)?
            != prior_transition.q.commit_oid
        || string_at(body, &["pull_request", "reviewed_merge_base_oid"]).map_err(digest_failure)?
            != prior_transition.c.commit_oid
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "prior input transition does not resolve to the receipt Git snapshot".to_owned(),
        ));
    }
    if prior_payload["repository"] != body["repository"]
        || prior_payload["pull_request"]["node_id"] != body["pull_request"]["node_id"]
        || prior_payload["pull_request"]["number"] != body["pull_request"]["number"]
        || prior_payload["pull_request"]["metadata_sha256"]
            != body["pull_request"]["metadata_sha256"]
        || string_at(&prior_payload, &["base_commit"]).map_err(digest_failure)?
            != transition.a.commit_oid
        || string_at(&prior_payload, &["head_commit"]).map_err(digest_failure)?
            != transition.b.commit_oid
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "prior payload identity or Git snapshot differs from the receipt".to_owned(),
        ));
    }
    let payload_ids = payload_item_ids(&prior_payload).map_err(digest_failure)?;
    validate_review_input_accounting(&prior_input, &prior_payload, &payload_ids)
        .map_err(digest_failure)?;
    let reviewer_input_projection_sha256 =
        sha256_value(&reviewer_input_projection(&prior_payload, context))
            .map_err(digest_failure)?;
    validate_result_manifest(
        &prior_result,
        &prior_input_sha256,
        &prior_payload_sha256,
        &reviewer_input_projection_sha256,
        &payload_ids,
    )
    .map_err(digest_failure)?;
    let selected_item_outcomes =
        result_item_outcomes(&prior_result, &payload_ids).map_err(digest_failure)?;
    let input_ids = string_array_at(
        &prior_input,
        &["resolution", "selected_payload", "selected_identity_sha256"],
    )
    .map_err(digest_failure)?;
    if payload_ids != input_ids
        || string_at(body, &["prior_result", "selected_outcome"]).map_err(digest_failure)?
            != string_at(&prior_result, &["outcome"]).map_err(digest_failure)?
        || string_at(body, &["prior_result", "reviewer_input_projection_sha256"])
            .map_err(digest_failure)?
            != reviewer_input_projection_sha256
        || string_at(
            &prior_input,
            &[
                "resolution",
                "selected_payload",
                "reviewer_input_projection_sha256",
            ],
        )
        .map_err(digest_failure)?
            != reviewer_input_projection_sha256
    {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "prior payload identities are not exactly covered".to_owned(),
        ));
    }

    let mut payload_current = BTreeMap::new();
    let expected_prior_base_drift = (prior_transition.a.commit_oid
        != prior_transition.c.commit_oid
        || prior_transition.a.tree_oid != prior_transition.c.tree_oid)
        .then(|| base_drift_obligation(repository, &prior_transition))
        .transpose()
        .map_err(digest_failure)?;
    for item in prior_payload["items"]
        .as_array()
        .expect("payload schema requires an item array")
    {
        let kind = string_at(item, &["kind"]).map_err(digest_failure)?;
        let obligation = string_at(item, &["obligation_sha256"]).map_err(digest_failure)?;
        if kind == "base_drift" {
            let evidence_sha256 = sha256_value(&item["evidence"]).map_err(digest_failure)?;
            if string_at(item, &["evidence_sha256"]).map_err(digest_failure)? != evidence_sha256
                || obligation != evidence_sha256
                || expected_prior_base_drift.as_ref().is_none_or(|expected| {
                    obligation != expected.0 || item["evidence"] != expected.1
                })
            {
                return Err((
                    ReceiptAbsentReason::DigestMismatch,
                    "prior payload contains an invalid base-drift digest".to_owned(),
                ));
            }
            continue;
        }
        if kind == "retained_non_current_obligation" {
            continue;
        }
        let identity_value = &item["identity"];
        let identity_sha256 = sha256_value(identity_value).map_err(digest_failure)?;
        if string_at(item, &["identity_sha256"]).map_err(digest_failure)? != identity_sha256
            || obligation != identity_sha256
        {
            return Err((
                ReceiptAbsentReason::DigestMismatch,
                "prior payload contains an invalid identity digest".to_owned(),
            ));
        }
        let identity: ExactGitChangeIdentity = serde_json::from_value(identity_value.clone())
            .map_err(|error| {
                (
                    ReceiptAbsentReason::DigestMismatch,
                    format!("prior payload identity is invalid: {error}"),
                )
            })?;
        validate_payload_source(
            &item["before_source"],
            identity.before_mode.as_deref(),
            identity.before_object_id.as_deref(),
        )
        .and_then(|()| {
            validate_payload_source(
                &item["after_source"],
                identity.after_mode.as_deref(),
                identity.after_object_id.as_deref(),
            )
        })
        .map_err(digest_failure)?;
        if kind == "current_change" && payload_current.insert(identity_sha256, identity).is_some() {
            return Err((
                ReceiptAbsentReason::DigestMismatch,
                "prior payload contains a duplicate current identity".to_owned(),
            ));
        }
    }
    let prior_git = discover_changes(
        repository,
        &transition.a.commit_oid,
        &transition.b.commit_oid,
    )
    .and_then(changes_by_digest)
    .map_err(digest_failure)?;
    let prior_git_ids = prior_git.keys().cloned().collect::<Vec<_>>();
    if string_array_at(
        &prior_input,
        &["resolution", "accounting", "full_current_identity_sha256"],
    )
    .map_err(digest_failure)?
        != prior_git_ids
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "prior input current identities do not match its reviewed Git range".to_owned(),
        ));
    }
    if !payload_current
        .keys()
        .all(|identity| prior_git.contains_key(identity))
    {
        return Err((
            ReceiptAbsentReason::DigestMismatch,
            "prior payload contains a current identity outside its reviewed Git range".to_owned(),
        ));
    }
    let covered_ids =
        string_array_at(body, &["coverage", "covered_identity_sha256"]).map_err(digest_failure)?;
    if covered_ids != prior_git_ids {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "receipt does not cover the complete reviewed current Git identity set".to_owned(),
        ));
    }
    let selected_current = ordered_string_set(
        &prior_input["resolution"]["accounting"],
        &["selected_current_identity_sha256"],
        "selected current identities",
    )
    .map_err(digest_failure)?;
    let input_carried = carried_outcomes(&prior_input).map_err(digest_failure)?;
    let identity_results = body["coverage"]["current_identity_results"]
        .as_array()
        .ok_or_else(|| {
            digest_failure(anyhow::anyhow!(
                "receipt current identity results are not an array"
            ))
        })?;
    let mut prior_item_outcomes = BTreeMap::new();
    let mut result_order = Vec::with_capacity(identity_results.len());
    for identity_result in identity_results {
        let identity = string_at(identity_result, &["identity_sha256"])
            .map_err(digest_failure)?
            .to_owned();
        let outcome = string_at(identity_result, &["outcome"])
            .map_err(digest_failure)?
            .to_owned();
        ReviewOutcome::parse(&outcome).map_err(digest_failure)?;
        if !prior_git.contains_key(&identity)
            || prior_item_outcomes
                .insert(identity.clone(), outcome.clone())
                .is_some()
        {
            return Err((
                ReceiptAbsentReason::IncompleteCoverage,
                "receipt current identity outcomes do not exactly match the reviewed Git range"
                    .to_owned(),
            ));
        }
        result_order.push(identity.clone());
        let lineage = &identity_result["lineage"];
        match string_at(lineage, &["kind"]).map_err(digest_failure)? {
            "selected_execution" => {
                if !selected_current.contains(&identity)
                    || string_at(lineage, &["obligation_sha256"]).map_err(digest_failure)?
                        != identity
                    || selected_item_outcomes.get(&identity) != Some(&outcome)
                    || !payload_current.contains_key(&identity)
                {
                    return Err((
                        ReceiptAbsentReason::IncompleteCoverage,
                        "selected current identity outcome is not backed by the bound reviewer execution"
                            .to_owned(),
                    ));
                }
            }
            "prior_receipt_exact_carry" => {
                let prior_reference = &prior_input["resolution"]["prior_receipt"];
                if input_carried.get(&identity) != Some(&outcome)
                    || !matches!(outcome.as_str(), "passed" | "advisory")
                    || string_at(lineage, &["prior_identity_sha256"]).map_err(digest_failure)?
                        != identity
                    || string_at(lineage, &["prior_receipt_body_sha256"]).map_err(digest_failure)?
                        != string_at(prior_reference, &["body_sha256"]).map_err(digest_failure)?
                {
                    return Err((
                        ReceiptAbsentReason::IncompleteCoverage,
                        "carried current identity outcome is not bound to the prior receipt lineage"
                            .to_owned(),
                    ));
                }
            }
            _ => {
                return Err((
                    ReceiptAbsentReason::DigestMismatch,
                    "receipt contains an unsupported current identity lineage".to_owned(),
                ));
            }
        }
    }
    if result_order != prior_git_ids {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "receipt current identity outcomes are not complete and canonically ordered".to_owned(),
        ));
    }
    let prior_decision =
        string_at(&prior_input, &["resolution", "decision"]).map_err(digest_failure)?;
    if !matches!(prior_decision, "full" | "residue") {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "a reusable receipt must bind a full or residue execution".to_owned(),
        ));
    }
    let selected_non_current_blocking_outcomes = selected_item_outcomes
        .iter()
        .filter(|(obligation, _)| !selected_current.contains(*obligation))
        .map(|(_, outcome)| outcome.as_str())
        .filter(|outcome| matches!(*outcome, "changes_requested" | "failed"));
    let retained_non_current_blocking_outcome =
        reduce_review_outcomes(selected_non_current_blocking_outcomes).map_err(digest_failure)?;
    if string_at(
        body,
        &["prior_result", "retained_non_current_blocking_outcome"],
    )
    .map_err(digest_failure)?
        != retained_non_current_blocking_outcome.as_str()
    {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "receipt retained non-current outcome does not conservatively preserve prior obligations"
                .to_owned(),
        ));
    }
    let mut blocking_non_current = BTreeMap::new();
    for item in prior_payload["items"]
        .as_array()
        .expect("payload schema requires an item array")
    {
        let obligation = string_at(item, &["obligation_sha256"]).map_err(digest_failure)?;
        let outcome = selected_item_outcomes
            .get(obligation)
            .expect("validated result covers every payload obligation");
        if selected_current.contains(obligation)
            || !matches!(outcome.as_str(), "changes_requested" | "failed")
        {
            continue;
        }
        let retained = if string_at(item, &["kind"]).map_err(digest_failure)?
            == "retained_non_current_obligation"
        {
            item.clone()
        } else {
            json!({
                "obligation_sha256": obligation,
                "kind": "retained_non_current_obligation",
                "origin_receipt_body_sha256": body_sha256,
                "original_item": item,
            })
        };
        if blocking_non_current
            .insert(obligation.to_owned(), retained)
            .is_some()
        {
            return Err((
                ReceiptAbsentReason::IncompleteCoverage,
                "receipt contains a duplicate blocking non-current obligation".to_owned(),
            ));
        }
    }
    let effective_outcome = reduce_review_outcomes(
        std::iter::once(string_at(&prior_result, &["outcome"]).map_err(digest_failure)?)
            .chain(input_carried.values().map(String::as_str)),
    )
    .map_err(digest_failure)?;
    if string_at(body, &["prior_result", "effective_outcome"]).map_err(digest_failure)?
        != effective_outcome.as_str()
    {
        return Err((
            ReceiptAbsentReason::IncompleteCoverage,
            "receipt effective outcome does not conservatively reduce selected and carried outcomes"
                .to_owned(),
        ));
    }
    let attestation_sha256 = sha256_value(attestation).map_err(digest_failure)?;
    let reference = json!({
        "status": "present",
        "schema_uri": REVIEW_RECEIPT_SCHEMA,
        "receipt_id": body["receipt_id"],
        "body_sha256": body_sha256,
        "attestation_sha256": attestation_sha256,
        "prior_input_sha256": prior_input_sha256,
        "prior_result_sha256": prior_result_sha256,
        "review_context_body_sha256": body["review_context_body_sha256"],
        "review_context_compatibility_sha256": body["review_context_compatibility_sha256"],
        "prior_outcome": body["prior_result"]["effective_outcome"],
        "retained_non_current_blocking_outcome": body["prior_result"]["retained_non_current_blocking_outcome"],
        "coverage_complete": true,
        "trusted_source_verified": true,
    });
    Ok(VerifiedReceipt {
        reference,
        body: body.clone(),
        prior_current: prior_git,
        prior_item_outcomes,
        prior_outcome: string_at(body, &["prior_result", "effective_outcome"])
            .map_err(digest_failure)?
            .to_owned(),
        blocking_non_current,
        reviewer_input_projection_sha256,
    })
}

fn digest_failure(error: anyhow::Error) -> (ReceiptAbsentReason, String) {
    (ReceiptAbsentReason::DigestMismatch, format!("{error:#}"))
}

fn build_blocked(
    generated_at: &str,
    context: &ContextFacts,
    transition: &Transition,
    receipt: Option<&ReviewCacheReceiptBundle<'_>>,
    decision_reason: &'static str,
    blocking_reason: String,
) -> Result<ReviewCacheArtifacts> {
    let blocking_reason = bounded_reason(&blocking_reason);
    let repository = context.value["body"]["repository"].clone();
    let pull_request = current_pull_request_reference(&context.value);
    let payload = build_payload(
        Path::new("."),
        ReviewCacheDecision::Blocked,
        context,
        transition,
        PayloadSelection {
            current: &BTreeMap::new(),
            retired: &BTreeMap::new(),
            base_drift: None,
            retained_non_current: &BTreeMap::new(),
        },
    )?;
    let payload_bytes = canonical_json_bytes(&payload)?;
    let payload_sha256 = sha256_bytes(&payload_bytes);
    let reviewer_input = reviewer_input_projection(&payload, context);
    let reviewer_input_bytes = canonical_json_bytes(&reviewer_input)?;
    let reviewer_input_projection_sha256 = sha256_bytes(&reviewer_input_bytes);
    let receipt_reason = if receipt.is_some() {
        ReceiptAbsentReason::IncompleteCoverage
    } else {
        ReceiptAbsentReason::NotProvided
    };
    let review_input = json!({
        "schema": REVIEW_INPUT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "generated_at": generated_at,
        "reuse_policy": "exact_git_change_identity_only_v1",
        "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
        "repository": repository,
        "pull_request": pull_request,
        "transition": transition_json(transition),
        "current_context": context_reference(context),
        "resolution": {
            "decision": "blocked",
            "reason": decision_reason,
            "prior_receipt": absent_receipt_reference(receipt_reason),
            "context_comparison": {
                "status": "unavailable",
                "closure_status": context.closure_status,
                "current_body_sha256": context.body_sha256,
                "current_compatibility_sha256": context.compatibility_sha256,
                "reason": "context_incomplete",
            },
            "selected_payload": {
                "mode": "blocked",
                "schema_uri": REVIEW_CACHE_PAYLOAD_SCHEMA,
                "canonical_payload_sha256": payload_sha256,
                "reviewer_input_projection_sha256": reviewer_input_projection_sha256,
                "byte_length": payload_bytes.len(),
                "selected_identity_sha256": [],
            },
            "accounting": {
                "coverage_complete": false,
                "full_current_identity_sha256": [],
                "carried": [],
                "selected_current_identity_sha256": [],
                "review_required": [],
                "unresolved_retired_obligation_sha256": [],
                "base_drift_obligation_sha256": [],
                "retained_non_current_obligation_sha256": [],
                "blocking_reasons": [blocking_reason],
            },
        },
    });
    validate_against_schema(&review_input, INPUT_SCHEMA_BYTES, "blocked review input")?;
    let review_input_bytes = canonical_json_bytes(&review_input)?;
    Ok(ReviewCacheArtifacts {
        decision: ReviewCacheDecision::Blocked,
        cached_outcome: None,
        review_input,
        review_input_bytes,
        selected_payload: payload,
        selected_payload_bytes: payload_bytes,
        reviewer_input,
        reviewer_input_bytes,
        receipt_notice: Some("review cache blocked before any input could be reused".to_owned()),
    })
}

fn build_payload(
    git_repository: &Path,
    decision: ReviewCacheDecision,
    context: &ContextFacts,
    transition: &Transition,
    selection: PayloadSelection<'_>,
) -> Result<Value> {
    let PayloadSelection {
        current,
        retired,
        base_drift,
        retained_non_current,
    } = selection;
    let mode = match decision {
        ReviewCacheDecision::Skip => "empty",
        ReviewCacheDecision::Residue => "residue",
        ReviewCacheDecision::Full => "full",
        ReviewCacheDecision::Blocked => "blocked",
    };
    let mut source_cache = HashMap::new();
    let mut source_bytes = 0_usize;
    let mut items = Vec::with_capacity(
        current.len()
            + retired.len()
            + usize::from(base_drift.is_some())
            + retained_non_current.len(),
    );
    for (kind, changes) in [
        ("current_change", current),
        ("retired_checkpoint_change", retired),
    ] {
        for (digest, identity) in changes {
            let before_source = materialize_source(
                git_repository,
                identity.before_mode.as_deref(),
                identity.before_object_id.as_deref(),
                &mut source_cache,
                &mut source_bytes,
            )?;
            let after_source = materialize_source(
                git_repository,
                identity.after_mode.as_deref(),
                identity.after_object_id.as_deref(),
                &mut source_cache,
                &mut source_bytes,
            )?;
            items.push(json!({
                "obligation_sha256": digest,
                "kind": kind,
                "identity_sha256": digest,
                "identity": identity,
                "before_source": before_source,
                "after_source": after_source,
            }));
        }
    }
    if let Some((digest, evidence)) = base_drift {
        items.push(json!({
            "obligation_sha256": digest,
            "kind": "base_drift",
            "evidence_sha256": digest,
            "evidence": evidence,
        }));
    }
    items.extend(retained_non_current.values().cloned());
    items.sort_by(|left, right| {
        left["obligation_sha256"]
            .as_str()
            .cmp(&right["obligation_sha256"].as_str())
    });
    let payload = json!({
        "schema": REVIEW_CACHE_PAYLOAD_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "mode": mode,
        "repository": context.value["body"]["repository"],
        "pull_request": current_pull_request_reference(&context.value),
        "base_commit": transition.c.commit_oid,
        "head_commit": transition.d.commit_oid,
        "items": items,
    });
    validate_against_schema(&payload, PAYLOAD_SCHEMA_BYTES, "selected review payload")?;
    validate_payload_items(&payload, None)?;
    Ok(payload)
}

fn base_drift_obligation(repository: &Path, transition: &Transition) -> Result<(String, Value)> {
    let changes = changes_by_digest(discover_changes(
        repository,
        &transition.a.commit_oid,
        &transition.c.commit_oid,
    )?)?;
    let mut source_cache = HashMap::new();
    let mut source_bytes = 0_usize;
    let mut materialized = Vec::with_capacity(changes.len());
    for (identity_sha256, identity) in changes {
        let before_source = materialize_source(
            repository,
            identity.before_mode.as_deref(),
            identity.before_object_id.as_deref(),
            &mut source_cache,
            &mut source_bytes,
        )?;
        let after_source = materialize_source(
            repository,
            identity.after_mode.as_deref(),
            identity.after_object_id.as_deref(),
            &mut source_cache,
            &mut source_bytes,
        )?;
        materialized.push(json!({
            "identity_sha256": identity_sha256,
            "identity": identity,
            "before_source": before_source,
            "after_source": after_source,
        }));
    }
    let evidence = json!({
        "kind": "base_drift_v1",
        "old_base_commit": transition.a.commit_oid,
        "old_base_tree": transition.a.tree_oid,
        "current_base_commit": transition.c.commit_oid,
        "current_base_tree": transition.c.tree_oid,
        "changes": materialized,
    });
    let digest = sha256_value(&evidence)?;
    Ok((digest, evidence))
}

fn materialize_source(
    repository: &Path,
    mode: Option<&str>,
    object_id: Option<&str>,
    cache: &mut HashMap<String, Value>,
    total_bytes: &mut usize,
) -> Result<Value> {
    match (mode, object_id) {
        (None, None) => Ok(json!({"kind": "absent"})),
        (Some("160000"), Some(object_id)) => Ok(json!({
            "kind": "gitlink",
            "object_id": object_id,
        })),
        (Some(_), Some(object_id)) => {
            if let Some(source) = cache.get(object_id) {
                *total_bytes = total_bytes
                    .checked_add(usize_at(source, &["byte_length"])?)
                    .context("selected source byte count overflow")?;
                ensure!(
                    *total_bytes <= MAX_SELECTED_SOURCE_BYTES,
                    "selected source bytes exceed the {MAX_SELECTED_SOURCE_BYTES} byte limit"
                );
                return Ok(source.clone());
            }
            let remaining = MAX_SELECTED_SOURCE_BYTES.saturating_sub(*total_bytes);
            let source =
                git_output_bounded(repository, &["cat-file", "blob", object_id], remaining)?.stdout;
            *total_bytes =
                checked_source_budget(*total_bytes, source.len(), MAX_SELECTED_SOURCE_BYTES)?;
            let value = json!({
                "kind": "git_blob",
                "object_id": object_id,
                "byte_length": source.len(),
                "sha256": sha256_bytes(&source),
                "content_base64": STANDARD.encode(&source),
            });
            cache.insert(object_id.to_owned(), value.clone());
            Ok(value)
        }
        _ => bail!("Git identity contains an inconsistent source mode and object ID"),
    }
}

fn checked_source_budget(current: usize, additional: usize, limit: usize) -> Result<usize> {
    let total = current
        .checked_add(additional)
        .context("selected source byte count overflow")?;
    ensure!(
        total <= limit,
        "selected source bytes exceed the {limit} byte limit"
    );
    Ok(total)
}

fn validate_payload_source(
    source: &Value,
    expected_mode: Option<&str>,
    expected_object_id: Option<&str>,
) -> Result<()> {
    match (expected_mode, expected_object_id) {
        (None, None) => ensure!(
            string_at(source, &["kind"])? == "absent",
            "absent Git identity has a materialized source"
        ),
        (Some("160000"), Some(object_id)) => {
            ensure!(
                string_at(source, &["kind"])? == "gitlink"
                    && string_at(source, &["object_id"])? == object_id,
                "Gitlink payload source does not match its identity"
            );
        }
        (Some(_), Some(object_id)) => {
            ensure!(
                string_at(source, &["kind"])? == "git_blob"
                    && string_at(source, &["object_id"])? == object_id,
                "Git blob payload source does not match its identity"
            );
            let encoded = string_at(source, &["content_base64"])?;
            let bytes = STANDARD
                .decode(encoded)
                .context("Git blob payload source is not canonical Base64")?;
            ensure!(
                STANDARD.encode(&bytes) == encoded,
                "Git blob payload source is not canonical Base64"
            );
            ensure!(
                usize_at(source, &["byte_length"])? == bytes.len()
                    && string_at(source, &["sha256"])? == sha256_bytes(&bytes),
                "Git blob payload source digest or length differs"
            );
        }
        _ => bail!("Git identity contains an inconsistent source mode and object ID"),
    }
    Ok(())
}

fn validate_payload_items(payload: &Value, repository: Option<&Path>) -> Result<()> {
    let items = payload["items"]
        .as_array()
        .context("selected payload items are not an array")?;
    let mut materialized_objects = BTreeMap::<String, Vec<u8>>::new();
    let mut total_source_bytes = 0_usize;

    for item in items {
        let outer_obligation = string_at(item, &["obligation_sha256"])?;
        let item = if string_at(item, &["kind"])? == "retained_non_current_obligation" {
            let original = &item["original_item"];
            ensure!(
                outer_obligation == string_at(original, &["obligation_sha256"])?
                    && matches!(
                        string_at(original, &["kind"])?,
                        "retired_checkpoint_change" | "base_drift"
                    ),
                "retained non-current obligation does not bind its original item"
            );
            original
        } else {
            item
        };
        let kind = string_at(item, &["kind"])?;
        let obligation = string_at(item, &["obligation_sha256"])?;
        if kind == "base_drift" {
            let evidence_sha256 = sha256_value(&item["evidence"])?;
            ensure!(
                string_at(item, &["evidence_sha256"])? == evidence_sha256
                    && obligation == evidence_sha256,
                "selected payload contains an invalid base-drift digest"
            );
            let changes = item["evidence"]["changes"]
                .as_array()
                .context("base-drift evidence changes are not an array")?;
            let mut identity_order = Vec::with_capacity(changes.len());
            for change in changes {
                identity_order.push(validate_nested_materialized_change(
                    change,
                    repository,
                    &mut materialized_objects,
                    &mut total_source_bytes,
                )?);
            }
            ensure!(
                identity_order.windows(2).all(|pair| pair[0] < pair[1]),
                "base-drift identities are not in strict canonical order"
            );
            continue;
        }

        ensure!(
            matches!(kind, "current_change" | "retired_checkpoint_change"),
            "selected payload contains an unsupported item kind"
        );
        let identity_value = &item["identity"];
        let identity_sha256 = sha256_value(identity_value)?;
        ensure!(
            string_at(item, &["identity_sha256"])? == identity_sha256
                && obligation == identity_sha256,
            "selected payload contains an invalid identity digest"
        );
        let identity: ExactGitChangeIdentity = serde_json::from_value(identity_value.clone())
            .context("selected payload identity is invalid")?;
        validate_identity_shape(&identity)?;
        validate_payload_source(
            &item["before_source"],
            identity.before_mode.as_deref(),
            identity.before_object_id.as_deref(),
        )?;
        validate_payload_source(
            &item["after_source"],
            identity.after_mode.as_deref(),
            identity.after_object_id.as_deref(),
        )?;
        for source in [&item["before_source"], &item["after_source"]] {
            if string_at(source, &["kind"])? != "git_blob" {
                continue;
            }
            let object_id = string_at(source, &["object_id"])?;
            let bytes = STANDARD
                .decode(string_at(source, &["content_base64"])?)
                .context("Git blob payload source is not canonical Base64")?;
            total_source_bytes =
                checked_source_budget(total_source_bytes, bytes.len(), MAX_SELECTED_SOURCE_BYTES)?;
            if let Some(existing) = materialized_objects.get(object_id) {
                ensure!(
                    existing == &bytes,
                    "selected payload materializes one Git object with different bytes"
                );
                continue;
            }
            if let Some(repository) = repository {
                let expected = git_output_bounded(
                    repository,
                    &["cat-file", "blob", object_id],
                    MAX_SELECTED_SOURCE_BYTES,
                )?
                .stdout;
                ensure!(
                    expected == bytes,
                    "selected payload bytes differ from Git blob {object_id}"
                );
            }
            materialized_objects.insert(object_id.to_owned(), bytes);
        }
    }
    payload_item_ids(payload)?;
    Ok(())
}

fn validate_nested_materialized_change(
    item: &Value,
    repository: Option<&Path>,
    materialized_objects: &mut BTreeMap<String, Vec<u8>>,
    total_source_bytes: &mut usize,
) -> Result<String> {
    let identity_value = &item["identity"];
    let identity_sha256 = sha256_value(identity_value)?;
    ensure!(
        string_at(item, &["identity_sha256"])? == identity_sha256,
        "base-drift payload contains an invalid identity digest"
    );
    let identity: ExactGitChangeIdentity = serde_json::from_value(identity_value.clone())
        .context("base-drift payload identity is invalid")?;
    validate_identity_shape(&identity)?;
    validate_payload_source(
        &item["before_source"],
        identity.before_mode.as_deref(),
        identity.before_object_id.as_deref(),
    )?;
    validate_payload_source(
        &item["after_source"],
        identity.after_mode.as_deref(),
        identity.after_object_id.as_deref(),
    )?;
    for source in [&item["before_source"], &item["after_source"]] {
        if string_at(source, &["kind"])? != "git_blob" {
            continue;
        }
        let object_id = string_at(source, &["object_id"])?;
        let bytes = STANDARD
            .decode(string_at(source, &["content_base64"])?)
            .context("Git blob payload source is not canonical Base64")?;
        *total_source_bytes =
            checked_source_budget(*total_source_bytes, bytes.len(), MAX_SELECTED_SOURCE_BYTES)?;
        if let Some(existing) = materialized_objects.get(object_id) {
            ensure!(
                existing == &bytes,
                "selected payload materializes one Git object with different bytes"
            );
            continue;
        }
        if let Some(repository) = repository {
            let expected = git_output_bounded(
                repository,
                &["cat-file", "blob", object_id],
                MAX_SELECTED_SOURCE_BYTES,
            )?
            .stdout;
            ensure!(
                expected == bytes,
                "selected payload bytes differ from Git blob {object_id}"
            );
        }
        materialized_objects.insert(object_id.to_owned(), bytes);
    }
    Ok(identity_sha256)
}

fn validate_identity_shape(identity: &ExactGitChangeIdentity) -> Result<()> {
    for path in [
        identity.before_path_base64.as_deref(),
        identity.after_path_base64.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let bytes = STANDARD
            .decode(path)
            .context("Git identity path is not canonical Base64")?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= 4096 && STANDARD.encode(&bytes) == path,
            "Git identity path is empty, oversized, or non-canonical Base64"
        );
    }
    let before_complete = identity.before_path_base64.is_some()
        && identity.before_mode.is_some()
        && identity.before_object_id.is_some();
    let after_complete = identity.after_path_base64.is_some()
        && identity.after_mode.is_some()
        && identity.after_object_id.is_some();
    let before_absent = identity.before_path_base64.is_none()
        && identity.before_mode.is_none()
        && identity.before_object_id.is_none();
    let after_absent = identity.after_path_base64.is_none()
        && identity.after_mode.is_none()
        && identity.after_object_id.is_none();
    match identity.status {
        ChangeStatus::Added => ensure!(
            before_absent && after_complete && identity.similarity_percent.is_none(),
            "added Git identity has inconsistent fields"
        ),
        ChangeStatus::Deleted => ensure!(
            before_complete && after_absent && identity.similarity_percent.is_none(),
            "deleted Git identity has inconsistent fields"
        ),
        ChangeStatus::Modified | ChangeStatus::TypeChanged => ensure!(
            before_complete && after_complete && identity.similarity_percent.is_none(),
            "modified Git identity has inconsistent fields"
        ),
        ChangeStatus::Copied | ChangeStatus::Renamed => ensure!(
            before_complete && after_complete && identity.similarity_percent.is_some(),
            "copied or renamed Git identity has inconsistent fields"
        ),
    }
    Ok(())
}

fn validate_review_input_accounting(
    review_input: &Value,
    payload: &Value,
    payload_ids: &[String],
) -> Result<()> {
    let accounting = &review_input["resolution"]["accounting"];
    let full_current = ordered_string_set(
        accounting,
        &["full_current_identity_sha256"],
        "full current identities",
    )?;
    let selected_current = ordered_string_set(
        accounting,
        &["selected_current_identity_sha256"],
        "selected current identities",
    )?;
    let retired = ordered_string_set(
        accounting,
        &["unresolved_retired_obligation_sha256"],
        "retired obligations",
    )?;
    let base_drift = ordered_string_set(
        accounting,
        &["base_drift_obligation_sha256"],
        "base-drift obligations",
    )?;
    let retained_non_current = ordered_string_set(
        accounting,
        &["retained_non_current_obligation_sha256"],
        "retained non-current obligations",
    )?;

    let carried_values = accounting["carried"]
        .as_array()
        .context("review input carried identities are not an array")?;
    let mut carried = BTreeSet::new();
    let mut carried_order = Vec::with_capacity(carried_values.len());
    for item in carried_values {
        let current = string_at(item, &["current_identity_sha256"])?;
        ensure!(
            current == string_at(item, &["receipt_identity_sha256"])?
                && string_at(item, &["basis"])? == "exact_git_change_identity",
            "review input contains a non-exact carry"
        );
        ensure!(
            carried.insert(current.to_owned()),
            "review input contains duplicate carried identities"
        );
        carried_order.push(current.to_owned());
    }
    ensure!(
        carried_order.windows(2).all(|pair| pair[0] < pair[1]),
        "review input carried identities are not in strict canonical order"
    );

    let requirements = accounting["review_required"]
        .as_array()
        .context("review input requirements are not an array")?;
    let mut required = BTreeSet::new();
    let mut required_order = Vec::with_capacity(requirements.len());
    for item in requirements {
        let obligation = string_at(item, &["obligation_sha256"])?;
        ensure!(
            required.insert(obligation.to_owned()),
            "review input contains duplicate review obligations"
        );
        required_order.push(obligation.to_owned());
    }
    ensure!(
        required_order.windows(2).all(|pair| pair[0] < pair[1]),
        "review input obligations are not in strict canonical order"
    );

    ensure!(
        carried.is_disjoint(&selected_current),
        "carried and selected current identities overlap"
    );
    let reconstructed_current = carried
        .union(&selected_current)
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        reconstructed_current == full_current,
        "full current identities do not equal carried plus selected current identities"
    );

    let mut expected_payload = selected_current.clone();
    ensure!(
        expected_payload.is_disjoint(&retired)
            && expected_payload.is_disjoint(&base_drift)
            && expected_payload.is_disjoint(&retained_non_current)
            && retired.is_disjoint(&base_drift),
        "selected current and unresolved obligation classes overlap"
    );
    ensure!(
        retired.is_disjoint(&retained_non_current) && base_drift.is_disjoint(&retained_non_current),
        "current and retained obligation classes overlap"
    );
    expected_payload.extend(retired.iter().cloned());
    expected_payload.extend(base_drift.iter().cloned());
    expected_payload.extend(retained_non_current.iter().cloned());
    let payload_set = payload_ids.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        payload_set.len() == payload_ids.len() && payload_set == expected_payload,
        "selected payload identities do not equal selected current plus unresolved obligations"
    );
    ensure!(
        required == payload_set,
        "review requirements do not exactly cover the selected payload"
    );
    ensure!(
        string_array_at(
            review_input,
            &["resolution", "selected_payload", "selected_identity_sha256"],
        )? == payload_ids,
        "review input selected identity list differs from the selected payload"
    );

    let mut payload_current = BTreeSet::new();
    let mut payload_retired = BTreeSet::new();
    let mut payload_base_drift = BTreeSet::new();
    let mut payload_retained_non_current = BTreeSet::new();
    for item in payload["items"]
        .as_array()
        .context("selected payload items are not an array")?
    {
        let obligation = string_at(item, &["obligation_sha256"])?.to_owned();
        match string_at(item, &["kind"])? {
            "current_change" => {
                payload_current.insert(obligation);
            }
            "retired_checkpoint_change" => {
                payload_retired.insert(obligation);
            }
            "base_drift" => {
                payload_base_drift.insert(obligation);
            }
            "retained_non_current_obligation" => {
                payload_retained_non_current.insert(obligation);
            }
            _ => bail!("selected payload contains an unsupported item kind"),
        }
    }
    ensure!(
        payload_current == selected_current
            && payload_retired == retired
            && payload_base_drift == base_drift
            && payload_retained_non_current == retained_non_current,
        "selected payload item kinds differ from review input accounting"
    );
    Ok(())
}

fn validate_result_manifest(
    result: &Value,
    review_input_sha256: &str,
    selected_payload_sha256: &str,
    reviewer_input_projection_sha256: &str,
    payload_ids: &[String],
) -> Result<()> {
    ensure!(
        string_at(result, &["review_input_sha256"])? == review_input_sha256
            && string_at(result, &["selected_payload_sha256"])? == selected_payload_sha256,
        "review result does not bind the supplied review input and selected payload"
    );
    ensure!(
        string_at(result, &["reviewer_input_projection_sha256"])?
            == reviewer_input_projection_sha256,
        "review result does not bind the reviewer-visible input projection"
    );
    ensure!(
        result["coverage"]["complete"].as_bool() == Some(true)
            && result["coverage"]["omitted_obligation_sha256"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && result["coverage"]["unresolved_obligation_sha256"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "review result does not assert complete coverage"
    );
    let covered = string_array_at(result, &["coverage", "covered_obligation_sha256"])?;
    ensure!(
        covered.windows(2).all(|pair| pair[0] < pair[1]) && covered == payload_ids,
        "review result coverage does not exactly match the selected payload"
    );
    let item_outcomes = result_item_outcomes(result, payload_ids)?;
    let reduced = reduce_review_outcomes(item_outcomes.values().map(String::as_str))?;
    ensure!(
        string_at(result, &["outcome"])? == reduced.as_str(),
        "review result outcome does not equal the built-in conservative reduction of obligation outcomes"
    );
    Ok(())
}

fn reduce_review_outcomes<'a>(
    outcomes: impl IntoIterator<Item = &'a str>,
) -> Result<ReviewOutcome> {
    let mut reduced = ReviewOutcome::Passed;
    for outcome in outcomes {
        let outcome = ReviewOutcome::parse(outcome)?;
        if outcome.severity() > reduced.severity() {
            reduced = outcome;
        }
    }
    Ok(reduced)
}

fn cross_item_aggregator_binding() -> Result<Value> {
    let contract = parse_strict_json(
        CROSS_ITEM_AGGREGATOR_BYTES.as_bytes(),
        "built-in cross-item aggregator contract",
    )?;
    ensure!(
        string_at(&contract, &["schema"])? == REVIEW_CACHE_CROSS_ITEM_AGGREGATOR_SCHEMA,
        "built-in cross-item aggregator schema URI differs"
    );
    let bytes = canonical_json_bytes(&contract)?;
    Ok(json!({
        "schema_uri": REVIEW_CACHE_CROSS_ITEM_AGGREGATOR_SCHEMA,
        "canonical_sha256": sha256_bytes(&bytes),
        "byte_length": bytes.len(),
    }))
}

fn receipt_signature_preimage(body_sha256: &str) -> Result<Vec<u8>> {
    let digest = decode_hex_array::<32>(body_sha256)?;
    let mut preimage = Vec::with_capacity(
        RECEIPT_SIGNATURE_DOMAIN.len()
            + RECEIPT_SIGNATURE_PREIMAGE_VERSION.len()
            + digest.len()
            + 2,
    );
    preimage.extend_from_slice(RECEIPT_SIGNATURE_DOMAIN.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(RECEIPT_SIGNATURE_PREIMAGE_VERSION.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&digest);
    Ok(preimage)
}

fn carried_outcomes(review_input: &Value) -> Result<BTreeMap<String, String>> {
    let values = review_input["resolution"]["accounting"]["carried"]
        .as_array()
        .context("review input carried identities are not an array")?;
    let mut outcomes = BTreeMap::new();
    for value in values {
        let identity = string_at(value, &["current_identity_sha256"])?.to_owned();
        let outcome = string_at(value, &["prior_outcome"])?.to_owned();
        ensure!(
            outcomes.insert(identity, outcome).is_none(),
            "review input contains duplicate carried identities"
        );
    }
    Ok(outcomes)
}

fn result_item_outcomes(
    result: &Value,
    payload_ids: &[String],
) -> Result<BTreeMap<String, String>> {
    let values = result["obligation_results"]
        .as_array()
        .context("review result obligation results are not an array")?;
    let mut outcomes = BTreeMap::new();
    let mut order = Vec::with_capacity(values.len());
    for value in values {
        let obligation = string_at(value, &["obligation_sha256"])?.to_owned();
        let outcome = string_at(value, &["outcome"])?.to_owned();
        ensure!(
            outcomes.insert(obligation.clone(), outcome).is_none(),
            "review result contains duplicate obligation outcomes"
        );
        order.push(obligation);
    }
    ensure!(
        order.windows(2).all(|pair| pair[0] < pair[1]) && order == payload_ids,
        "review result obligation outcomes do not exactly match the selected payload"
    );
    Ok(outcomes)
}

fn ordered_string_set(value: &Value, path: &[&str], label: &str) -> Result<BTreeSet<String>> {
    let values = string_array_at(value, path)?;
    ensure!(
        values.windows(2).all(|pair| pair[0] < pair[1]),
        "{label} are not in strict canonical order"
    );
    Ok(values.into_iter().collect())
}

fn payload_item_ids(payload: &Value) -> Result<Vec<String>> {
    let items = payload["items"]
        .as_array()
        .context("selected payload items are not an array")?;
    let mut ids = Vec::with_capacity(items.len());
    let mut unique = BTreeSet::new();
    for item in items {
        let id = string_at(item, &["obligation_sha256"])?.to_owned();
        ensure!(
            unique.insert(id.clone()),
            "selected payload contains duplicate obligation identities"
        );
        ids.push(id);
    }
    ensure!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "selected payload identities are not in strict canonical order"
    );
    Ok(ids)
}

fn reviewer_input_projection(payload: &Value, context: &ContextFacts) -> Value {
    json!({
        "schema": REVIEW_CACHE_REVIEWER_INPUT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "composition": context.value["body"]["reviewer"]["composition"],
        "repository": payload["repository"],
        "pull_request": payload["pull_request"],
        "items": payload["items"],
    })
}

fn changes_by_digest(
    changes: Vec<ExactGitChangeIdentity>,
) -> Result<BTreeMap<String, ExactGitChangeIdentity>> {
    let mut result = BTreeMap::new();
    for identity in changes {
        let digest = sha256_value(&serde_json::to_value(&identity)?)?;
        ensure!(
            result.insert(digest, identity).is_none(),
            "current Git diff contains a duplicate exact change identity"
        );
    }
    Ok(result)
}

fn resolve_transition(
    repository: &Path,
    requested_base: &str,
    checkpoint: &str,
    head: &str,
) -> Result<Transition> {
    ensure!(
        git_text(repository, &["rev-parse", "--is-shallow-repository"])? == "false",
        "review cache requires complete Git ancestry"
    );
    let q = resolve_snapshot(repository, requested_base, "requested base")?;
    let b = resolve_snapshot(repository, checkpoint, "review checkpoint")?;
    let d = resolve_snapshot(repository, head, "current head")?;
    let a_commit = unique_merge_base(repository, &q.commit_oid, &b.commit_oid, "checkpoint")?;
    let c_commit = unique_merge_base(repository, &q.commit_oid, &d.commit_oid, "current head")?;
    let a = resolve_snapshot(repository, &a_commit, "checkpoint merge base")?;
    let c = resolve_snapshot(repository, &c_commit, "current merge base")?;
    Ok(Transition { q, a, b, c, d })
}

fn resolve_snapshot(repository: &Path, object_id: &str, label: &str) -> Result<Snapshot> {
    ensure!(
        is_sha1(object_id),
        "{label} is not an exact SHA-1 object ID"
    );
    let commit_spec = format!("{object_id}^{{commit}}");
    let commit_oid = git_text(
        repository,
        &["rev-parse", "--verify", "--end-of-options", &commit_spec],
    )?;
    ensure!(
        commit_oid == object_id,
        "{label} resolved to {commit_oid}, expected {object_id}"
    );
    let tree_spec = format!("{object_id}^{{tree}}");
    let tree_oid = git_text(
        repository,
        &["rev-parse", "--verify", "--end-of-options", &tree_spec],
    )?;
    ensure!(is_sha1(&tree_oid), "{label} tree is not a SHA-1 object ID");
    Ok(Snapshot {
        commit_oid,
        tree_oid,
    })
}

fn unique_merge_base(repository: &Path, left: &str, right: &str, label: &str) -> Result<String> {
    let output = git_text(repository, &["merge-base", "--all", left, right])?;
    let values = output.lines().collect::<Vec<_>>();
    ensure!(
        values.len() == 1,
        "{label} requires exactly one merge base, found {}",
        values.len()
    );
    ensure!(
        is_sha1(values[0]),
        "{label} merge base is not a SHA-1 object ID"
    );
    Ok(values[0].to_owned())
}

fn discover_changes(
    repository: &Path,
    base: &str,
    head: &str,
) -> Result<Vec<ExactGitChangeIdentity>> {
    let output = git_output_bounded(
        repository,
        &[
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--ignore-submodules=none",
            base,
            head,
            "--",
        ],
        MAX_DIFF_BYTES,
    )?;
    ensure!(
        output.stderr.is_empty()
            || output.stderr
                == b"warning: lazy fetching disabled; some objects may not be available\n",
        "git diff produced diagnostics: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let changes = parse_raw_diff(&output.stdout)?;
    pair_exact_relocations(changes)
}

fn parse_raw_diff(bytes: &[u8]) -> Result<Vec<ExactGitChangeIdentity>> {
    let mut fields = bytes.split(|byte| *byte == 0).peekable();
    let mut changes = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            ensure!(fields.peek().is_none(), "unexpected empty git diff record");
            break;
        }
        let header = std::str::from_utf8(header).context("git diff header is not UTF-8")?;
        let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
        ensure!(
            columns.len() == 5,
            "unexpected git raw diff header: {header}"
        );
        let before_mode = nonzero(
            columns[0]
                .strip_prefix(':')
                .context("missing git mode prefix")?,
        );
        let after_mode = nonzero(columns[1]);
        let before_object_id = nonzero(columns[2]);
        let after_object_id = nonzero(columns[3]);
        for object_id in before_object_id.iter().chain(after_object_id.iter()) {
            ensure!(
                is_sha1(object_id),
                "git diff returned a non-SHA-1 object ID"
            );
        }
        for mode in before_mode.iter().chain(after_mode.iter()) {
            ensure!(is_mode(mode), "git diff returned an invalid file mode");
        }
        let status_text = columns[4];
        let status_code = status_text
            .bytes()
            .next()
            .context("missing git diff status")?;
        let (status, similarity_percent) = match status_code {
            b'A' => (ChangeStatus::Added, None),
            b'C' => (ChangeStatus::Copied, Some(parse_similarity(status_text)?)),
            b'D' => (ChangeStatus::Deleted, None),
            b'M' => (ChangeStatus::Modified, None),
            b'R' => (ChangeStatus::Renamed, Some(parse_similarity(status_text)?)),
            b'T' => (ChangeStatus::TypeChanged, None),
            _ => bail!("unsupported git diff status: {status_text}"),
        };
        let first_path = next_path(&mut fields)?;
        let (before_path_base64, after_path_base64) = match status {
            ChangeStatus::Added => (None, Some(first_path)),
            ChangeStatus::Deleted => (Some(first_path), None),
            ChangeStatus::Copied | ChangeStatus::Renamed => {
                (Some(first_path), Some(next_path(&mut fields)?))
            }
            ChangeStatus::Modified | ChangeStatus::TypeChanged => {
                (Some(first_path.clone()), Some(first_path))
            }
        };
        match status {
            ChangeStatus::Added => ensure!(
                before_mode.is_none()
                    && before_object_id.is_none()
                    && after_mode.is_some()
                    && after_object_id.is_some(),
                "added Git record has inconsistent object metadata"
            ),
            ChangeStatus::Deleted => ensure!(
                before_mode.is_some()
                    && before_object_id.is_some()
                    && after_mode.is_none()
                    && after_object_id.is_none(),
                "deleted Git record has inconsistent object metadata"
            ),
            ChangeStatus::Copied
            | ChangeStatus::Modified
            | ChangeStatus::Renamed
            | ChangeStatus::TypeChanged => ensure!(
                before_mode.is_some()
                    && before_object_id.is_some()
                    && after_mode.is_some()
                    && after_object_id.is_some(),
                "paired Git record has inconsistent object metadata"
            ),
        }
        changes.push(ExactGitChangeIdentity {
            status,
            similarity_percent,
            before_path_base64,
            after_path_base64,
            before_mode,
            after_mode,
            before_object_id,
            after_object_id,
        });
        ensure!(
            changes.len() <= MAX_IDENTITIES,
            "changed identity limit exceeded"
        );
    }
    Ok(changes)
}

fn pair_exact_relocations(
    changes: Vec<ExactGitChangeIdentity>,
) -> Result<Vec<ExactGitChangeIdentity>> {
    let mut candidates = HashMap::<(String, String), (Vec<usize>, Vec<usize>)>::new();
    for (index, change) in changes.iter().enumerate() {
        match change.status {
            ChangeStatus::Deleted => {
                if let (Some(object_id), Some(mode)) =
                    (&change.before_object_id, &change.before_mode)
                {
                    candidates
                        .entry((object_id.clone(), mode.clone()))
                        .or_default()
                        .0
                        .push(index);
                }
            }
            ChangeStatus::Added => {
                if let (Some(object_id), Some(mode)) = (&change.after_object_id, &change.after_mode)
                {
                    candidates
                        .entry((object_id.clone(), mode.clone()))
                        .or_default()
                        .1
                        .push(index);
                }
            }
            ChangeStatus::Copied
            | ChangeStatus::Modified
            | ChangeStatus::Renamed
            | ChangeStatus::TypeChanged => {}
        }
    }
    let pairs = candidates
        .into_values()
        .filter_map(|(deleted, added)| {
            (deleted.len() == 1 && added.len() == 1).then(|| (deleted[0], added[0]))
        })
        .collect::<Vec<_>>();
    let mut changes = changes.into_iter().map(Some).collect::<Vec<_>>();
    for (deleted_index, added_index) in pairs {
        let deleted = changes[deleted_index]
            .take()
            .context("deleted relocation candidate is missing")?;
        let added = changes[added_index]
            .take()
            .context("added relocation candidate is missing")?;
        changes[deleted_index.min(added_index)] = Some(ExactGitChangeIdentity {
            status: ChangeStatus::Renamed,
            similarity_percent: Some(100),
            before_path_base64: deleted.before_path_base64,
            after_path_base64: added.after_path_base64,
            before_mode: deleted.before_mode,
            after_mode: added.after_mode,
            before_object_id: deleted.before_object_id,
            after_object_id: added.after_object_id,
        });
    }
    let mut changes = changes.into_iter().flatten().collect::<Vec<_>>();
    changes.sort();
    Ok(changes)
}

fn verify_change_objects(repository: &Path, changes: &[ExactGitChangeIdentity]) -> Result<()> {
    let mut objects = BTreeSet::new();
    for change in changes {
        for (mode, object_id) in [
            (&change.before_mode, &change.before_object_id),
            (&change.after_mode, &change.after_object_id),
        ] {
            if let (Some(mode), Some(object_id)) = (mode, object_id)
                && mode != "160000"
            {
                objects.insert(object_id.as_str());
            }
        }
    }
    for object_id in objects {
        let object_spec = format!("{object_id}^{{blob}}");
        git_output_bounded(
            repository,
            &["cat-file", "-e", &object_spec],
            MAX_GIT_DIAGNOSTIC_BYTES,
        )?;
    }
    Ok(())
}

fn transition_json(transition: &Transition) -> Value {
    json!({
        "route": "full_history_verified",
        "Q": snapshot_json(&transition.q),
        "A": snapshot_json(&transition.a),
        "B": snapshot_json(&transition.b),
        "C": snapshot_json(&transition.c),
        "D": snapshot_json(&transition.d),
    })
}

fn snapshot_json(snapshot: &Snapshot) -> Value {
    json!({
        "commit_oid": snapshot.commit_oid,
        "tree_oid": snapshot.tree_oid,
    })
}

fn current_pull_request_reference(context: &Value) -> Value {
    json!({
        "node_id": context["body"]["pull_request"]["node_id"],
        "number": context["body"]["pull_request"]["number"],
        "metadata_sha256": context["body"]["pull_request"]["canonical_metadata_sha256"],
    })
}

fn context_reference(context: &ContextFacts) -> Value {
    json!({
        "schema_uri": REVIEW_CONTEXT_SCHEMA,
        "body_sha256": context.body_sha256,
        "compatibility_sha256": context.compatibility_sha256,
        "closure_status": context.closure_status,
    })
}

fn exact_context_comparison(context: &ContextFacts, receipt_body: &Value) -> Result<Value> {
    Ok(json!({
        "status": "exact",
        "closure_status": "closed",
        "current_body_sha256": context.body_sha256,
        "receipt_body_sha256": string_at(receipt_body, &["review_context_body_sha256"] )?,
        "current_compatibility_sha256": context.compatibility_sha256,
        "receipt_compatibility_sha256": string_at(receipt_body, &["review_context_compatibility_sha256"] )?,
    }))
}

fn mismatch_context_comparison(context: &ContextFacts, receipt_body: &Value) -> Result<Value> {
    Ok(json!({
        "status": "mismatch",
        "closure_status": context.closure_status,
        "current_body_sha256": context.body_sha256,
        "receipt_body_sha256": string_at(receipt_body, &["review_context_body_sha256"] )?,
        "current_compatibility_sha256": context.compatibility_sha256,
        "receipt_compatibility_sha256": string_at(receipt_body, &["review_context_compatibility_sha256"] )?,
    }))
}

fn unavailable_context_comparison(context: &ContextFacts) -> Value {
    let reason = match context.open_reason.as_deref() {
        Some("hosted_model_unpinned") => "hosted_model_unpinned",
        Some(_) => "open_dependency_closure",
        None => "receipt_absent",
    };
    json!({
        "status": "unavailable",
        "closure_status": context.closure_status,
        "current_body_sha256": context.body_sha256,
        "current_compatibility_sha256": context.compatibility_sha256,
        "reason": reason,
    })
}

fn absent_receipt_reference(reason: ReceiptAbsentReason) -> Value {
    json!({"status": "absent", "reason": reason.as_str()})
}

fn parse_strict_json(bytes: &[u8], label: &str) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)
        .with_context(|| format!("failed to decode {label}"))?;
    deserializer
        .end()
        .with_context(|| format!("{label} contains trailing data"))?;
    Ok(value.0)
}

fn validate_against_schema(value: &Value, schema_bytes: &str, label: &str) -> Result<()> {
    let schema: Value = serde_json::from_str(schema_bytes)
        .with_context(|| format!("failed to decode embedded {label} schema"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .with_context(|| format!("embedded {label} schema is invalid"))?;
    if let Some(error) = validator.iter_errors(value).next() {
        bail!("{label} violates its schema: {error}");
    }
    Ok(())
}

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_writer(output, value)?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_canonical_json(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn sha256_value(value: &Value) -> Result<String> {
    Ok(sha256_bytes(&canonical_json_bytes(value)?))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonically_strictly_ordered(value: &Value) -> Result<bool> {
    let values = value.as_array().context("ordered value is not an array")?;
    let encoded = values
        .iter()
        .map(canonical_json_bytes)
        .collect::<Result<Vec<_>>>()?;
    Ok(encoded.windows(2).all(|pair| pair[0] < pair[1]))
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str> {
    let mut current = value;
    for component in path {
        current = current
            .as_object()
            .and_then(|object| object.get(*component))
            .with_context(|| format!("missing JSON field {}", path.join(".")))?;
    }
    current
        .as_str()
        .with_context(|| format!("JSON field {} is not a string", path.join(".")))
}

fn usize_at(value: &Value, path: &[&str]) -> Result<usize> {
    let mut current = value;
    for component in path {
        current = current
            .as_object()
            .and_then(|object| object.get(*component))
            .with_context(|| format!("missing JSON field {}", path.join(".")))?;
    }
    let value = current
        .as_u64()
        .with_context(|| format!("JSON field {} is not an unsigned integer", path.join(".")))?;
    usize::try_from(value).with_context(|| format!("JSON field {} exceeds usize", path.join(".")))
}

fn string_array_at(value: &Value, path: &[&str]) -> Result<Vec<String>> {
    let mut current = value;
    for component in path {
        current = current
            .as_object()
            .and_then(|object| object.get(*component))
            .with_context(|| format!("missing JSON field {}", path.join(".")))?;
    }
    current
        .as_array()
        .with_context(|| format!("JSON field {} is not an array", path.join(".")))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .with_context(|| format!("JSON field {} contains a non-string", path.join(".")))
        })
        .collect()
}

fn decode_hex_array<const N: usize>(value: &str) -> Result<[u8; N]> {
    ensure!(value.len() == N * 2, "hex value has the wrong length");
    let mut decoded = [0_u8; N];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn hex_nibble(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => bail!("hex value is not lowercase hexadecimal"),
    }
}

fn next_path<'a>(fields: &mut impl Iterator<Item = &'a [u8]>) -> Result<String> {
    let path = fields.next().context("Git diff record is missing a path")?;
    ensure!(!path.is_empty(), "Git diff path is empty");
    ensure!(path.len() <= 4096, "Git diff path exceeds 4096 bytes");
    Ok(STANDARD.encode(path))
}

fn nonzero(value: &str) -> Option<String> {
    (!value.bytes().all(|byte| byte == b'0')).then(|| value.to_owned())
}

fn parse_similarity(status: &str) -> Result<u8> {
    let similarity = status
        .get(1..)
        .context("missing Git similarity")?
        .parse::<u8>()
        .context("invalid Git similarity")?;
    ensure!(similarity <= 100, "Git similarity exceeds 100%");
    Ok(similarity)
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_mode(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
}

fn bounded_reason(reason: &str) -> String {
    let reason = reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if reason.chars().count() <= 512 {
        reason
    } else {
        reason.chars().take(512).collect()
    }
}

fn git_text(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = git_output_bounded(repository, arguments, MAX_GIT_DIAGNOSTIC_BYTES)?;
    let value = String::from_utf8(output.stdout).context("Git output is not valid UTF-8")?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn git_output_bounded(
    repository: &Path,
    arguments: &[&str],
    stdout_limit: usize,
) -> Result<Output> {
    let mut child = isolated_git_command(repository)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run Git in {}", repository.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("failed to capture Git stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture Git stderr")?;
    let stdout_reader = std::thread::spawn(move || read_bounded(stdout, stdout_limit));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr, MAX_GIT_DIAGNOSTIC_BYTES));
    let (stdout, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("Git stdout reader panicked"))??;
    if stdout_exceeded {
        child
            .kill()
            .context("failed to stop Git after stdout exceeded its limit")?;
    }
    let status = child.wait().context("failed to wait for Git")?;
    let (stderr, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("Git stderr reader panicked"))??;
    ensure!(
        !stdout_exceeded,
        "Git output exceeds the {stdout_limit} byte limit"
    );
    ensure!(
        !stderr_exceeded,
        "Git diagnostics exceed the {MAX_GIT_DIAGNOSTIC_BYTES} byte limit"
    );
    ensure!(
        status.success(),
        "Git {} failed with {status}: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&stderr).trim()
    );
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut exceeded = false;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let keep = limit.saturating_sub(retained.len()).min(count);
        retained.extend_from_slice(&chunk[..keep]);
        if keep < count {
            exceeded = true;
            break;
        }
    }
    Ok((retained, exceeded))
}

fn isolated_git_command(repository: &Path) -> Command {
    let mut command = Command::new("git");
    for (name, _) in env::vars_os() {
        if unsafe_git_environment(&name) {
            command.env_remove(name);
        }
    }
    command
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_GRAFT_FILE", "")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg("--no-replace-objects")
        .arg("-c")
        .arg(format!("diff.orderFile={}", null_device()))
        .arg("-C")
        .arg(repository);
    command
}

fn unsafe_git_environment(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"GIT_"))
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_every_object_level() {
        let value = json!({"z": {"b": 1, "a": 2}, "a": [true, {"y": 3, "x": 4}]});
        assert_eq!(
            canonical_json_bytes(&value).unwrap(),
            br#"{"a":[true,{"x":4,"y":3}],"z":{"a":2,"b":1}}"#
        );
    }

    #[test]
    fn strict_json_rejects_duplicate_keys() {
        let error = parse_strict_json(br#"{"value":1,"value":2}"#, "fixture").unwrap_err();
        assert!(error.to_string().contains("failed to decode fixture"));
    }

    #[test]
    fn identity_hash_includes_raw_path_bytes() {
        let first = ExactGitChangeIdentity {
            status: ChangeStatus::Modified,
            similarity_percent: None,
            before_path_base64: Some(STANDARD.encode(b"src/a.rs")),
            after_path_base64: Some(STANDARD.encode(b"src/a.rs")),
            before_mode: Some("100644".to_owned()),
            after_mode: Some("100644".to_owned()),
            before_object_id: Some("1".repeat(40)),
            after_object_id: Some("2".repeat(40)),
        };
        let mut second = first.clone();
        second.after_path_base64 = Some(STANDARD.encode(b"src/b.rs"));
        assert_ne!(
            sha256_value(&serde_json::to_value(first).unwrap()).unwrap(),
            sha256_value(&serde_json::to_value(second).unwrap()).unwrap()
        );
    }

    #[test]
    fn base_drift_and_current_sources_share_one_payload_budget() {
        let mut payload_bytes = 0;
        payload_bytes = checked_source_budget(payload_bytes, 6, 10).unwrap();
        payload_bytes = checked_source_budget(payload_bytes, 4, 10).unwrap();
        assert_eq!(payload_bytes, 10);
        let error = checked_source_budget(payload_bytes, 1, 10).unwrap_err();
        assert!(error.to_string().contains("exceed the 10 byte limit"));
    }
}
