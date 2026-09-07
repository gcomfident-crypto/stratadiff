#![cfg(unix)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::{Command, Output},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stratadiff::review_cache::{
    REVIEW_CACHE_CROSS_ITEM_AGGREGATOR_SCHEMA, REVIEW_CACHE_PAYLOAD_SCHEMA,
    REVIEW_CACHE_RESULT_SCHEMA, REVIEW_CACHE_REVIEWER_MANIFEST_SCHEMA, REVIEW_CONTEXT_SCHEMA,
    canonical_json_bytes,
};

const GENERATED_AT: &str = "2026-09-06T12:00:00Z";
const KEY_ID: &str = "review-cache-test-key";
const TRUST_DOMAIN: &str = "review-cache-tests";

struct Fixture {
    directory: tempfile::TempDir,
    base: String,
    checkpoint: String,
    equivalent: String,
    residue: String,
    third_equivalent: String,
    updated_base: String,
    rebased_equivalent: String,
}

struct ReceiptFiles {
    receipt: std::path::PathBuf,
    input: std::path::PathBuf,
    payload: std::path::PathBuf,
    result: std::path::PathBuf,
    public_key: String,
    trust_policy_sha256: String,
}

struct ReceiptIssueCommand<'a> {
    repository: &'a Path,
    context: &'a Path,
    input: &'a Path,
    payload: &'a Path,
    result: &'a Path,
    expected_base: &'a str,
    expected_head: &'a str,
    prior: Option<&'a ReceiptFiles>,
    trust_policy_sha256: &'a str,
    signing_key: &'a Path,
    output: &'a Path,
}

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

