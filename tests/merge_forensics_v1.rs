use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stratadiff::doctor::evaluate_pull_request_doctor_v3;
use stratadiff::readiness_audit::{
    GithubPullRequestDoctorApi, GithubReadinessApi, GithubReadinessApiResponse,
    PullRequestDoctorCollection, collect_pull_request_doctor_snapshot_v3,
};
use tempfile::TempDir;

const CASES_SCHEMA: &str = "stratadiff-merge-forensics-cases-v1";
const TRANSCRIPTS_SCHEMA: &str = "stratadiff-merge-forensics-transcripts-v1";
const PREDICTIONS_SCHEMA: &str = "stratadiff-merge-forensics-predictions-v1";
const DATASET_VERSION: &str = "1.0.0";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CasesAsset {
    schema: String,
    dataset_version: String,
    cases: Vec<CaseRef>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseRef {
    id: String,
    transcript_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptsAsset {
    schema: String,
    dataset_version: String,
    query_contracts: Vec<QueryContract>,
    payloads: Vec<Payload>,
    transcripts: Vec<Transcript>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryContract {
    id: String,
    operation_name: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    id: String,
    media_type: String,
    body_utf8: String,
    byte_length: usize,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transcript {
    id: String,
    case_id: String,
    request: TranscriptRequest,
    exchanges: Vec<Exchange>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptRequest {
    provider_url: String,
    repository: String,
    captured_at: String,
    pull_request_number: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case")]
enum Exchange {
    Rest {
        sequence: usize,
        request: RestRequest,
        response: ResponseRef,
    },
    Graphql {
        sequence: usize,
        request: GraphqlRequest,
        response: ResponseRef,
    },
}

impl Exchange {
    fn sequence(&self) -> usize {
        match self {
            Self::Rest { sequence, .. } | Self::Graphql { sequence, .. } => *sequence,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestRequest {
    method: String,
    endpoint: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphqlRequest {
    query_contract_id: String,
    variables: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseRef {
    status: u16,
    link_header: Option<String>,
    payload_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredictionsAsset {
    schema: String,
    dataset_version: String,
    cases: Vec<Value>,
}

struct TranscriptApi {
    exchanges: VecDeque<Exchange>,
    payloads: BTreeMap<String, Vec<u8>>,
    query_contracts: BTreeMap<String, String>,
}

impl TranscriptApi {
    fn new(asset: &TranscriptsAsset, transcript: &Transcript) -> Self {
        let payloads = asset
            .payloads
            .iter()
            .map(|payload| {
                assert_eq!(payload.media_type, "application/json");
                let bytes = payload.body_utf8.as_bytes();
                assert_eq!(bytes.len(), payload.byte_length, "payload {}", payload.id);
                assert_eq!(sha256(bytes), payload.sha256, "payload {}", payload.id);
                (payload.id.clone(), bytes.to_vec())
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(payloads.len(), asset.payloads.len());

        let query_contracts = asset
            .query_contracts
            .iter()
            .map(|contract| {
                assert_eq!(contract.operation_name, "StrataDiffPullRequestCandidate");
                assert_eq!(contract.sha256.len(), 64);
                (contract.id.clone(), contract.sha256.clone())
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(query_contracts.len(), asset.query_contracts.len());

        for (index, exchange) in transcript.exchanges.iter().enumerate() {
            assert_eq!(
                exchange.sequence(),
                index + 1,
                "transcript {}",
                transcript.id
            );
        }

        Self {
            exchanges: transcript.exchanges.clone().into(),
            payloads,
            query_contracts,
        }
    }

    fn response(&self, response: ResponseRef) -> Result<GithubReadinessApiResponse> {
        let body = self
            .payloads
            .get(&response.payload_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown transcript payload {}", response.payload_id))?;
        Ok(GithubReadinessApiResponse {
            status: response.status,
            body,
            link_header: response.link_header,
        })
    }

    fn finish(&self) {
        assert!(
            self.exchanges.is_empty(),
            "{} transcript exchanges were not consumed",
            self.exchanges.len()
        );
    }
}

impl GithubReadinessApi for TranscriptApi {
    fn get(&mut self, endpoint: &str) -> Result<GithubReadinessApiResponse> {
        let exchange = self
            .exchanges
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("unexpected REST GET {endpoint}"))?;
        match exchange {
            Exchange::Rest {
                request, response, ..
            } => {
                ensure!(request.method == "GET", "transcript REST method is not GET");
                ensure!(
                    request.endpoint == endpoint,
                    "REST transcript mismatch: expected {}, observed {endpoint}",
                    request.endpoint
                );
                self.response(response)
            }
            Exchange::Graphql { .. } => {
                bail!("expected GraphQL exchange, observed REST {endpoint}")
            }
        }
    }
}

impl GithubPullRequestDoctorApi for TranscriptApi {
    fn graphql(&mut self, query: &str, variables: &Value) -> Result<GithubReadinessApiResponse> {
        let exchange = self
            .exchanges
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("unexpected GraphQL request"))?;
        match exchange {
            Exchange::Graphql {
                request, response, ..
            } => {
                let expected_query_sha = &self.query_contracts[&request.query_contract_id];
                ensure!(
                    sha256(query.as_bytes()) == *expected_query_sha,
                    "GraphQL query contract changed"
                );
                ensure!(
                    request.variables == *variables,
                    "GraphQL variables differ from the transcript"
                );
                self.response(response)
            }
            Exchange::Rest { request, .. } => bail!(
                "expected REST {} {}, observed GraphQL",
                request.method,
                request.endpoint
            ),
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn benchmark_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benchmarks/merge-forensics-v1")
}

fn load_assets() -> (CasesAsset, TranscriptsAsset, PredictionsAsset) {
    let root = benchmark_root();
    let cases = serde_json::from_slice(&std::fs::read(root.join("cases.json")).unwrap()).unwrap();
    let transcripts =
        serde_json::from_slice(&std::fs::read(root.join("transcripts.json")).unwrap()).unwrap();
    let predictions =
        serde_json::from_slice(&std::fs::read(root.join("baseline-predictions.json")).unwrap())
            .unwrap();
    (cases, transcripts, predictions)
}

fn report_prediction(case_id: &str, report: &Value) -> Value {
    let requirements = report["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|requirement| {
            let evidence = requirement["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|evidence| {
                    json!({
                        "kind": evidence["kind"],
                        "id": evidence["id"],
                        "sha": evidence["sha"],
                        "context": evidence["context"],
                        "app_id": evidence["app_id"],
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "context": requirement["key"]["context"],
                "expected_app_id": requirement["key"]["expected_app_id"],
                "policies": requirement["policies"].as_array().unwrap().iter().map(|policy| json!({
                    "kind": policy["kind"],
                    "id": policy["id"],
                    "name": policy["name"],
                    "url": policy["url"],
                })).collect::<Vec<_>>(),
                "status": requirement["status"],
                "evidence": evidence,
            })
        })
        .collect::<Vec<_>>();
    let workflow_diagnoses = report["workflow_trigger_diagnoses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let producer = item["producer"].as_object().map(|producer| {
                json!({
                    "source_sha": producer["source_sha"],
                    "check_run_id": producer["check_run_id"],
                    "check_name": producer["check_name"],
                    "app_id": producer["app_id"],
                    "app_slug": producer["app_slug"],
                    "check_suite_id": producer["check_suite_id"],
                    "workflow_run_id": producer["workflow_run_id"],
                    "workflow_run_path": producer["workflow_run_path"],
                    "workflow_job_id": producer["workflow_job_id"],
                    "workflow_id": producer["workflow_id"],
                    "workflow_path": producer["workflow_path"],
                })
            });
            let diagnosis = item["diagnosis"].as_object().map(|diagnosis| {
                json!({
                    "cause_code": diagnosis["cause_code"],
                    "confidence": diagnosis["confidence"],
                    "evidence": diagnosis["evidence"],
                    "fix_action_code": diagnosis["fix"]["action_code"],
                })
            });
            json!({
                "requirement": item["requirement"],
                "producer": producer,
                "diagnosis": diagnosis,
            })
        })
        .collect::<Vec<_>>();
    let next_action_codes = report["next_actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["code"].clone())
        .collect::<Vec<_>>();

    json!({
        "case_id": case_id,
        "execution": {"status": "report", "error_kind": null},
        "target": {
            "kind": report["target"]["evaluation"]["kind"],
            "resolution": report["target"]["evaluation"]["resolution"],
            "sha": report["target"]["evaluation"]["sha"],
            "base_sha": report["target"]["evaluation"]["base_sha"],
            "queue_entry_id": report["target"]["evaluation"]["queue_entry_id"],
            "queue_state": report["target"]["evaluation"]["queue_state"],
        },
        "verdict": report["verdict"],
        "requirements": requirements,
        "workflow_diagnoses": workflow_diagnoses,
        "next_action_codes": next_action_codes,
    })
}

fn retry_prediction(case_id: &str, error: &anyhow::Error) -> Value {
    let message = format!("{error:#}");
    let error_kind = if message.contains("workflow producer evidence changed during diagnosis") {
        "producer_drift"
    } else if message.contains("pull-request doctor evidence changed")
        || message.contains("candidate changed")
        || message.contains("target identity")
    {
        "target_drift"
    } else {
        panic!("case {case_id} failed outside the benchmark retry contract: {message}");
    };
    json!({
        "case_id": case_id,
        "execution": {"status": "retry", "error_kind": error_kind},
        "target": null,
        "verdict": null,
        "requirements": [],
        "workflow_diagnoses": [],
        "next_action_codes": [],
    })
}

fn replay_predictions() -> Value {
    let (cases, transcript_asset, _) = load_assets();
    assert_eq!(cases.schema, CASES_SCHEMA);
    assert_eq!(transcript_asset.schema, TRANSCRIPTS_SCHEMA);
    assert_eq!(cases.dataset_version, DATASET_VERSION);
    assert_eq!(transcript_asset.dataset_version, DATASET_VERSION);
    assert_eq!(cases.cases.len(), 12);

    let transcripts = transcript_asset
        .transcripts
        .iter()
        .map(|transcript| (transcript.id.as_str(), transcript))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(transcripts.len(), transcript_asset.transcripts.len());
    let mut case_ids = BTreeSet::new();
    let mut predictions = Vec::new();
    for case in &cases.cases {
        assert!(
            case_ids.insert(case.id.as_str()),
            "duplicate case {}",
            case.id
        );
        let transcript = transcripts[case.transcript_id.as_str()];
        assert_eq!(transcript.case_id, case.id);
        let mut api = TranscriptApi::new(&transcript_asset, transcript);
        let request = PullRequestDoctorCollection {
            provider_url: &transcript.request.provider_url,
            repository: &transcript.request.repository,
            captured_at: &transcript.request.captured_at,
            pull_request_number: transcript.request.pull_request_number,
        };
        let prediction = match collect_pull_request_doctor_snapshot_v3(request, &mut api) {
            Ok(snapshot) => {
                let report = evaluate_pull_request_doctor_v3(&snapshot).unwrap_or_else(|error| {
                    panic!("case {} report evaluation failed: {error:#}", case.id)
                });
                report_prediction(&case.id, &serde_json::to_value(report).unwrap())
            }
            Err(error) => retry_prediction(&case.id, &error),
        };
        api.finish();
        predictions.push(prediction);
    }

    json!({
        "schema": PREDICTIONS_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "cases": predictions,
    })
}

#[test]
fn raw_provider_transcripts_match_the_frozen_baseline() {
    let (_, _, expected) = load_assets();
    assert_eq!(expected.schema, PREDICTIONS_SCHEMA);
    assert_eq!(expected.dataset_version, DATASET_VERSION);
    assert_eq!(replay_predictions()["cases"], json!(expected.cases));
}

#[test]
fn independent_verifier_accepts_the_rust_replay() {
    let output_directory = TempDir::new().unwrap();
    let prediction_path = output_directory.path().join("predictions.json");
    std::fs::write(
        &prediction_path,
        serde_json::to_vec_pretty(&replay_predictions()).unwrap(),
    )
    .unwrap();
    let output = Command::new("python3")
        .arg("-B")
        .arg(benchmark_root().join("verify.py"))
        .arg("score")
        .arg(&prediction_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "score failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn benchmark_bundle_verifier_and_self_test_pass() {
    for command in ["verify", "self-test"] {
        let output = Command::new("python3")
            .arg("-B")
            .arg(benchmark_root().join("verify.py"))
            .arg(command)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn production_sources_do_not_depend_on_benchmark_cases_or_oracle() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut directories = vec![repository_root.join("src")];
    let crates_root = repository_root.join("crates");
    if crates_root.is_dir() {
        directories.push(crates_root);
    }
    let mut sources = vec![repository_root.join("build.rs")];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }
    for path in sources {
        if !path.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(!source.contains("merge-forensics-v1"), "{}", path.display());
        assert!(!source.contains("oracle.json"), "{}", path.display());
        for index in 1..=12 {
            let case_id = format!("c{index:03}");
            assert!(
                !source.contains(&case_id),
                "{} contains {case_id}",
                path.display()
            );
        }
    }
}