fn commit(repository: &Path, message: &str) -> String {
    git(repository, &["add", "--all"]);
    git(repository, &["commit", "-q", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path();
    git(repository, &["init", "-q"]);
    git(repository, &["config", "user.name", "StrataDiff Test"]);
    git(
        repository,
        &["config", "user.email", "stratadiff@example.test"],
    );
    fs::write(repository.join("a.txt"), "base a\n").unwrap();
    fs::write(repository.join("b.txt"), "base b\n").unwrap();
    let base = commit(repository, "base");

    git(repository, &["checkout", "-q", "-b", "reviewed"]);
    fs::write(repository.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(repository.join("b.txt"), "reviewed b\n").unwrap();
    let checkpoint = commit(repository, "reviewed");

    git(repository, &["checkout", "-q", "-b", "equivalent", &base]);
    fs::write(repository.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(repository.join("b.txt"), "reviewed b\n").unwrap();
    let equivalent = commit(repository, "equivalent rewrite");

    git(repository, &["checkout", "-q", "-b", "residue", &base]);
    fs::write(repository.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(repository.join("b.txt"), "changed again\n").unwrap();
    let residue = commit(repository, "partial rewrite");

    git(
        repository,
        &["checkout", "-q", "-b", "third-equivalent", &base],
    );
    fs::write(repository.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(repository.join("b.txt"), "changed again\n").unwrap();
    let third_equivalent = commit(repository, "second equivalent rewrite");

    git(repository, &["checkout", "-q", "-b", "updated-base", &base]);
    fs::write(repository.join("upstream.txt"), "new upstream context\n").unwrap();
    let updated_base = commit(repository, "updated base");
    git(
        repository,
        &["checkout", "-q", "-b", "rebased-equivalent", &updated_base],
    );
    fs::write(repository.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(repository.join("b.txt"), "reviewed b\n").unwrap();
    let rebased_equivalent = commit(repository, "equivalent changes on updated base");

    Fixture {
        directory,
        base,
        checkpoint,
        equivalent,
        residue,
        third_equivalent,
        updated_base,
        rebased_equivalent,
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

fn decode_hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
        .collect()
}

fn snapshot_tree(repository: &Path, commit: &str) -> String {
    git(repository, &["rev-parse", &format!("{commit}^{{tree}}")])
}

fn cross_item_aggregator_binding() -> Value {
    let contract: Value = serde_json::from_str(include_str!(
        "../schema/review-cache-cross-item-aggregator-v1.json"
    ))
    .unwrap();
    let bytes = canonical_json_bytes(&contract).unwrap();
    json!({
        "schema_uri": REVIEW_CACHE_CROSS_ITEM_AGGREGATOR_SCHEMA,
        "canonical_sha256": sha256(&bytes),
        "byte_length": bytes.len(),
    })
}

fn context(repository: &Path, base: &str, checkpoint: &str, head: &str, open: bool) -> Value {
    context_with_scope(
        repository,
        base,
        checkpoint,
        head,
        open,
        "selected_payload_only",
    )
}

fn context_with_scope(
    repository: &Path,
    base: &str,
    checkpoint: &str,
    head: &str,
    open: bool,
    input_scope: &str,
) -> Value {
    let commits = [base, checkpoint, head]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let trees = commits
        .iter()
        .map(|commit| snapshot_tree(repository, commit))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let dependency_closure = if open {
        json!({
            "status": "open",
            "reason": "hosted_model_unpinned",
            "observed_inputs_sha256": "0".repeat(64),
        })
    } else {
        json!({
            "status": "closed",
            "manifest_sha256": "0".repeat(64),
        })
    };
    let model = if open {
        json!({
            "kind": "hosted_model_unpinned",
            "provider": "test-provider",
            "model_id": "rolling-model",
            "provider_revision": "rolling",
            "inference_parameters_sha256": "1".repeat(64),
            "automatically_reusable": false,
        })
    } else {
        json!({"kind": "none"})
    };
    let body = json!({
        "reuse_policy": "exact_git_change_identity_only_v1",
        "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
        "review_input_scope": input_scope,
        "dependency_closure": dependency_closure,
        "reviewer": {
            "implementation": {
                "name": "fixture-reviewer",
                "version": "1.0.0",
                "entrypoint": "review",
                "artifact_sha256": "2".repeat(64),
            },
            "model": model,
            "prompt": {
                "schema_uri": "urn:test:prompt:v1",
                "canonical_sha256": "3".repeat(64),
                "byte_length": 10,
            },
            "policy": {
                "schema_uri": "urn:test:policy:v1",
                "canonical_sha256": "4".repeat(64),
                "byte_length": 10,
            },
            "configuration": {
                "schema_uri": "urn:test:config:v1",
                "canonical_sha256": "5".repeat(64),
                "byte_length": 10,
            },
            "runtime": {
                "execution_kind": "sandbox",
                "runtime_name": "fixture",
                "runtime_version": "1",
                "operating_system": "linux",
                "architecture": "x86_64",
                "runtime_manifest_sha256": "6".repeat(64),
                "environment_sha256": "7".repeat(64),
            },
            "composition": {
                "kind": "itemwise_closed_v1",
                "reviewer_input_projection": {
                    "schema_uri": "urn:stratadiff:reviewer-visible-input:v1",
                    "canonical_sha256": "e".repeat(64),
                    "byte_length": 10,
                },
                "item_review_contract": {
                    "schema_uri": "urn:test:item-review-contract:v1",
                    "canonical_sha256": "c".repeat(64),
                    "byte_length": 10,
                },
                "cross_item_aggregator": cross_item_aggregator_binding(),
            },
            "tools": [],
        },
        "repository": {
            "provider": "github",
            "host": "github.com",
            "owner": "acme",
            "name": "widget",
            "repository_id": "R_test",
            "object_format": "sha1",
        },
        "repository_closure": {
            "kind": "declared_git_object_and_path_closure_v1",
            "complete": true,
            "root_commit_oids": commits,
            "root_tree_oids": trees,
            "object_count": 12,
            "path_count": 2,
            "object_manifest_sha256": "8".repeat(64),
            "path_manifest_sha256": "9".repeat(64),
            "canonical_closure_sha256": sha256(head.as_bytes()),
        },
        "pull_request": {
            "node_id": "PR_test",
            "number": 7,
            "base_ref": "main",
            "head_ref": "feature",
            "base_oid": base,
            "head_oid": head,
            "observed_at": GENERATED_AT,
            "canonical_metadata_sha256": "a".repeat(64),
        },
        "historical_dispositions": [],
    });
    let compatibility = json!({
        "reuse_policy": body["reuse_policy"],
        "change_identity_schema": body["change_identity_schema"],
        "review_input_scope": body["review_input_scope"],
        "dependency_closure": body["dependency_closure"],
        "reviewer": body["reviewer"],
    });
    json!({
        "schema": REVIEW_CONTEXT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "body_sha256": sha256(&canonical_json_bytes(&body).unwrap()),
        "compatibility_sha256": sha256(&canonical_json_bytes(&compatibility).unwrap()),
        "body": body,
    })
}

fn holistic_context(mut context: Value) -> Value {
    context["body"]["reviewer"]["composition"] = json!({
        "kind": "holistic",
        "reviewer_input_projection": {
            "schema_uri": "urn:stratadiff:reviewer-visible-input:v1",
            "canonical_sha256": "e".repeat(64),
            "byte_length": 10,
        },
    });
    refresh_context_digests(&mut context);
    context
}

fn refresh_context_digests(context: &mut Value) {
    let body = &context["body"];
    let compatibility = json!({
        "reuse_policy": body["reuse_policy"],
        "change_identity_schema": body["change_identity_schema"],
        "review_input_scope": body["review_input_scope"],
        "dependency_closure": body["dependency_closure"],
        "reviewer": body["reviewer"],
    });
    context["body_sha256"] = Value::String(sha256(&canonical_json_bytes(body).unwrap()));
    context["compatibility_sha256"] =
        Value::String(sha256(&canonical_json_bytes(&compatibility).unwrap()));
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, canonical_json_bytes(value).unwrap()).unwrap();
}

fn run_cache(
    fixture: &Fixture,
    context_path: &Path,
    output: &Path,
    payload: &Path,
    receipt: Option<&ReceiptFiles>,
    github_output: Option<&Path>,
) -> Output {
    run_cache_with_checkpoint(
        fixture,
        &fixture.checkpoint,
        context_path,
        output,
        payload,
        receipt,
        github_output,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_cache_with_checkpoint(
    fixture: &Fixture,
    checkpoint: &str,
    context_path: &Path,
    output: &Path,
    payload: &Path,
    receipt: Option<&ReceiptFiles>,
    github_output: Option<&Path>,
) -> Output {
    let reviewer_input = payload.with_extension("reviewer-input.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
    command
        .arg("review-cache")
        .arg(checkpoint)
        .arg("--repo")
        .arg(fixture.directory.path())
        .arg("--context")
        .arg(context_path)
        .arg("--generated-at")
        .arg(GENERATED_AT)
        .arg("--output")
        .arg(output)
        .arg("--payload-output")
        .arg(payload)
        .arg("--reviewer-input-output")
        .arg(reviewer_input);
    if let Some(receipt) = receipt {
        command
            .arg("--receipt")
            .arg(&receipt.receipt)
            .arg("--prior-input")
            .arg(&receipt.input)
            .arg("--prior-payload")
            .arg(&receipt.payload)
            .arg("--prior-result")
            .arg(&receipt.result)
            .arg("--trusted-key-id")
            .arg(KEY_ID)
            .arg("--trusted-public-key")
            .arg(&receipt.public_key)
            .arg("--trust-domain")
            .arg(TRUST_DOMAIN)
            .arg("--trust-policy-sha256")
            .arg(&receipt.trust_policy_sha256);
    }
    if let Some(github_output) = github_output {
        command.arg("--github-output").arg(github_output);
    }
    command.output().unwrap()
}

fn run_receipt_issue(request: ReceiptIssueCommand<'_>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
    command
        .arg("review-cache-receipt")
        .arg("--repo")
        .arg(request.repository)
        .arg("--context")
        .arg(request.context)
        .arg("--review-input")
        .arg(request.input)
        .arg("--selected-payload")
        .arg(request.payload)
        .arg("--result")
        .arg(request.result)
        .arg("--expected-base")
        .arg(request.expected_base)
        .arg("--expected-head")
        .arg(request.expected_head)
        .arg("--receipt-id")
        .arg("receipt-test-1")
        .arg("--issued-at")
        .arg(GENERATED_AT)
        .arg("--issuer-id")
        .arg("fixture-reviewer")
        .arg("--trust-domain")
        .arg(TRUST_DOMAIN)
        .arg("--trust-policy-sha256")
        .arg(request.trust_policy_sha256)
        .arg("--key-id")
        .arg(KEY_ID)
        .arg("--signing-key-file")
        .arg(request.signing_key)
        .arg("--output")
        .arg(request.output);
    if let Some(prior) = request.prior {
        command
            .arg("--prior-receipt")
            .arg(&prior.receipt)
            .arg("--prior-input")
            .arg(&prior.input)
            .arg("--prior-payload")
            .arg(&prior.payload)
            .arg("--prior-result")
            .arg(&prior.result)
            .arg("--prior-key-id")
            .arg(KEY_ID)
            .arg("--prior-public-key")
            .arg(&prior.public_key)
            .arg("--prior-trust-domain")
            .arg(TRUST_DOMAIN)
            .arg("--prior-trust-policy-sha256")
            .arg(&prior.trust_policy_sha256);
    }
    command.output().unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_receipt(
    directory: &Path,
    context: &Value,
    input_path: &Path,
    payload_path: &Path,
) -> ReceiptFiles {
    create_receipt_with_outcome(directory, context, input_path, payload_path, "passed", None)
}

fn create_receipt_with_outcome(
    directory: &Path,
    context: &Value,
    input_path: &Path,
    payload_path: &Path,
    overall_outcome: &str,
    failed_path: Option<&[u8]>,
) -> ReceiptFiles {
    create_receipt_with_prior(
        directory,
        context,
        input_path,
        payload_path,
        overall_outcome,
        failed_path,
        None,
        None,
        "receipt",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_receipt_with_prior(
    directory: &Path,
    context: &Value,
    input_path: &Path,
    payload_path: &Path,
    overall_outcome: &str,
    failed_path: Option<&[u8]>,
    failed_kind: Option<&str>,
    prior: Option<&ReceiptFiles>,
    prefix: &str,
) -> ReceiptFiles {
    let input_bytes = fs::read(input_path).unwrap();
    let input: Value = serde_json::from_slice(&input_bytes).unwrap();
    let payload_bytes = fs::read(payload_path).unwrap();
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();
    let identities = payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["obligation_sha256"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let obligation_results = payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let matches_failed_path = failed_path.is_some()
                && item["identity"]["after_path_base64"]
                    .as_str()
                    .map(|path| STANDARD.decode(path).unwrap())
                    .as_deref()
                    == failed_path;
            let matches_failed_kind = failed_kind == item["kind"].as_str();
            json!({
                "obligation_sha256": item["obligation_sha256"],
                "outcome": if matches_failed_path || matches_failed_kind { overall_outcome } else { "passed" },
            })
        })
        .collect::<Vec<_>>();
    let result_value = json!({
        "schema": REVIEW_CACHE_RESULT_SCHEMA,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "producer": {
            "adapter_id": "fixture-reviewer",
            "execution_id": "fixture-execution-1",
        },
        "review_input_sha256": sha256(&input_bytes),
        "selected_payload_sha256": sha256(&payload_bytes),
        "reviewer_input_projection_sha256": input["resolution"]["selected_payload"]["reviewer_input_projection_sha256"],
        "outcome": overall_outcome,
        "coverage": {
            "complete": true,
            "covered_obligation_sha256": identities.clone(),
            "omitted_obligation_sha256": [],
            "unresolved_obligation_sha256": [],
        },
        "obligation_results": obligation_results,
        "reviewer_output": {
            "schema": "urn:test:reviewer-output:v1",
            "passed": true,
        },
    });
    let result_bytes = canonical_json_bytes(&result_value).unwrap();
    let result_path = directory.join(format!("{prefix}-result.json"));
    fs::write(&result_path, &result_bytes).unwrap();
    let trust_policy_sha256 = "b".repeat(64);
    let key = SigningKey::from_bytes(&[17; 32]);
    let key_path = directory.join(format!("{prefix}-signing-key.hex"));
    fs::write(&key_path, format!("{}\n", encode_hex(key.as_bytes()))).unwrap();
    let context_path = directory.join(format!("{prefix}-context.json"));
    write_json(&context_path, context);
    let receipt_path = directory.join(format!("{prefix}.json"));
    let output = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &context_path,
        input: input_path,
        payload: payload_path,
        result: &result_path,
        expected_base: context["body"]["pull_request"]["base_oid"]
            .as_str()
            .unwrap(),
        expected_head: context["body"]["pull_request"]["head_oid"]
            .as_str()
            .unwrap(),
        prior,
        trust_policy_sha256: &trust_policy_sha256,
        signing_key: &key_path,
        output: &receipt_path,
    });
    assert_success(&output);
    ReceiptFiles {
        receipt: receipt_path,
        input: input_path.to_owned(),
        payload: payload_path.to_owned(),
        result: result_path,
        public_key: encode_hex(key.verifying_key().as_bytes()),
        trust_policy_sha256,
    }
}

fn prepare_prior_review(fixture: &Fixture) -> (Value, ReceiptFiles) {
    prepare_prior_review_with_scope(fixture, "selected_payload_only", "prior")
}

fn prepare_prior_review_with_scope(
    fixture: &Fixture,
    input_scope: &str,
    prefix: &str,
) -> (Value, ReceiptFiles) {
    let directory = fixture.directory.path();
    let prior_context = context_with_scope(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.checkpoint,
        false,
        input_scope,
    );
    let context_path = directory.join(format!("{prefix}-context.json"));
    let input_path = directory.join(format!("{prefix}-input.json"));
    let payload_path = directory.join(format!("{prefix}-payload.json"));
    write_json(&context_path, &prior_context);
    let output = run_cache(
        fixture,
        &context_path,
        &input_path,
        &payload_path,
        None,
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "full");
    let receipt = create_receipt(directory, &prior_context, &input_path, &payload_path);
    (prior_context, receipt)
}

#[test]
fn first_run_is_full_and_exports_github_action_outputs() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let value = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    let context_path = directory.join("context.json");
    let input_path = directory.join("input.json");
    let payload_path = directory.join("payload.json");
    let github_output = directory.join("github-output.txt");
    write_json(&context_path, &value);
    let output = run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        None,
        Some(&github_output),
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&fs::read(&payload_path).unwrap()).unwrap();
    let reviewer_input_bytes =
        fs::read(payload_path.with_extension("reviewer-input.json")).unwrap();
    let reviewer_input: Value = serde_json::from_slice(&reviewer_input_bytes).unwrap();
    assert_eq!(decision["resolution"]["decision"], "full");
    assert_eq!(decision["resolution"]["accounting"]["carried"], json!([]));
    assert_eq!(payload["schema"], REVIEW_CACHE_PAYLOAD_SCHEMA);
    assert_eq!(payload["mode"], "full");
    assert_eq!(payload["items"].as_array().unwrap().len(), 2);
    assert!(reviewer_input.get("base_commit").is_none());
    assert!(reviewer_input.get("head_commit").is_none());
    assert_eq!(reviewer_input["items"], payload["items"]);
    assert_eq!(
        decision["resolution"]["selected_payload"]["reviewer_input_projection_sha256"],
        sha256(&reviewer_input_bytes)
    );
    let outputs = fs::read_to_string(github_output).unwrap();
    assert!(outputs.contains("decision=full\n"));
    assert!(outputs.contains("should_run=true\n"));
}

#[test]
fn context_builder_materializes_git_closure_and_feeds_preflight() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let seed = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    let manifest = json!({
        "schema": REVIEW_CACHE_REVIEWER_MANIFEST_SCHEMA,
        "contract_version": "1.0.0",
        "dependency_closure": seed["body"]["dependency_closure"],
        "reviewer": seed["body"]["reviewer"],
        "historical_dispositions": seed["body"]["historical_dispositions"],
    });
    let manifest_path = directory.join("reviewer-manifest.json");
    let context_path = directory.join("built-context.json");
    write_json(&manifest_path, &manifest);
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review-cache-context")
        .arg("--repo")
        .arg(directory)
        .arg("--base")
        .arg(&fixture.base)
        .arg("--checkpoint")
        .arg(&fixture.checkpoint)
        .arg("--head")
        .arg(&fixture.equivalent)
        .arg("--reviewer-manifest")
        .arg(&manifest_path)
        .arg("--provider-host")
        .arg("github.com")
        .arg("--owner")
        .arg("acme")
        .arg("--name")
        .arg("widget")
        .arg("--repository-id")
        .arg("R_test")
        .arg("--pull-request-node-id")
        .arg("PR_test")
        .arg("--pull-request-number")
        .arg("7")
        .arg("--base-ref")
        .arg("main")
        .arg("--head-ref")
        .arg("feature")
        .arg("--observed-at")
        .arg(GENERATED_AT)
        .arg("--canonical-metadata-sha256")
        .arg("a".repeat(64))
        .arg("--review-input-scope")
        .arg("selected_payload_only")
        .arg("--output")
        .arg(&context_path)
        .output()
        .unwrap();
    assert_success(&output);
    let built: Value = serde_json::from_slice(&fs::read(&context_path).unwrap()).unwrap();
    assert_eq!(
        built["body"]["pull_request"]["head_oid"],
        fixture.equivalent
    );
    assert!(
        built["body"]["repository_closure"]["object_count"]
            .as_u64()
            .unwrap()
            > 0
    );

    let input_path = directory.join("built-context-input.json");
    let payload_path = directory.join("built-context-payload.json");
    assert_success(&run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        None,
        None,
    ));
}

#[test]
fn itemwise_context_must_bind_the_builtin_cross_item_aggregator() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let mut value = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    value["body"]["reviewer"]["composition"]["cross_item_aggregator"]["canonical_sha256"] =
        Value::String("0".repeat(64));
    refresh_context_digests(&mut value);
    let context_path = directory.join("bad-aggregator-context.json");
    let input_path = directory.join("bad-aggregator-input.json");
    let payload_path = directory.join("bad-aggregator-payload.json");
    write_json(&context_path, &value);
    assert_success(&run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        None,
        None,
    ));
    let decision: Value = serde_json::from_slice(&fs::read(input_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "blocked");
    assert_eq!(
        decision["resolution"]["reason"],
        "invalid_receipt_or_context"
    );
    assert!(
        decision["resolution"]["accounting"]["blocking_reasons"][0]
            .as_str()
            .unwrap()
            .contains("built-in conservative outcome reduction")
    );
}

#[test]
fn signed_receipt_allows_only_exact_identity_skip_and_residue() {
    let fixture = fixture();
    let (_, receipt) = prepare_prior_review(&fixture);
    let directory = fixture.directory.path();

    let equivalent_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    let equivalent_context_path = directory.join("equivalent-context.json");
    let skip_input = directory.join("skip-input.json");
    let skip_payload = directory.join("skip-payload.json");
    write_json(&equivalent_context_path, &equivalent_context);
    let output = run_cache(
        &fixture,
        &equivalent_context_path,
        &skip_input,
        &skip_payload,
        Some(&receipt),
        None,
    );
    assert_success(&output);
    let skip: Value = serde_json::from_slice(&fs::read(skip_input).unwrap()).unwrap();
    assert_eq!(skip["resolution"]["decision"], "skip");
    assert_eq!(
        skip["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        skip["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .iter()
            .all(|carry| carry["basis"] == "exact_git_change_identity")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(skip_payload).unwrap()).unwrap()["items"],
        json!([])
    );

    let residue_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.residue,
        false,
    );
    let residue_context_path = directory.join("residue-context.json");
    let residue_input = directory.join("residue-input.json");
    let residue_payload = directory.join("residue-payload.json");
    write_json(&residue_context_path, &residue_context);
    let output = run_cache(
        &fixture,
        &residue_context_path,
        &residue_input,
        &residue_payload,
        Some(&receipt),
        None,
    );
    assert_success(&output);
    let residue: Value = serde_json::from_slice(&fs::read(residue_input).unwrap()).unwrap();
    assert_eq!(residue["resolution"]["decision"], "residue");
    assert_eq!(
        residue["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        residue["resolution"]["accounting"]["selected_current_identity_sha256"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        residue["resolution"]["accounting"]["unresolved_retired_obligation_sha256"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn residue_receipt_preserves_complete_identity_lineage_across_three_hops() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let prior_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.checkpoint,
        false,
    );
    let prior_context_path = directory.join("three-hop-prior-context.json");
    let prior_input_path = directory.join("three-hop-prior-input.json");
    let prior_payload_path = directory.join("three-hop-prior-payload.json");
    write_json(&prior_context_path, &prior_context);
    assert_success(&run_cache(
        &fixture,
        &prior_context_path,
        &prior_input_path,
        &prior_payload_path,
        None,
        None,
    ));
    let first_receipt = create_receipt_with_outcome(
        directory,
        &prior_context,
        &prior_input_path,
        &prior_payload_path,
        "advisory",
        Some(b"a.txt"),
    );

    let residue_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.residue,
        false,
    );
    let residue_context_path = directory.join("three-hop-residue-context.json");
    let residue_input_path = directory.join("three-hop-residue-input.json");
    let residue_payload_path = directory.join("three-hop-residue-payload.json");
    write_json(&residue_context_path, &residue_context);
    assert_success(&run_cache(
        &fixture,
        &residue_context_path,
        &residue_input_path,
        &residue_payload_path,
        Some(&first_receipt),
        None,
    ));
    let second_receipt = create_receipt_with_prior(
        directory,
        &residue_context,
        &residue_input_path,
        &residue_payload_path,
        "passed",
        None,
        None,
        Some(&first_receipt),
        "three-hop-second-receipt",
    );
    let second_receipt_value: Value =
        serde_json::from_slice(&fs::read(&second_receipt.receipt).unwrap()).unwrap();
    assert_eq!(
        second_receipt_value["body"]["coverage"]["covered_identity_sha256"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let lineage_kinds = second_receipt_value["body"]["coverage"]["current_identity_results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["lineage"]["kind"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        lineage_kinds,
        BTreeSet::from(["prior_receipt_exact_carry", "selected_execution"])
    );
    assert_eq!(
        second_receipt_value["body"]["prior_result"]["selected_outcome"],
        "passed"
    );
    assert_eq!(
        second_receipt_value["body"]["prior_result"]["effective_outcome"],
        "advisory"
    );
    assert_eq!(
        second_receipt_value["body"]["prior_result"]["retained_non_current_blocking_outcome"],
        "passed"
    );
    let missing_prior_output = directory.join("three-hop-missing-prior.json");
    let missing_prior = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &directory.join("three-hop-second-receipt-context.json"),
        input: &second_receipt.input,
        payload: &second_receipt.payload,
        result: &second_receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.residue,
        prior: None,
        trust_policy_sha256: &second_receipt.trust_policy_sha256,
        signing_key: &directory.join("three-hop-second-receipt-signing-key.hex"),
        output: &missing_prior_output,
    });
    assert!(!missing_prior.status.success());
    assert!(
        String::from_utf8_lossy(&missing_prior.stderr)
            .contains("a residue receipt requires the complete trusted prior receipt bundle")
    );
    let original_first_result = fs::read(&first_receipt.result).unwrap();
    let mut tampered_first_result: Value = serde_json::from_slice(&original_first_result).unwrap();
    tampered_first_result["reviewer_output"]["tampered"] = Value::Bool(true);
    write_json(&first_receipt.result, &tampered_first_result);
    let tampered_prior_output = directory.join("three-hop-tampered-prior.json");
    let tampered_prior = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &directory.join("three-hop-second-receipt-context.json"),
        input: &second_receipt.input,
        payload: &second_receipt.payload,
        result: &second_receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.residue,
        prior: Some(&first_receipt),
        trust_policy_sha256: &second_receipt.trust_policy_sha256,
        signing_key: &directory.join("three-hop-second-receipt-signing-key.hex"),
        output: &tampered_prior_output,
    });
    assert!(!tampered_prior.status.success());
    assert!(
        String::from_utf8_lossy(&tampered_prior.stderr)
            .contains("prior receipt is not eligible for residue issuance")
    );
    fs::write(&first_receipt.result, original_first_result).unwrap();

    let third_context = context(
        directory,
        &fixture.base,
        &fixture.residue,
        &fixture.third_equivalent,
        false,
    );
    let third_context_path = directory.join("three-hop-final-context.json");
    let third_input_path = directory.join("three-hop-final-input.json");
    let third_payload_path = directory.join("three-hop-final-payload.json");
    write_json(&third_context_path, &third_context);
    assert_success(&run_cache_with_checkpoint(
        &fixture,
        &fixture.residue,
        &third_context_path,
        &third_input_path,
        &third_payload_path,
        Some(&second_receipt),
        None,
    ));
    let third_input: Value = serde_json::from_slice(&fs::read(&third_input_path).unwrap()).unwrap();
    assert_eq!(third_input["resolution"]["decision"], "skip");
    assert_eq!(
        third_input["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        third_input["resolution"]["prior_receipt"]["prior_outcome"],
        "advisory"
    );
    let second_outcomes = second_receipt_value["body"]["coverage"]["current_identity_results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["identity_sha256"].as_str().unwrap(),
                item["outcome"].as_str().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let third_outcomes = third_input["resolution"]["accounting"]["carried"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["current_identity_sha256"].as_str().unwrap(),
                item["prior_outcome"].as_str().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(third_outcomes, second_outcomes);
}

#[test]
fn raw_digest_signature_and_open_context_never_carry() {
    let fixture = fixture();
    let (_, mut receipt) = prepare_prior_review(&fixture);
    let directory = fixture.directory.path();
    let current_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    let context_path = directory.join("current-context.json");
    write_json(&context_path, &current_context);

    let mut receipt_value: Value =
        serde_json::from_slice(&fs::read(&receipt.receipt).unwrap()).unwrap();
    assert_eq!(
        receipt_value["attestation"]["signature_domain"],
        "stratadiff.review-receipt"
    );
    assert_eq!(
        receipt_value["attestation"]["signature_preimage_version"],
        "1"
    );
    let body_sha256 = receipt_value["attestation"]["body_sha256"]
        .as_str()
        .unwrap();
    let raw_digest_signature = SigningKey::from_bytes(&[17; 32]).sign(&decode_hex(body_sha256));
    receipt_value["attestation"]["signature"] =
        Value::String(encode_hex(&raw_digest_signature.to_bytes()));
    let bad_receipt = directory.join("bad-receipt.json");
    write_json(&bad_receipt, &receipt_value);
    receipt.receipt = bad_receipt;
    let bad_input = directory.join("bad-input.json");
    let bad_payload = directory.join("bad-payload.json");
    let output = run_cache(
        &fixture,
        &context_path,
        &bad_input,
        &bad_payload,
        Some(&receipt),
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(bad_input).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "full");
    assert_eq!(
        decision["resolution"]["prior_receipt"]["reason"],
        "invalid_signature"
    );
    assert_eq!(decision["resolution"]["accounting"]["carried"], json!([]));

    let (_, valid_receipt) = prepare_prior_review(&fixture);
    let open_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        true,
    );
    let open_context_path = directory.join("open-context.json");
    let open_input = directory.join("open-input.json");
    let open_payload = directory.join("open-payload.json");
    write_json(&open_context_path, &open_context);
    let output = run_cache(
        &fixture,
        &open_context_path,
        &open_input,
        &open_payload,
        Some(&valid_receipt),
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(open_input).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "full");
    assert_eq!(decision["resolution"]["reason"], "context_open");
    assert_eq!(decision["resolution"]["accounting"]["carried"], json!([]));
}

#[test]
fn unavailable_current_blob_emits_blocked_without_selected_input() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let value = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.residue,
        false,
    );
    let context_path = directory.join("blocked-context.json");
    let input_path = directory.join("blocked-input.json");
    let payload_path = directory.join("blocked-payload.json");
    write_json(&context_path, &value);
    let blob = git(
        directory,
        &["rev-parse", &format!("{}:b.txt", fixture.residue)],
    );
    let object_path = directory
        .join(".git/objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    fs::remove_file(object_path).unwrap();

    let output = run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        None,
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&fs::read(&payload_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "blocked");
    assert_eq!(decision["resolution"]["accounting"]["carried"], json!([]));
    assert_eq!(
        decision["resolution"]["selected_payload"]["selected_identity_sha256"],
        json!([])
    );
    assert_eq!(payload["mode"], "blocked");
    assert_eq!(payload["items"], json!([]));
}

#[test]
fn diverged_base_preserves_exact_carries_but_keeps_base_drift_in_the_residue() {
    let fixture = fixture();
    let (_, receipt) = prepare_prior_review(&fixture);
    let directory = fixture.directory.path();
    let current_context = context(
        directory,
        &fixture.updated_base,
        &fixture.checkpoint,
        &fixture.rebased_equivalent,
        false,
    );
    let context_path = directory.join("diverged-context.json");
    let input_path = directory.join("diverged-input.json");
    let payload_path = directory.join("diverged-payload.json");
    write_json(&context_path, &current_context);
    let output = run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        Some(&receipt),
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&fs::read(&payload_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "residue");
    assert_eq!(
        decision["resolution"]["context_comparison"]["status"],
        "exact"
    );
    assert_ne!(
        decision["resolution"]["context_comparison"]["current_body_sha256"],
        decision["resolution"]["context_comparison"]["receipt_body_sha256"]
    );
    assert_eq!(
        decision["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        decision["resolution"]["accounting"]["selected_current_identity_sha256"],
        json!([])
    );
    assert_eq!(
        decision["resolution"]["accounting"]["base_drift_obligation_sha256"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(payload["items"][0]["kind"], "base_drift");
    let base_changes = payload["items"][0]["evidence"]["changes"]
        .as_array()
        .unwrap();
    assert_eq!(base_changes.len(), 1);
    assert_eq!(base_changes[0]["identity"]["status"], "added");
    assert_eq!(base_changes[0]["before_source"]["kind"], "absent");
    assert_eq!(base_changes[0]["after_source"]["kind"], "git_blob");
    assert_eq!(
        STANDARD
            .decode(
                base_changes[0]["after_source"]["content_base64"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        b"new upstream context\n"
    );
    create_receipt_with_prior(
        directory,
        &current_context,
        &input_path,
        &payload_path,
        "passed",
        None,
        None,
        Some(&receipt),
        "base-drift-receipt",
    );
}

#[test]
fn non_current_blocker_is_re_reviewed_before_a_later_pass_can_clear_it() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    git(
        directory,
        &[
            "checkout",
            "-q",
            "-b",
            "blocking-follow-up",
            &fixture.rebased_equivalent,
        ],
    );
    fs::write(directory.join("c.txt"), "new follow-up\n").unwrap();
    let follow_up = commit(directory, "follow up");
    git(
        directory,
        &[
            "checkout",
            "-q",
            "-b",
            "blocking-final-equivalent",
            &fixture.updated_base,
        ],
    );
    fs::write(directory.join("a.txt"), "reviewed a\n").unwrap();
    fs::write(directory.join("b.txt"), "reviewed b\n").unwrap();
    fs::write(directory.join("c.txt"), "new follow-up\n").unwrap();
    let final_equivalent = commit(directory, "equivalent cleared result");
    let (_, first_receipt) = prepare_prior_review(&fixture);
    let drift_context = context(
        directory,
        &fixture.updated_base,
        &fixture.checkpoint,
        &fixture.rebased_equivalent,
        false,
    );
    let drift_context_path = directory.join("blocking-drift-context.json");
    let drift_input_path = directory.join("blocking-drift-input.json");
    let drift_payload_path = directory.join("blocking-drift-payload.json");
    write_json(&drift_context_path, &drift_context);
    assert_success(&run_cache(
        &fixture,
        &drift_context_path,
        &drift_input_path,
        &drift_payload_path,
        Some(&first_receipt),
        None,
    ));
    let blocking_receipt = create_receipt_with_prior(
        directory,
        &drift_context,
        &drift_input_path,
        &drift_payload_path,
        "changes_requested",
        None,
        Some("base_drift"),
        Some(&first_receipt),
        "blocking-drift-receipt",
    );
    let blocking_value: Value =
        serde_json::from_slice(&fs::read(&blocking_receipt.receipt).unwrap()).unwrap();
    assert_eq!(
        blocking_value["body"]["prior_result"]["retained_non_current_blocking_outcome"],
        "changes_requested"
    );

    let follow_context = context(
        directory,
        &fixture.updated_base,
        &fixture.rebased_equivalent,
        &follow_up,
        false,
    );
    let follow_context_path = directory.join("blocking-follow-context.json");
    let follow_input_path = directory.join("blocking-follow-input.json");
    let follow_payload_path = directory.join("blocking-follow-payload.json");
    write_json(&follow_context_path, &follow_context);
    assert_success(&run_cache_with_checkpoint(
        &fixture,
        &fixture.rebased_equivalent,
        &follow_context_path,
        &follow_input_path,
        &follow_payload_path,
        Some(&blocking_receipt),
        None,
    ));
    let follow_input: Value =
        serde_json::from_slice(&fs::read(&follow_input_path).unwrap()).unwrap();
    let follow_payload: Value =
        serde_json::from_slice(&fs::read(&follow_payload_path).unwrap()).unwrap();
    assert_eq!(follow_input["resolution"]["decision"], "residue");
    assert_eq!(
        follow_input["resolution"]["prior_receipt"]["retained_non_current_blocking_outcome"],
        "changes_requested"
    );
    let retained = follow_payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "retained_non_current_obligation")
        .unwrap();
    assert_eq!(retained["original_item"]["kind"], "base_drift");

    let cleared_receipt = create_receipt_with_prior(
        directory,
        &follow_context,
        &follow_input_path,
        &follow_payload_path,
        "passed",
        None,
        None,
        Some(&blocking_receipt),
        "cleared-drift-receipt",
    );
    let cleared_value: Value =
        serde_json::from_slice(&fs::read(&cleared_receipt.receipt).unwrap()).unwrap();
    assert_eq!(
        cleared_value["body"]["prior_result"]["retained_non_current_blocking_outcome"],
        "passed"
    );
    assert_eq!(
        cleared_value["body"]["prior_result"]["effective_outcome"],
        "passed"
    );

    let final_context = context(
        directory,
        &fixture.updated_base,
        &follow_up,
        &final_equivalent,
        false,
    );
    let final_context_path = directory.join("blocking-final-context.json");
    let final_input_path = directory.join("blocking-final-input.json");
    let final_payload_path = directory.join("blocking-final-payload.json");
    write_json(&final_context_path, &final_context);
    assert_success(&run_cache_with_checkpoint(
        &fixture,
        &follow_up,
        &final_context_path,
        &final_input_path,
        &final_payload_path,
        Some(&cleared_receipt),
        None,
    ));
    let final_input: Value = serde_json::from_slice(&fs::read(&final_input_path).unwrap()).unwrap();
    assert_eq!(
        final_input["resolution"]["decision"],
        "skip",
        "{}",
        serde_json::to_string_pretty(&final_input).unwrap()
    );
}

#[test]
fn whole_repository_scope_forces_full_when_the_root_changes() {
    let fixture = fixture();
    let (_, receipt) =
        prepare_prior_review_with_scope(&fixture, "declared_repository_closure", "whole-prior");
    let directory = fixture.directory.path();
    let current_context = context_with_scope(
        directory,
        &fixture.updated_base,
        &fixture.checkpoint,
        &fixture.rebased_equivalent,
        false,
        "declared_repository_closure",
    );
    let context_path = directory.join("whole-current-context.json");
    let input_path = directory.join("whole-current-input.json");
    let payload_path = directory.join("whole-current-payload.json");
    write_json(&context_path, &current_context);
    let output = run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        Some(&receipt),
        None,
    );
    assert_success(&output);
    let decision: Value = serde_json::from_slice(&fs::read(input_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "full");
    assert_eq!(decision["resolution"]["reason"], "context_mismatch");
    assert_eq!(decision["resolution"]["accounting"]["carried"], json!([]));
    assert_eq!(
        decision["resolution"]["accounting"]["base_drift_obligation_sha256"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn holistic_review_never_partially_carries_and_preserves_a_blocking_skip() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let prior_context = holistic_context(context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.checkpoint,
        false,
    ));
    let prior_context_path = directory.join("holistic-prior-context.json");
    let prior_input_path = directory.join("holistic-prior-input.json");
    let prior_payload_path = directory.join("holistic-prior-payload.json");
    write_json(&prior_context_path, &prior_context);
    assert_success(&run_cache(
        &fixture,
        &prior_context_path,
        &prior_input_path,
        &prior_payload_path,
        None,
        None,
    ));
    let receipt = create_receipt_with_outcome(
        directory,
        &prior_context,
        &prior_input_path,
        &prior_payload_path,
        "failed",
        Some(b"a.txt"),
    );

    let equivalent_context = holistic_context(context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    ));
    let equivalent_context_path = directory.join("holistic-equivalent-context.json");
    let skip_input = directory.join("holistic-skip-input.json");
    let skip_payload = directory.join("holistic-skip-payload.json");
    let github_output = directory.join("holistic-github-output.txt");
    write_json(&equivalent_context_path, &equivalent_context);
    let output = run_cache(
        &fixture,
        &equivalent_context_path,
        &skip_input,
        &skip_payload,
        Some(&receipt),
        Some(&github_output),
    );
    assert!(!output.status.success());
    let skip: Value = serde_json::from_slice(&fs::read(skip_input).unwrap()).unwrap();
    assert_eq!(skip["resolution"]["decision"], "skip");
    assert_eq!(
        skip["resolution"]["prior_receipt"]["prior_outcome"],
        "failed"
    );
    let outputs = fs::read_to_string(github_output).unwrap();
    assert!(outputs.contains("cached_outcome=failed\n"));
    assert!(outputs.contains("cached_blocking=true\n"));

    let residue_context = holistic_context(context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.residue,
        false,
    ));
    let residue_context_path = directory.join("holistic-residue-context.json");
    let residue_input = directory.join("holistic-residue-input.json");
    let residue_payload = directory.join("holistic-residue-payload.json");
    write_json(&residue_context_path, &residue_context);
    assert_success(&run_cache(
        &fixture,
        &residue_context_path,
        &residue_input,
        &residue_payload,
        Some(&receipt),
        None,
    ));
    let full: Value = serde_json::from_slice(&fs::read(residue_input).unwrap()).unwrap();
    assert_eq!(full["resolution"]["decision"], "full");
    assert_eq!(full["resolution"]["accounting"]["carried"], json!([]));
}

#[test]
fn failed_itemwise_obligation_is_selected_again_instead_of_carried() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let prior_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.checkpoint,
        false,
    );
    let prior_context_path = directory.join("failed-prior-context.json");
    let prior_input_path = directory.join("failed-prior-input.json");
    let prior_payload_path = directory.join("failed-prior-payload.json");
    write_json(&prior_context_path, &prior_context);
    assert_success(&run_cache(
        &fixture,
        &prior_context_path,
        &prior_input_path,
        &prior_payload_path,
        None,
        None,
    ));
    let receipt = create_receipt_with_outcome(
        directory,
        &prior_context,
        &prior_input_path,
        &prior_payload_path,
        "failed",
        Some(b"b.txt"),
    );

    let current_context = context(
        directory,
        &fixture.base,
        &fixture.checkpoint,
        &fixture.equivalent,
        false,
    );
    let context_path = directory.join("failed-current-context.json");
    let input_path = directory.join("failed-current-input.json");
    let payload_path = directory.join("failed-current-payload.json");
    write_json(&context_path, &current_context);
    assert_success(&run_cache(
        &fixture,
        &context_path,
        &input_path,
        &payload_path,
        Some(&receipt),
        None,
    ));
    let decision: Value = serde_json::from_slice(&fs::read(input_path).unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&fs::read(payload_path).unwrap()).unwrap();
    assert_eq!(decision["resolution"]["decision"], "residue");
    assert_eq!(
        decision["resolution"]["accounting"]["carried"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        decision["resolution"]["accounting"]["carried"][0]["prior_outcome"],
        "passed"
    );
    assert_eq!(payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        STANDARD
            .decode(
                payload["items"][0]["identity"]["after_path_base64"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        b"b.txt"
    );
}

#[test]
fn receipt_issuer_rejects_incomplete_coverage_and_a_stale_live_head() {
    let fixture = fixture();
    let (_, receipt) = prepare_prior_review(&fixture);
    let directory = fixture.directory.path();
    let context_path = directory.join("receipt-context.json");
    let key_path = directory.join("receipt-signing-key.hex");
    let rejected_receipt = directory.join("rejected-receipt.json");
    let valid_result = fs::read(&receipt.result).unwrap();
    let mut incomplete: Value = serde_json::from_slice(&valid_result).unwrap();
    incomplete["coverage"]["covered_obligation_sha256"]
        .as_array_mut()
        .unwrap()
        .pop();
    write_json(&receipt.result, &incomplete);
    let output = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &context_path,
        input: &receipt.input,
        payload: &receipt.payload,
        result: &receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.checkpoint,
        prior: None,
        trust_policy_sha256: &receipt.trust_policy_sha256,
        signing_key: &key_path,
        output: &rejected_receipt,
    });
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("review result coverage does not exactly match the selected payload")
    );

    let mut inconsistent: Value = serde_json::from_slice(&valid_result).unwrap();
    inconsistent["obligation_results"][0]["outcome"] =
        Value::String("changes_requested".to_owned());
    write_json(&receipt.result, &inconsistent);
    let output = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &context_path,
        input: &receipt.input,
        payload: &receipt.payload,
        result: &receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.checkpoint,
        prior: None,
        trust_policy_sha256: &receipt.trust_policy_sha256,
        signing_key: &key_path,
        output: &rejected_receipt,
    });
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("review result outcome does not equal the built-in conservative reduction")
    );

    fs::write(&receipt.result, valid_result).unwrap();
    let output = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &context_path,
        input: &receipt.input,
        payload: &receipt.payload,
        result: &receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.base,
        prior: None,
        trust_policy_sha256: &receipt.trust_policy_sha256,
        signing_key: &key_path,
        output: &rejected_receipt,
    });
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "adapter-observed live base tip or head differs from the routed review snapshots"
    ));
}

#[test]
fn receipt_binds_live_base_tip_separately_from_the_reviewed_merge_base() {
    let fixture = fixture();
    let directory = fixture.directory.path();
    let current_context = context(
        directory,
        &fixture.updated_base,
        &fixture.equivalent,
        &fixture.equivalent,
        false,
    );
    let context_path = directory.join("live-base-context.json");
    let input_path = directory.join("live-base-input.json");
    let payload_path = directory.join("live-base-payload.json");
    write_json(&context_path, &current_context);
    assert_success(&run_cache_with_checkpoint(
        &fixture,
        &fixture.equivalent,
        &context_path,
        &input_path,
        &payload_path,
        None,
        None,
    ));
    let receipt = create_receipt(directory, &current_context, &input_path, &payload_path);
    let receipt_value: Value =
        serde_json::from_slice(&fs::read(&receipt.receipt).unwrap()).unwrap();
    assert_eq!(
        receipt_value["body"]["pull_request"]["requested_base_oid"],
        fixture.updated_base
    );
    assert_eq!(
        receipt_value["body"]["pull_request"]["reviewed_merge_base_oid"],
        fixture.base
    );
    assert_eq!(
        receipt_value["body"]["pull_request"]["reviewed_head_oid"],
        fixture.equivalent
    );

    let output = run_receipt_issue(ReceiptIssueCommand {
        repository: directory,
        context: &directory.join("receipt-context.json"),
        input: &receipt.input,
        payload: &receipt.payload,
        result: &receipt.result,
        expected_base: &fixture.base,
        expected_head: &fixture.equivalent,
        prior: None,
        trust_policy_sha256: &receipt.trust_policy_sha256,
        signing_key: &directory.join("receipt-signing-key.hex"),
        output: &directory.join("stale-live-base-receipt.json"),
    });
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "adapter-observed live base tip or head differs from the routed review snapshots"
    ));
}
