use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitCode, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::{ArgGroup, Parser};
use serde::Serialize;
use sha2::{Digest, Sha256};
use stratadiff::diffbenchmark::{OffsetRange, comparable_tree_sitter_java_nodes, parse_god_report};
use stratadiff::diffbenchmark_case::{
    CategoryCoverageLedger, CoverageLedger, EndpointSide, adapt_intra_file_case,
};
use stratadiff::diffbenchmark_eval::{
    AmbiguityScore, CaseEvaluationInput, CategoryScore, ExactRelationScore, MultiplicityLaneScore,
    RepresentationWarning, UnscoredPredictionCounts, evaluate_case,
};
use stratadiff::diffbenchmark_materialization::{
    DIFFBENCHMARK_LITERATURE_CASES, DIFFBENCHMARK_REVISION, MATERIALIZATION_MANIFEST_SCHEMA,
    MaterializationManifest, MaterializedCase,
};
use stratadiff::diffbenchmark_prediction::{
    BridgeCoverage, EnumeratedJdtNode, PredictionAdapterDiagnostics, PredictionAdapterInput,
    adapt_predictions,
};
use stratadiff::{Language, VerificationLimits, analyze_bytes, verify_report_with_limits};

const MANIFEST_NAME: &str = "manifest.json";
const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";
const INFO_CSV: &str = "info.csv";
const LITERATURE_CSV: &str = "csv-outputs/adb-paper/literature-exp-INTRA_FILE_ONLY-NO_FILTER-RefOracle-NO_COMMENTS_AND_JAVADOCS-2025_04_10 18:15:50.csv";
const EVALUATION_REPORT_SCHEMA: &str = "stratadiff-diffbenchmark-evaluation-v3";
const CANONICAL_MATERIALIZATION_MANIFEST_BLAKE3: &str =
    "0012eecb59360ef45e9ccc2ecaa9c11ca1387bfa6c391238d0301a84ee44d9d3";
const BUILD_GIT_REVISION: &str = env!("STRATADIFF_BUILD_GIT_REVISION");
const BUILD_GIT_DIRTY: &str = env!("STRATADIFF_BUILD_GIT_DIRTY");
const BUILD_CARGO_LOCK_SHA256: &str = env!("STRATADIFF_BUILD_CARGO_LOCK_SHA256");
const BUILD_PROFILE: &str = env!("STRATADIFF_BUILD_PROFILE");
const JDT_PROFILE: &str = "gumtree-3.0.0-jdt-core-3.35.0-ecj-3.35.0-helper-v3";
const JDT_PROTOCOL: &str = "stratadiff-jdt-tsv-v2";
const JDT_ENUMERATION_BASIS: &str =
    "verified_helper_unique_identity_root_preorder_then_comment_table";
const GUMTREE_JDT_GENERATOR_VERSION: &str = "3.0.0";
const ECLIPSE_JDT_CORE_VERSION: &str = "3.35.0";
const ECJ_VERSION: &str = "3.35.0";
const GUMTREE_FAT_JAR_SHA256: &str =
    "959404693f963f658ff2c6a9111eca9fa392a845ce656613178c96994515f909";
const ECLIPSE_JDT_CORE_JAR_SHA256: &str =
    "8f5bcb00355b271638b9d82a8cccd0e733225cb74c4a3f0f55a7b75d43ee442a";
const ECJ_JAR_SHA256: &str = "b89df382369f2d16b19b67085426dc0fb7401fe1ad9fba1806af4e9729f5d1c0";
const ENUMERATE_JDT_SOURCE: &[u8] =
    include_bytes!("../../tools/diffbenchmark/jdt/EnumerateJdt.java.source");
const JAVA_TRUST_BOUNDARY: &str = "caller_selected_local_executable";
const JAVA_VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const JAVA_VERSION_OUTPUT_LIMIT: usize = 64 * 1024;
const JAVA_VERSION_MAX_HEAP: &str = "-Xmx128m";
const JDT_ENUMERATOR_TIMEOUT: Duration = Duration::from_secs(300);
const JDT_ENUMERATOR_STDOUT_LIMIT: usize = 512 * 1024 * 1024;
const JDT_ENUMERATOR_STDERR_LIMIT: usize = 1024 * 1024;
const JDT_ENUMERATOR_NODE_LIMIT: usize = 10_000_000;
const JDT_ENUMERATOR_MAX_HEAP: &str = "-Xmx1024m";
const KNOWN_MALFORMED_ORACLE_PATH: &str = "hrd-oracle/adb-paper/literature-exp/apache.hive/5f78f9ef1e6c798849d34cc66721e6c1d9709b6f/ql.src.test.org.apache.hadoop.hive.ql.io.orc.TestInputOutputFormat/GOD.json";
const KNOWN_MALFORMED_ORACLE_BLAKE3: &str =
    "3a2a4c674f7e549421562088d73a3ef096986004923355429d5d6f99a912af9a";
const KNOWN_MALFORMED_SOURCE_ORACLE_PATH: &str = "hrd-oracle/adb-paper/literature-exp/Alluxio.alluxio/0ba343846f21649e29ffc600f30a7f3e463fb24c/servers.src.main.java.tachyon.worker.block.meta.BlockMeta/GOD.json";
const KNOWN_MALFORMED_SOURCE_ORACLE_BLAKE3: &str =
    "3f8dc9bebb9f6c1298ce96de408fbd3719d547aa0084407493f584e454140975";
const KNOWN_MALFORMED_SOURCE_COMMIT: &str = "0ba343846f21649e29ffc600f30a7f3e463fb24c";
const KNOWN_MALFORMED_SOURCE_PARENT: &str = "317054bc8e079baed535bede9f6c025e5d756a1b";
const KNOWN_MALFORMED_SOURCE_PATH: &str =
    "servers/src/main/java/tachyon/worker/block/meta/BlockMeta.java";
const KNOWN_MALFORMED_SOURCE_BEFORE_BLAKE3: &str =
    "3978fde55c4a96218349ed01fcaab702aac7441660eab0071cd717a0dc99ec79";
const KNOWN_MALFORMED_SOURCE_AFTER_BLAKE3: &str =
    "6ef87c9fb037c1d760684a5457055f781801279be335b82c78bac5a85c73e7d6";

#[derive(Debug, Parser)]
#[command(name = "stratadiff-evaluate")]
#[command(about = "Evaluate StrataDiff on a materialized pinned DiffBenchmark corpus")]
#[command(version)]
#[command(group(
    ArgGroup::new("jdt_provider")
        .required(true)
        .args(["jdt_cache", "jdt_enumerator"])
))]
struct Cli {
    /// Root of the pinned DiffBenchmark checkout.
    checkout: PathBuf,
    /// Root containing the materialization manifest and source cache.
    materialization_root: PathBuf,
    /// Bootstrap cache whose pinned JDT artifacts and helper will be verified.
    #[arg(
        long,
        value_name = "DIRECTORY",
        conflicts_with = "jdt_enumerator",
        requires = "java_executable"
    )]
    jdt_cache: Option<PathBuf>,
    /// Trusted local Java executable; its canonical path must match the cache record.
    #[arg(
        long,
        value_name = "EXECUTABLE",
        requires = "jdt_cache",
        conflicts_with = "jdt_enumerator"
    )]
    java_executable: Option<PathBuf>,
    /// Unverified executable implementing the JDT enumeration TSV protocol.
    #[arg(
        long,
        value_name = "EXECUTABLE",
        requires = "allow_unverified_jdt_enumerator",
        conflicts_with = "jdt_cache"
    )]
    jdt_enumerator: Option<PathBuf>,
    /// Explicitly permit an unverified JDT enumerator; its report has no version claims.
    #[arg(long, requires = "jdt_enumerator")]
    allow_unverified_jdt_enumerator: bool,
    /// Write the pretty JSON report to this path instead of stdout.
    #[arg(long, value_name = "JSON")]
    output: Option<PathBuf>,
    /// Evaluate only the first N manifest cases after validating the manifest header.
    #[arg(long, value_name = "N")]
    limit: Option<usize>,
    /// Exit unsuccessfully after writing the report unless the benchmark is complete.
    #[arg(long)]
    require_complete: bool,
}

#[derive(Clone, Debug)]
struct CaseIdentity {
    index: usize,
    oracle_path: String,
    commit: String,
    before_repository_path: String,
    after_repository_path: String,
    before_materialized_path: String,
    after_materialized_path: String,
}

struct ReadyCase {
    identity: CaseIdentity,
    oracle: stratadiff::diffbenchmark::GodReport,
    before_path: PathBuf,
    after_path: PathBuf,
    before_source: Vec<u8>,
    after_source: Vec<u8>,
}

enum JdtRuntime {
    Verified(VerifiedJdtCache),
    Unverified {
        executable: PathBuf,
        executable_blake3: String,
    },
}

struct VerifiedJdtCache {
    _runtime_directory: tempfile::TempDir,
    cache_root: PathBuf,
    java_executable: PathBuf,
    java_runtime_version: String,
    gumtree_fat_jar: PathBuf,
    eclipse_jdt_core_jar: PathBuf,
    ecj_jar: PathBuf,
    helper_source: PathBuf,
    helper_source_sha256: String,
}

struct JdtArtifactDigests<'a> {
    gumtree_fat_jar: &'a str,
    eclipse_jdt_core_jar: &'a str,
    ecj_jar: &'a str,
}

const JDT_ARTIFACT_DIGESTS: JdtArtifactDigests<'static> = JdtArtifactDigests {
    gumtree_fat_jar: GUMTREE_FAT_JAR_SHA256,
    eclipse_jdt_core_jar: ECLIPSE_JDT_CORE_JAR_SHA256,
    ecj_jar: ECJ_JAR_SHA256,
};

enum PreparedCase {
    Ready(Box<ReadyCase>),
    Finished(Box<CaseReport>),
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaseStage {
    InputValidation,
    OracleParse,
    OracleAdaptation,
    Analysis,
    ReportSerialization,
    Verification,
    PredictionAdaptation,
    Evaluation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    index: usize,
    oracle_path: String,
    commit: String,
    before_repository_path: String,
    after_repository_path: String,
    before_materialized_path: String,
    after_materialized_path: String,
    outcome: CaseOutcome,
}

#[derive(Clone, Copy)]
struct CaseMeasurements {
    analysis_latency_micros: u64,
    combined_input_bytes: usize,
    serialized_diff_report_bytes: usize,
    verification_work: usize,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum CaseOutcome {
    Evaluated {
        analysis_latency_micros: u64,
        combined_input_bytes: usize,
        serialized_diff_report_bytes: usize,
        verification_work: usize,
        program_elements: Box<CaseCategorySummary>,
        mappings: Box<CaseCategorySummary>,
        pooled_category_observations: Box<CaseCategorySummary>,
        prediction_diagnostics: Box<PredictionDiagnosticsSummary>,
    },
    KnownMalformedOracle {
        error: String,
    },
    KnownMalformedSource {
        side: EndpointSide,
        content_blake3: &'static str,
        error: String,
    },
    Error {
        stage: CaseStage,
        error: String,
        analysis_latency_micros: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExactCounts {
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
}

impl ExactCounts {
    fn from_score(score: ExactRelationScore) -> Self {
        Self {
            true_positives: score.true_positives,
            false_positives: score.false_positives,
            false_negatives: score.false_negatives,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            true_positives: self.true_positives + other.true_positives,
            false_positives: self.false_positives + other.false_positives,
            false_negatives: self.false_negatives + other.false_negatives,
        }
    }

    fn metrics(self) -> ExactMetrics {
        ExactMetrics {
            true_positives: self.true_positives,
            false_positives: self.false_positives,
            false_negatives: self.false_negatives,
            precision: ratio(
                self.true_positives,
                self.true_positives + self.false_positives,
            ),
            recall: ratio(
                self.true_positives,
                self.true_positives + self.false_negatives,
            ),
            f1: f1(self),
            unforced_gold_relation_rate: ratio(
                self.false_negatives,
                self.true_positives + self.false_negatives,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExactMetrics {
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
    unforced_gold_relation_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct LaneCounts {
    oracle_relations: usize,
    forced_true_positives: usize,
    forced_false_negatives: usize,
}

impl LaneCounts {
    fn from_score(score: MultiplicityLaneScore) -> Self {
        Self {
            oracle_relations: score.oracle_relations,
            forced_true_positives: score.forced_true_positives,
            forced_false_negatives: score.forced_false_negatives,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            oracle_relations: self.oracle_relations + other.oracle_relations,
            forced_true_positives: self.forced_true_positives + other.forced_true_positives,
            forced_false_negatives: self.forced_false_negatives + other.forced_false_negatives,
        }
    }

    fn metrics(self) -> LaneMetrics {
        LaneMetrics {
            oracle_relations: self.oracle_relations,
            forced_true_positives: self.forced_true_positives,
            forced_false_negatives: self.forced_false_negatives,
            recall: ratio(self.forced_true_positives, self.oracle_relations),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaneMetrics {
    oracle_relations: usize,
    forced_true_positives: usize,
    forced_false_negatives: usize,
    recall: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct AmbiguityCounts {
    oracle_multi_relations: usize,
    predicted_candidates: usize,
    covered_multi_relations: usize,
    missed_multi_relations: usize,
    extra_candidates: usize,
}

impl AmbiguityCounts {
    fn from_score(score: AmbiguityScore) -> Self {
        Self {
            oracle_multi_relations: score.oracle_multi_relations,
            predicted_candidates: score.predicted_candidates,
            covered_multi_relations: score.covered_multi_relations,
            missed_multi_relations: score.missed_multi_relations,
            extra_candidates: score.extra_candidates,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            oracle_multi_relations: self.oracle_multi_relations + other.oracle_multi_relations,
            predicted_candidates: self.predicted_candidates + other.predicted_candidates,
            covered_multi_relations: self.covered_multi_relations + other.covered_multi_relations,
            missed_multi_relations: self.missed_multi_relations + other.missed_multi_relations,
            extra_candidates: self.extra_candidates + other.extra_candidates,
        }
    }

    fn metrics(self) -> AmbiguityMetrics {
        AmbiguityMetrics {
            oracle_multi_relations: self.oracle_multi_relations,
            predicted_candidates: self.predicted_candidates,
            covered_multi_relations: self.covered_multi_relations,
            missed_multi_relations: self.missed_multi_relations,
            extra_candidates: self.extra_candidates,
            coverage: ratio(self.covered_multi_relations, self.oracle_multi_relations),
            expansion: ratio(self.predicted_candidates, self.covered_multi_relations),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AmbiguityMetrics {
    oracle_multi_relations: usize,
    predicted_candidates: usize,
    covered_multi_relations: usize,
    missed_multi_relations: usize,
    extra_candidates: usize,
    coverage: Option<f64>,
    expansion: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct UnscoredCounts {
    forced: usize,
    ambiguity_candidates: usize,
}

impl UnscoredCounts {
    fn from_score(score: UnscoredPredictionCounts) -> Self {
        Self {
            forced: score.forced,
            ambiguity_candidates: score.ambiguity_candidates,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            forced: self.forced + other.forced,
            ambiguity_candidates: self.ambiguity_candidates + other.ambiguity_candidates,
        }
    }

    fn metrics(self) -> UnscoredMetrics {
        UnscoredMetrics {
            forced: self.forced,
            ambiguity_candidates: self.ambiguity_candidates,
            total: self.forced + self.ambiguity_candidates,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnscoredMetrics {
    forced: usize,
    ambiguity_candidates: usize,
    total: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct CoverageCounts {
    raw_relations: usize,
    scorable_relations: usize,
    excluded_relations: usize,
}

impl CoverageCounts {
    fn from_ledger(ledger: &CategoryCoverageLedger) -> Self {
        Self {
            raw_relations: ledger.raw_relations,
            scorable_relations: ledger.scorable_relations,
            excluded_relations: ledger.excluded_relations,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            raw_relations: self.raw_relations + other.raw_relations,
            scorable_relations: self.scorable_relations + other.scorable_relations,
            excluded_relations: self.excluded_relations + other.excluded_relations,
        }
    }

    fn metrics(self) -> CoverageMetrics {
        CoverageMetrics {
            raw_relations: self.raw_relations,
            scorable_relations: self.scorable_relations,
            excluded_relations: self.excluded_relations,
            scorable_rate: ratio(self.scorable_relations, self.raw_relations),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoverageMetrics {
    raw_relations: usize,
    scorable_relations: usize,
    excluded_relations: usize,
    scorable_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct CaseCategorySummary {
    exact: ExactCounts,
    ambiguity_covered_oracle_relations: usize,
    singleton: LaneCounts,
    multi: LaneCounts,
    ambiguity: AmbiguityCounts,
    representation: RepresentationCounts,
    unscored: UnscoredCounts,
    coverage: CoverageCounts,
}

impl CaseCategorySummary {
    fn new(score: CategoryScore, coverage: &CategoryCoverageLedger) -> Self {
        Self {
            exact: ExactCounts::from_score(score.exact_relations),
            ambiguity_covered_oracle_relations: score.ambiguity_covered_oracle_relations,
            singleton: LaneCounts::from_score(score.singleton_relations),
            multi: LaneCounts::from_score(score.multi_relations),
            ambiguity: AmbiguityCounts::from_score(score.ambiguity),
            representation: RepresentationCounts::from_score(score.representation_warning),
            unscored: UnscoredCounts::from_score(score.unscored_predictions),
            coverage: CoverageCounts::from_ledger(coverage),
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            exact: self.exact.add(other.exact),
            ambiguity_covered_oracle_relations: self.ambiguity_covered_oracle_relations
                + other.ambiguity_covered_oracle_relations,
            singleton: self.singleton.add(other.singleton),
            multi: self.multi.add(other.multi),
            ambiguity: self.ambiguity.add(other.ambiguity),
            representation: self.representation.add(other.representation),
            unscored: self.unscored.add(other.unscored),
            coverage: self.coverage.add(other.coverage),
        }
    }
}

impl Serialize for CaseCategorySummary {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableCategorySummary {
            exact_relations: self.exact.metrics(),
            ambiguity_covered_gold_relation_rate: RateSummary::new(
                self.ambiguity_covered_oracle_relations,
                self.exact.true_positives + self.exact.false_negatives,
            ),
            singleton_relations: self.singleton.metrics(),
            multi_relations: self.multi.metrics(),
            ambiguity: self.ambiguity.metrics(),
            representation_warning: self.representation.metrics(),
            unscored_predictions: self.unscored.metrics(),
            coverage: self.coverage.metrics(),
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SerializableCategorySummary {
    exact_relations: ExactMetrics,
    ambiguity_covered_gold_relation_rate: RateSummary,
    singleton_relations: LaneMetrics,
    multi_relations: LaneMetrics,
    ambiguity: AmbiguityMetrics,
    representation_warning: RepresentationMetrics,
    unscored_predictions: UnscoredMetrics,
    coverage: CoverageMetrics,
}

#[derive(Clone, Copy, Debug, Default)]
struct RepresentationCounts {
    eligible_multi_groups: usize,
    forced_touched_multi_groups: usize,
    forced_gold_edges_in_multi_groups: usize,
    forced_false_positive_edges_incident_to_multi_groups: usize,
}

impl RepresentationCounts {
    fn from_score(score: RepresentationWarning) -> Self {
        Self {
            eligible_multi_groups: score.eligible_multi_groups,
            forced_touched_multi_groups: score.forced_touched_multi_groups,
            forced_gold_edges_in_multi_groups: score.forced_gold_edges_in_multi_groups,
            forced_false_positive_edges_incident_to_multi_groups: score
                .forced_false_positive_edges_incident_to_multi_groups,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            eligible_multi_groups: self.eligible_multi_groups + other.eligible_multi_groups,
            forced_touched_multi_groups: self.forced_touched_multi_groups
                + other.forced_touched_multi_groups,
            forced_gold_edges_in_multi_groups: self.forced_gold_edges_in_multi_groups
                + other.forced_gold_edges_in_multi_groups,
            forced_false_positive_edges_incident_to_multi_groups: self
                .forced_false_positive_edges_incident_to_multi_groups
                + other.forced_false_positive_edges_incident_to_multi_groups,
        }
    }

    fn metrics(self) -> RepresentationMetrics {
        RepresentationMetrics {
            eligible_multi_groups: self.eligible_multi_groups,
            forced_touched_multi_groups: self.forced_touched_multi_groups,
            forced_gold_edges_in_multi_groups: self.forced_gold_edges_in_multi_groups,
            forced_false_positive_edges_incident_to_multi_groups: self
                .forced_false_positive_edges_incident_to_multi_groups,
            multi_group_overclaim_rate: ratio(
                self.forced_touched_multi_groups,
                self.eligible_multi_groups,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepresentationMetrics {
    eligible_multi_groups: usize,
    forced_touched_multi_groups: usize,
    forced_gold_edges_in_multi_groups: usize,
    forced_false_positive_edges_incident_to_multi_groups: usize,
    multi_group_overclaim_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct BridgeCounts {
    enumerated_nodes: usize,
    supported_nodes: usize,
    bridged_nodes: usize,
    unsupported_nodes: usize,
    unresolved_supported_nodes: usize,
}

impl BridgeCounts {
    fn from_coverage(coverage: BridgeCoverage) -> Self {
        Self {
            enumerated_nodes: coverage.enumerated_nodes,
            supported_nodes: coverage.supported_nodes,
            bridged_nodes: coverage.bridged_nodes,
            unsupported_nodes: coverage.unsupported_nodes,
            unresolved_supported_nodes: coverage.unresolved_supported_nodes,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            enumerated_nodes: self.enumerated_nodes + other.enumerated_nodes,
            supported_nodes: self.supported_nodes + other.supported_nodes,
            bridged_nodes: self.bridged_nodes + other.bridged_nodes,
            unsupported_nodes: self.unsupported_nodes + other.unsupported_nodes,
            unresolved_supported_nodes: self.unresolved_supported_nodes
                + other.unresolved_supported_nodes,
        }
    }

    fn metrics(self) -> BridgeMetrics {
        BridgeMetrics {
            enumerated_nodes: self.enumerated_nodes,
            supported_nodes: self.supported_nodes,
            bridged_nodes: self.bridged_nodes,
            unsupported_nodes: self.unsupported_nodes,
            unresolved_supported_nodes: self.unresolved_supported_nodes,
            taxonomy_coverage: ratio(self.supported_nodes, self.enumerated_nodes),
            resolution_rate: ratio(self.bridged_nodes, self.supported_nodes),
            end_to_end_coverage: ratio(self.bridged_nodes, self.enumerated_nodes),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeMetrics {
    enumerated_nodes: usize,
    supported_nodes: usize,
    bridged_nodes: usize,
    unsupported_nodes: usize,
    unresolved_supported_nodes: usize,
    taxonomy_coverage: Option<f64>,
    resolution_rate: Option<f64>,
    end_to_end_coverage: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PredictionDiagnosticsSummary {
    before_bridge: BridgeMetrics,
    after_bridge: BridgeMetrics,
    combined_bridge: BridgeMetrics,
    ignored_input_pair_relations: usize,
    ignored_suggested_relations: usize,
}

impl PredictionDiagnosticsSummary {
    fn from_diagnostics(diagnostics: PredictionAdapterDiagnostics) -> Self {
        let before = BridgeCounts::from_coverage(diagnostics.before_bridge);
        let after = BridgeCounts::from_coverage(diagnostics.after_bridge);
        Self {
            before_bridge: before.metrics(),
            after_bridge: after.metrics(),
            combined_bridge: before.add(after).metrics(),
            ignored_input_pair_relations: diagnostics.ignored_input_pair_relations,
            ignored_suggested_relations: diagnostics.ignored_suggested_relations,
        }
    }
}

#[derive(Default)]
struct AverageAccumulator {
    sum: f64,
    count: usize,
}

impl AverageAccumulator {
    fn add(&mut self, value: Option<f64>) {
        if let Some(value) = value {
            self.sum += value;
            self.count += 1;
        }
    }

    fn summary(&self) -> AverageSummary {
        AverageSummary {
            value: (self.count != 0).then(|| self.sum / self.count as f64),
            defined_cases: self.count,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AverageSummary {
    value: Option<f64>,
    defined_cases: usize,
}

#[derive(Default)]
struct CategoryAccumulator {
    perfect_exact_forced_gold_bearing_cases: usize,
    gold_bearing_cases: usize,
    exact: ExactCounts,
    ambiguity_covered_oracle_relations: usize,
    singleton: LaneCounts,
    multi: LaneCounts,
    ambiguity: AmbiguityCounts,
    representation: RepresentationCounts,
    unscored: UnscoredCounts,
    coverage: CoverageCounts,
    precision: AverageAccumulator,
    recall: AverageAccumulator,
    f1: AverageAccumulator,
}

impl CategoryAccumulator {
    fn add(&mut self, summary: CaseCategorySummary) {
        if summary.exact.true_positives + summary.exact.false_negatives != 0 {
            self.gold_bearing_cases += 1;
            if summary.exact.false_positives == 0 && summary.exact.false_negatives == 0 {
                self.perfect_exact_forced_gold_bearing_cases += 1;
            }
        }
        let metrics = summary.exact.metrics();
        self.precision.add(metrics.precision);
        self.recall.add(metrics.recall);
        self.f1.add(metrics.f1);
        self.exact = self.exact.add(summary.exact);
        self.ambiguity_covered_oracle_relations += summary.ambiguity_covered_oracle_relations;
        self.singleton = self.singleton.add(summary.singleton);
        self.multi = self.multi.add(summary.multi);
        self.ambiguity = self.ambiguity.add(summary.ambiguity);
        self.representation = self.representation.add(summary.representation);
        self.unscored = self.unscored.add(summary.unscored);
        self.coverage = self.coverage.add(summary.coverage);
    }

    fn summary(&self) -> AggregateCategorySummary {
        AggregateCategorySummary {
            micro: self.exact.metrics(),
            ambiguity_covered_gold_relation_rate: RateSummary::new(
                self.ambiguity_covered_oracle_relations,
                self.exact.true_positives + self.exact.false_negatives,
            ),
            macro_average: MacroSummary {
                precision: self.precision.summary(),
                recall: self.recall.summary(),
                f1: self.f1.summary(),
            },
            perfect_exact_forced_gold_bearing_cases: RateSummary::new(
                self.perfect_exact_forced_gold_bearing_cases,
                self.gold_bearing_cases,
            ),
            singleton_relations: self.singleton.metrics(),
            multi_relations: self.multi.metrics(),
            ambiguity: self.ambiguity.metrics(),
            representation_warning: self.representation.metrics(),
            unscored_predictions: self.unscored.metrics(),
            coverage: self.coverage.metrics(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MacroSummary {
    precision: AverageSummary,
    recall: AverageSummary,
    f1: AverageSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateSummary {
    numerator: usize,
    denominator: usize,
    value: Option<f64>,
}

impl RateSummary {
    fn new(numerator: usize, denominator: usize) -> Self {
        Self {
            numerator,
            denominator,
            value: ratio(numerator, denominator),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregateCategorySummary {
    micro: ExactMetrics,
    ambiguity_covered_gold_relation_rate: RateSummary,
    macro_average: MacroSummary,
    perfect_exact_forced_gold_bearing_cases: RateSummary,
    singleton_relations: LaneMetrics,
    multi_relations: LaneMetrics,
    ambiguity: AmbiguityMetrics,
    representation_warning: RepresentationMetrics,
    unscored_predictions: UnscoredMetrics,
    coverage: CoverageMetrics,
}

#[derive(Default)]
struct AggregateAccumulator {
    program_elements: CategoryAccumulator,
    mappings: CategoryAccumulator,
    pooled_category_observations: CategoryAccumulator,
    before_bridge: BridgeCounts,
    after_bridge: BridgeCounts,
    ignored_input_pair_relations: usize,
    ignored_suggested_relations: usize,
    latencies_micros: Vec<u64>,
    serialized_diff_report_bytes: Vec<usize>,
    verification_work: Vec<usize>,
}

impl AggregateAccumulator {
    fn add(&mut self, outcome: &CaseOutcome) {
        let (
            analysis_latency_micros,
            serialized_diff_report_bytes,
            verification_work,
            program_elements,
            mappings,
            pooled_category_observations,
            prediction_diagnostics,
        ) = match outcome {
            CaseOutcome::Evaluated {
                analysis_latency_micros,
                serialized_diff_report_bytes,
                verification_work,
                program_elements,
                mappings,
                pooled_category_observations,
                prediction_diagnostics,
                ..
            } => (
                analysis_latency_micros,
                serialized_diff_report_bytes,
                verification_work,
                program_elements,
                mappings,
                pooled_category_observations,
                prediction_diagnostics,
            ),
            CaseOutcome::Error {
                analysis_latency_micros: Some(latency),
                ..
            } => {
                self.latencies_micros.push(*latency);
                return;
            }
            CaseOutcome::KnownMalformedOracle { .. }
            | CaseOutcome::KnownMalformedSource { .. }
            | CaseOutcome::Error { .. } => return,
        };
        self.program_elements.add(**program_elements);
        self.mappings.add(**mappings);
        self.pooled_category_observations
            .add(**pooled_category_observations);
        self.before_bridge = self.before_bridge.add(BridgeCounts {
            enumerated_nodes: prediction_diagnostics.before_bridge.enumerated_nodes,
            supported_nodes: prediction_diagnostics.before_bridge.supported_nodes,
            bridged_nodes: prediction_diagnostics.before_bridge.bridged_nodes,
            unsupported_nodes: prediction_diagnostics.before_bridge.unsupported_nodes,
            unresolved_supported_nodes: prediction_diagnostics
                .before_bridge
                .unresolved_supported_nodes,
        });
        self.after_bridge = self.after_bridge.add(BridgeCounts {
            enumerated_nodes: prediction_diagnostics.after_bridge.enumerated_nodes,
            supported_nodes: prediction_diagnostics.after_bridge.supported_nodes,
            bridged_nodes: prediction_diagnostics.after_bridge.bridged_nodes,
            unsupported_nodes: prediction_diagnostics.after_bridge.unsupported_nodes,
            unresolved_supported_nodes: prediction_diagnostics
                .after_bridge
                .unresolved_supported_nodes,
        });
        self.ignored_input_pair_relations += prediction_diagnostics.ignored_input_pair_relations;
        self.ignored_suggested_relations += prediction_diagnostics.ignored_suggested_relations;
        self.latencies_micros.push(*analysis_latency_micros);
        self.serialized_diff_report_bytes
            .push(*serialized_diff_report_bytes);
        self.verification_work.push(*verification_work);
    }

    fn summary(&self) -> AggregateSummary {
        AggregateSummary {
            program_elements: self.program_elements.summary(),
            mappings: self.mappings.summary(),
            pooled_category_observations: self.pooled_category_observations.summary(),
            prediction_diagnostics: AggregatePredictionDiagnostics {
                before_bridge: self.before_bridge.metrics(),
                after_bridge: self.after_bridge.metrics(),
                combined_bridge: self.before_bridge.add(self.after_bridge).metrics(),
                ignored_input_pair_relations: self.ignored_input_pair_relations,
                ignored_suggested_relations: self.ignored_suggested_relations,
            },
            analysis_latency_micros: latency_summary(&self.latencies_micros),
            serialized_diff_report_bytes: size_summary(&self.serialized_diff_report_bytes),
            verification_work: size_summary(&self.verification_work),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregatePredictionDiagnostics {
    before_bridge: BridgeMetrics,
    after_bridge: BridgeMetrics,
    combined_bridge: BridgeMetrics,
    ignored_input_pair_relations: usize,
    ignored_suggested_relations: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatencySummary {
    measured_cases: usize,
    p50: Option<u64>,
    p95: Option<u64>,
    max: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SizeSummary {
    measured_cases: usize,
    p50: Option<usize>,
    p95: Option<usize>,
    max: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregateSummary {
    program_elements: AggregateCategorySummary,
    mappings: AggregateCategorySummary,
    pooled_category_observations: AggregateCategorySummary,
    prediction_diagnostics: AggregatePredictionDiagnostics,
    analysis_latency_micros: LatencySummary,
    serialized_diff_report_bytes: SizeSummary,
    verification_work: SizeSummary,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunCounts {
    manifest_cases: usize,
    selected_cases: usize,
    evaluated_cases: usize,
    verified_reports: usize,
    successful_replays: usize,
    known_malformed_oracle_cases: usize,
    known_malformed_source_cases: usize,
    error_cases: usize,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum EvaluatorProvenance {
    VerifiedCache {
        profile: &'static str,
        protocol: &'static str,
        cache_root: Box<str>,
        gumtree_jdt_generator_version: &'static str,
        gumtree_fat_jar_sha256: &'static str,
        eclipse_jdt_core_version: &'static str,
        eclipse_jdt_core_jar_sha256: &'static str,
        ecj_version: &'static str,
        ecj_jar_sha256: &'static str,
        helper_source_sha256: String,
        java_executable: String,
        java_runtime_version: Box<str>,
        java_trust_boundary: &'static str,
        enumeration_basis: &'static str,
    },
    UnverifiedExecutable {
        executable: String,
        executable_blake3: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceSummary {
    process_vm_hwm_kib: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineProvenance {
    git_revision: &'static str,
    git_dirty: Option<bool>,
    cargo_lock_sha256: &'static str,
    build_profile: &'static str,
    executable_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationReport {
    schema: &'static str,
    engine_version: &'static str,
    engine_provenance: EngineProvenance,
    engine_provenance_complete: bool,
    dataset_revision: &'static str,
    provenance: EvaluatorProvenance,
    materialization_manifest_schema: &'static str,
    materialization_manifest_blake3: String,
    canonical_materialization_manifest: bool,
    checkout_root: String,
    materialization_root: String,
    counts: RunCounts,
    full_corpus_selected: bool,
    benchmark_complete: bool,
    aggregate: AggregateSummary,
    resources: ResourceSummary,
    cases: Vec<CaseReport>,
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let (engine_provenance, engine_provenance_complete) = engine_provenance()?;
    let checkout = canonical_directory(&cli.checkout, "checkout")?;
    let materialization_root =
        canonical_directory(&cli.materialization_root, "materialization root")?;
    let jdt_runtime = resolve_jdt_runtime(&cli)?;
    validate_revision(&checkout)?;
    validate_checkout_clean(&checkout)?;
    let (manifest, manifest_blake3) = read_manifest(&materialization_root)?;
    validate_manifest_header(&manifest)?;
    let selected_cases = cli.limit.unwrap_or(DIFFBENCHMARK_LITERATURE_CASES);
    ensure!(selected_cases != 0, "--limit must be greater than zero");
    ensure!(
        selected_cases <= DIFFBENCHMARK_LITERATURE_CASES,
        "--limit {selected_cases} exceeds the {DIFFBENCHMARK_LITERATURE_CASES}-case corpus"
    );

    let mut prepared = Vec::with_capacity(selected_cases);
    for (index, case) in manifest.cases.iter().take(selected_cases).enumerate() {
        prepared.push(prepare_case(index, case, &checkout, &materialization_root));
    }

    let ready: Vec<_> = prepared
        .iter()
        .filter_map(|case| match case {
            PreparedCase::Ready(case) => Some(case.as_ref()),
            PreparedCase::Finished(_) => None,
        })
        .collect();
    let enumerated = if ready.is_empty() {
        Vec::new()
    } else {
        enumerate_jdt(&jdt_runtime, &ready)?
    };
    let mut enumerated = enumerated.into_iter();
    let mut aggregate = AggregateAccumulator::default();
    let mut cases = Vec::with_capacity(selected_cases);
    for case in prepared {
        let report = match case {
            PreparedCase::Ready(case) => {
                let before_nodes = enumerated
                    .next()
                    .expect("validated enumerator output has one block per before source");
                let after_nodes = enumerated
                    .next()
                    .expect("validated enumerator output has one block per after source");
                evaluate_ready_case(*case, &before_nodes, &after_nodes)
            }
            PreparedCase::Finished(report) => *report,
        };
        aggregate.add(&report.outcome);
        cases.push(report);
    }
    assert!(enumerated.next().is_none());

    let evaluated_cases = cases
        .iter()
        .filter(|case| matches!(case.outcome, CaseOutcome::Evaluated { .. }))
        .count();
    let known_malformed_oracle_cases = cases
        .iter()
        .filter(|case| matches!(case.outcome, CaseOutcome::KnownMalformedOracle { .. }))
        .count();
    let known_malformed_source_cases = cases
        .iter()
        .filter(|case| matches!(case.outcome, CaseOutcome::KnownMalformedSource { .. }))
        .count();
    let error_cases = cases
        .iter()
        .filter(|case| matches!(case.outcome, CaseOutcome::Error { .. }))
        .count();
    let full_corpus_selected = selected_cases == DIFFBENCHMARK_LITERATURE_CASES;
    let jdt_cache_verified = matches!(jdt_runtime, JdtRuntime::Verified(_));
    let canonical_materialization_manifest =
        manifest_blake3 == CANONICAL_MATERIALIZATION_MANIFEST_BLAKE3;
    let counts = RunCounts {
        manifest_cases: manifest.cases.len(),
        selected_cases,
        evaluated_cases,
        verified_reports: evaluated_cases,
        successful_replays: evaluated_cases,
        known_malformed_oracle_cases,
        known_malformed_source_cases,
        error_cases,
    };
    let benchmark_complete = is_benchmark_complete(
        engine_provenance_complete,
        jdt_cache_verified,
        canonical_materialization_manifest,
        full_corpus_selected,
        &counts,
    );
    let report = EvaluationReport {
        schema: EVALUATION_REPORT_SCHEMA,
        engine_version: env!("CARGO_PKG_VERSION"),
        engine_provenance,
        engine_provenance_complete,
        dataset_revision: DIFFBENCHMARK_REVISION,
        provenance: jdt_runtime.provenance(),
        materialization_manifest_schema: MATERIALIZATION_MANIFEST_SCHEMA,
        materialization_manifest_blake3: manifest_blake3,
        canonical_materialization_manifest,
        checkout_root: path_string(&checkout),
        materialization_root: path_string(&materialization_root),
        counts,
        full_corpus_selected,
        benchmark_complete,
        aggregate: aggregate.summary(),
        resources: ResourceSummary {
            process_vm_hwm_kib: process_vm_hwm_kib()?,
        },
        cases,
    };
    write_report(&report, cli.output.as_deref())?;
    Ok(
        if error_cases == 0 && (!cli.require_complete || benchmark_complete) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        },
    )
}

fn engine_provenance() -> Result<(EngineProvenance, bool)> {
    let executable = env::current_exe().context("failed to locate the evaluator executable")?;
    let executable = canonical_file(&executable, "evaluator executable")?;
    let executable_sha256 = sha256_file(&executable)?;
    let git_dirty = match BUILD_GIT_DIRTY {
        "false" => Some(false),
        "true" => Some(true),
        "unavailable" => None,
        _ => None,
    };
    let complete = is_lower_hex(BUILD_GIT_REVISION, 40)
        && git_dirty == Some(false)
        && is_lower_hex(BUILD_CARGO_LOCK_SHA256, 64)
        && BUILD_PROFILE == "release"
        && is_lower_hex(&executable_sha256, 64);
    Ok((
        EngineProvenance {
            git_revision: BUILD_GIT_REVISION,
            git_dirty,
            cargo_lock_sha256: BUILD_CARGO_LOCK_SHA256,
            build_profile: BUILD_PROFILE,
            executable_sha256,
        },
        complete,
    ))
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve {label} {}", path.display()))?;
    ensure!(
        canonical.is_dir(),
        "{label} is not a directory: {}",
        path.display()
    );
    Ok(canonical)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve {label} {}", path.display()))?;
    ensure!(
        canonical.is_file(),
        "{label} is not a file: {}",
        path.display()
    );
    Ok(canonical)
}

fn resolve_jdt_runtime(cli: &Cli) -> Result<JdtRuntime> {
    if let Some(cache) = &cli.jdt_cache {
        let java_executable = cli
            .java_executable
            .as_deref()
            .context("--jdt-cache requires --java-executable")?;
        let java_executable = canonical_file(java_executable, "trusted Java executable")?;
        ensure!(
            !fs::symlink_metadata(cache)
                .with_context(|| format!("failed to inspect JDT cache {}", cache.display()))?
                .file_type()
                .is_symlink(),
            "JDT cache must not be a symbolic link: {}",
            cache.display()
        );
        return Ok(JdtRuntime::Verified(validate_jdt_cache(
            cache,
            &java_executable,
        )?));
    }

    let executable = cli
        .jdt_enumerator
        .as_deref()
        .context("either --jdt-cache or --jdt-enumerator is required")?;
    ensure!(
        cli.allow_unverified_jdt_enumerator,
        "--jdt-enumerator requires --allow-unverified-jdt-enumerator"
    );
    let executable = canonical_file(executable, "JDT enumerator")?;
    let bytes = fs::read(&executable).with_context(|| {
        format!(
            "failed to read unverified JDT enumerator {}",
            executable.display()
        )
    })?;
    Ok(JdtRuntime::Unverified {
        executable,
        executable_blake3: blake3::hash(&bytes).to_hex().to_string(),
    })
}

fn validate_jdt_cache(path: &Path, trusted_java: &Path) -> Result<VerifiedJdtCache> {
    validate_jdt_cache_with_expected(
        path,
        trusted_java,
        &JDT_ARTIFACT_DIGESTS,
        ENUMERATE_JDT_SOURCE,
    )
}

fn validate_jdt_cache_with_expected(
    path: &Path,
    trusted_java: &Path,
    expected_digests: &JdtArtifactDigests<'_>,
    expected_helper_source: &[u8],
) -> Result<VerifiedJdtCache> {
    let cache_root = canonical_directory(path, "JDT cache")?;
    let downloads = verified_cache_directory(&cache_root, "downloads")?;
    let provenance = verified_cache_directory(&cache_root, "provenance")?;

    let gumtree_fat_jar =
        verified_cache_file(&downloads, "gumtree-3.0.0.jar", "cached GumTree fat JAR")?;
    let eclipse_jdt_core_jar = verified_cache_file(
        &downloads,
        "org.eclipse.jdt.core-3.35.0.jar",
        "cached Eclipse JDT Core JAR",
    )?;
    let ecj_jar = verified_cache_file(&downloads, "ecj-3.35.0.jar", "cached ECJ JAR")?;
    verify_sha256(
        &gumtree_fat_jar,
        expected_digests.gumtree_fat_jar,
        "cached GumTree fat JAR",
    )?;
    verify_sha256(
        &eclipse_jdt_core_jar,
        expected_digests.eclipse_jdt_core_jar,
        "cached Eclipse JDT Core JAR",
    )?;
    verify_sha256(&ecj_jar, expected_digests.ecj_jar, "cached ECJ JAR")?;

    let cached_helper_source = verified_cache_file(
        &provenance,
        "EnumerateJdt.java.source",
        "cached JDT helper source",
    )?;
    let helper_bytes = fs::read(&cached_helper_source).with_context(|| {
        format!(
            "failed to read cached JDT helper source {}",
            cached_helper_source.display()
        )
    })?;
    ensure!(
        helper_bytes == expected_helper_source,
        "cached JDT helper source does not match this evaluator"
    );
    let helper_source_sha256 = sha256_bytes(&helper_bytes);

    let java_provenance = verified_cache_file(
        &provenance,
        "java-executable",
        "cached Java executable provenance",
    )?;
    let java_path = read_canonical_line(&java_provenance, "cached Java executable provenance")?;
    let configured_java = PathBuf::from(&java_path);
    ensure!(
        configured_java.is_absolute(),
        "cached Java executable is not absolute: {java_path}"
    );
    let java_executable = canonical_file(&configured_java, "cached Java executable")?;
    ensure!(
        java_executable == configured_java,
        "cached Java executable is not canonical: {java_path}"
    );
    ensure!(
        java_executable == trusted_java,
        "cached Java executable {} does not match trusted Java executable {}",
        java_executable.display(),
        trusted_java.display()
    );
    let java_runtime_version = inspect_java_runtime(trusted_java)?;

    let mut runtime_directory_builder = tempfile::Builder::new();
    runtime_directory_builder.prefix("stratadiff-jdt-runtime-");
    #[cfg(unix)]
    runtime_directory_builder.permissions(fs::Permissions::from_mode(0o700));
    let runtime_directory = runtime_directory_builder
        .tempdir()
        .context("failed to create private JDT runtime directory")?;
    let gumtree_fat_jar = copy_verified_artifact(
        &gumtree_fat_jar,
        runtime_directory.path(),
        "gumtree-3.0.0.jar",
        expected_digests.gumtree_fat_jar,
        "snapshotted GumTree fat JAR",
    )?;
    let eclipse_jdt_core_jar = copy_verified_artifact(
        &eclipse_jdt_core_jar,
        runtime_directory.path(),
        "org.eclipse.jdt.core-3.35.0.jar",
        expected_digests.eclipse_jdt_core_jar,
        "snapshotted Eclipse JDT Core JAR",
    )?;
    let ecj_jar = copy_verified_artifact(
        &ecj_jar,
        runtime_directory.path(),
        "ecj-3.35.0.jar",
        expected_digests.ecj_jar,
        "snapshotted ECJ JAR",
    )?;
    let helper_source = copy_verified_artifact(
        &cached_helper_source,
        runtime_directory.path(),
        "EnumerateJdt.java",
        &helper_source_sha256,
        "snapshotted JDT helper source",
    )?;
    let snapshotted_helper =
        fs::read(&helper_source).context("failed to read snapshotted JDT helper source")?;
    ensure!(
        snapshotted_helper == expected_helper_source,
        "snapshotted JDT helper source does not match this evaluator"
    );

    Ok(VerifiedJdtCache {
        _runtime_directory: runtime_directory,
        cache_root,
        java_executable: trusted_java.to_owned(),
        java_runtime_version,
        gumtree_fat_jar,
        eclipse_jdt_core_jar,
        ecj_jar,
        helper_source,
        helper_source_sha256,
    })
}

fn copy_verified_artifact(
    source: &Path,
    destination_directory: &Path,
    destination_name: &str,
    expected_sha256: &str,
    label: &str,
) -> Result<PathBuf> {
    let destination = destination_directory.join(destination_name);
    let mut source_file = fs::File::open(source)
        .with_context(|| format!("failed to open {} for snapshotting", source.display()))?;
    let mut destination_options = fs::OpenOptions::new();
    destination_options.write(true).create_new(true);
    #[cfg(unix)]
    destination_options.mode(0o600);
    let mut destination_file = destination_options.open(&destination).with_context(|| {
        format!(
            "failed to create private JDT artifact {}",
            destination.display()
        )
    })?;
    io::copy(&mut source_file, &mut destination_file).with_context(|| {
        format!(
            "failed to copy {} into private JDT runtime",
            source.display()
        )
    })?;
    destination_file.flush().with_context(|| {
        format!(
            "failed to flush private JDT artifact {}",
            destination.display()
        )
    })?;
    drop(destination_file);
    verify_sha256(&destination, expected_sha256, label)?;
    Ok(destination)
}

fn verified_cache_directory(cache_root: &Path, name: &str) -> Result<PathBuf> {
    let path = cache_root.join(name);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect JDT cache directory {}", path.display()))?;
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "JDT cache path is not a real directory: {}",
        path.display()
    );
    Ok(path)
}

fn verified_cache_file(directory: &Path, name: &str, label: &str) -> Result<PathBuf> {
    let path = directory.join(name);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "{label} is not a real regular file: {}",
        path.display()
    );
    Ok(path)
}

fn verify_sha256(path: &Path, expected: &str, label: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    ensure!(
        actual == expected,
        "{label} SHA-256 mismatch: expected {expected}, found {actual}"
    );
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open {} for SHA-256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_canonical_line(path: &Path, label: &str) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {label}"))?;
    let text = std::str::from_utf8(&bytes).with_context(|| format!("{label} is not UTF-8"))?;
    let value = text
        .strip_suffix('\n')
        .with_context(|| format!("{label} does not end with one LF"))?;
    ensure!(
        !value.is_empty() && !value.contains(['\r', '\n']),
        "{label} is not one canonical line"
    );
    Ok(value.to_owned())
}

struct BoundedBytes {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn read_bounded<R: Read>(
    mut reader: R,
    limit: usize,
    overflow: &AtomicU8,
    overflow_bit: u8,
) -> io::Result<BoundedBytes> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(BoundedBytes {
                bytes,
                exceeded: false,
            });
        }
        let retained = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        if retained != count {
            overflow.fetch_or(overflow_bit, Ordering::Release);
            return Ok(BoundedBytes {
                bytes,
                exceeded: true,
            });
        }
    }
}

fn terminate_child(child: &mut Child, label: &str) -> Result<std::process::ExitStatus> {
    match child.kill() {
        Ok(()) => child
            .wait()
            .with_context(|| format!("failed to reap {label} after killing it")),
        Err(kill_error) => match child
            .try_wait()
            .with_context(|| format!("failed to inspect {label} after kill failed"))?
        {
            Some(status) => Ok(status),
            None => Err(kill_error).with_context(|| format!("failed to kill {label}")),
        },
    }
}

fn join_bounded_reader(
    reader: thread::JoinHandle<io::Result<BoundedBytes>>,
    label: &str,
) -> Result<BoundedBytes> {
    reader
        .join()
        .map_err(|_| anyhow!("{label} reader thread panicked"))?
        .with_context(|| format!("failed to read {label}"))
}

fn run_bounded_command(
    command: &mut Command,
    label: &str,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    let stdout = child.stdout.take().context("child stdout was not piped")?;
    let stderr = child.stderr.take().context("child stderr was not piped")?;
    let overflow = Arc::new(AtomicU8::new(0));
    let stdout_overflow = Arc::clone(&overflow);
    let stderr_overflow = Arc::clone(&overflow);
    let stdout_reader =
        thread::spawn(move || read_bounded(stdout, stdout_limit, &stdout_overflow, 1));
    let stderr_reader =
        thread::spawn(move || read_bounded(stderr, stderr_limit, &stderr_overflow, 2));

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if overflow.load(Ordering::Acquire) != 0 {
            break terminate_child(&mut child, label)?;
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break terminate_child(&mut child, label)?;
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_bounded_reader(stdout_reader, &format!("{label} stdout"))?;
    let stderr = join_bounded_reader(stderr_reader, &format!("{label} stderr"))?;
    ensure!(
        !timed_out,
        "{label} timed out after {} milliseconds",
        timeout.as_millis()
    );
    ensure!(
        !stdout.exceeded,
        "{label} stdout exceeded {stdout_limit} bytes"
    );
    ensure!(
        !stderr.exceeded,
        "{label} stderr exceeded {stderr_limit} bytes"
    );
    Ok(Output {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

fn inspect_java_runtime(executable: &Path) -> Result<String> {
    let mut command = Command::new(executable);
    clear_java_environment(&mut command);
    command.args([JAVA_VERSION_MAX_HEAP, "-version"]);
    let label = format!("trusted Java executable {} -version", executable.display());
    let output = run_bounded_command(
        &mut command,
        &label,
        JAVA_VERSION_TIMEOUT,
        JAVA_VERSION_OUTPUT_LIMIT,
        JAVA_VERSION_OUTPUT_LIMIT,
    )?;
    ensure!(
        output.status.success(),
        "trusted Java executable {} failed -version with {}",
        executable.display(),
        output.status
    );
    let stdout =
        std::str::from_utf8(&output.stdout).context("java -version stdout is not UTF-8")?;
    let stderr =
        std::str::from_utf8(&output.stderr).context("java -version stderr is not UTF-8")?;
    let version = match (stdout.trim_end(), stderr.trim_end()) {
        ("", "") => bail!("trusted Java executable produced no version"),
        (stdout, "") => stdout.to_owned(),
        ("", stderr) => stderr.to_owned(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    };
    let first_line = version.lines().next().context("java -version is empty")?;
    let quoted = first_line
        .split_once('"')
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value)
        .with_context(|| format!("unrecognized java -version output: {first_line}"))?;
    let mut components = quoted.split('.');
    let first = components
        .next()
        .context("java version has no major component")?
        .parse::<u32>()
        .context("java version major is not an integer")?;
    let major = if first == 1 {
        components
            .next()
            .context("legacy java version has no second component")?
            .parse::<u32>()
            .context("legacy java version major is not an integer")?
    } else {
        first
    };
    ensure!(
        major >= 17,
        "Java 17 or newer is required, found: {first_line}"
    );
    Ok(version)
}

fn clear_java_environment(command: &mut Command) {
    for name in [
        "CLASSPATH",
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "_JAVA_OPTIONS",
    ] {
        command.env_remove(name);
    }
}

impl JdtRuntime {
    fn provenance(&self) -> EvaluatorProvenance {
        match self {
            Self::Verified(cache) => EvaluatorProvenance::VerifiedCache {
                profile: JDT_PROFILE,
                protocol: JDT_PROTOCOL,
                cache_root: path_string(&cache.cache_root).into_boxed_str(),
                gumtree_jdt_generator_version: GUMTREE_JDT_GENERATOR_VERSION,
                gumtree_fat_jar_sha256: GUMTREE_FAT_JAR_SHA256,
                eclipse_jdt_core_version: ECLIPSE_JDT_CORE_VERSION,
                eclipse_jdt_core_jar_sha256: ECLIPSE_JDT_CORE_JAR_SHA256,
                ecj_version: ECJ_VERSION,
                ecj_jar_sha256: ECJ_JAR_SHA256,
                helper_source_sha256: cache.helper_source_sha256.clone(),
                java_executable: path_string(&cache.java_executable),
                java_runtime_version: cache.java_runtime_version.clone().into_boxed_str(),
                java_trust_boundary: JAVA_TRUST_BOUNDARY,
                enumeration_basis: JDT_ENUMERATION_BASIS,
            },
            Self::Unverified {
                executable,
                executable_blake3,
            } => EvaluatorProvenance::UnverifiedExecutable {
                executable: path_string(executable),
                executable_blake3: executable_blake3.clone(),
            },
        }
    }
}

fn validate_revision(checkout: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .output()
        .with_context(|| format!("failed to run git in {}", checkout.display()))?;
    if !output.status.success() {
        bail!(
            "git rev-parse failed for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let revision = std::str::from_utf8(&output.stdout)
        .context("git revision is not UTF-8")?
        .trim_end_matches(['\r', '\n']);
    ensure!(
        revision == DIFFBENCHMARK_REVISION,
        "DiffBenchmark revision mismatch: expected {DIFFBENCHMARK_REVISION}, found {revision}"
    );
    Ok(())
}

fn validate_checkout_clean(checkout: &Path) -> Result<()> {
    validate_git_diff(checkout, false)?;
    validate_git_diff(checkout, true)
}

fn validate_git_diff(checkout: &Path, cached: bool) -> Result<()> {
    let mut command = Command::new("git");
    command.arg("-C").arg(checkout).arg("diff");
    if cached {
        command.arg("--cached");
    }
    let output = command
        .arg("--quiet")
        .arg("--")
        .args([ORACLE_ROOT, INFO_CSV, LITERATURE_CSV])
        .output()
        .with_context(|| {
            format!(
                "failed to inspect checkout cleanliness in {}",
                checkout.display()
            )
        })?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => bail!(
            "DiffBenchmark checkout has {} changes in benchmark inputs",
            if cached { "staged" } else { "working tree" }
        ),
        _ => bail!(
            "git diff failed for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        ),
    }
}

fn read_manifest(root: &Path) -> Result<(MaterializationManifest, String)> {
    let path = root.join(MANIFEST_NAME);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect manifest {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "materialization manifest is not a regular file: {}",
        path.display()
    );
    let bytes =
        fs::read(&path).with_context(|| format!("failed to read manifest {}", path.display()))?;
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let manifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid materialization manifest {}", path.display()))?;
    Ok((manifest, digest))
}

fn validate_manifest_header(manifest: &MaterializationManifest) -> Result<()> {
    ensure!(
        manifest.schema == MATERIALIZATION_MANIFEST_SCHEMA,
        "materialization manifest schema mismatch: expected {MATERIALIZATION_MANIFEST_SCHEMA}, found {}",
        manifest.schema
    );
    ensure!(
        manifest.dataset_revision == DIFFBENCHMARK_REVISION,
        "materialization manifest revision mismatch: expected {DIFFBENCHMARK_REVISION}, found {}",
        manifest.dataset_revision
    );
    ensure!(
        manifest.case_count == DIFFBENCHMARK_LITERATURE_CASES,
        "materialization manifest caseCount mismatch: expected {DIFFBENCHMARK_LITERATURE_CASES}, found {}",
        manifest.case_count
    );
    ensure!(
        manifest.cases.len() == DIFFBENCHMARK_LITERATURE_CASES,
        "materialization manifest cases length mismatch: expected {DIFFBENCHMARK_LITERATURE_CASES}, found {}",
        manifest.cases.len()
    );
    let mut oracle_paths = BTreeSet::new();
    let mut materialized_paths = BTreeSet::new();
    for (index, case) in manifest.cases.iter().enumerate() {
        validate_sha(&case.commit).with_context(|| format!("invalid case {index} commit"))?;
        validate_sha(&case.parent).with_context(|| format!("invalid case {index} parent"))?;
        validate_digest(&case.oracle_blake3)
            .with_context(|| format!("invalid case {index} oracle digest"))?;
        validate_digest(&case.before.content_blake3)
            .with_context(|| format!("invalid case {index} before source digest"))?;
        validate_digest(&case.after.content_blake3)
            .with_context(|| format!("invalid case {index} after source digest"))?;
        validate_relative_path(&case.oracle_path, "oracle path")?;
        validate_relative_path(&case.before.repository_path, "before repository path")?;
        validate_relative_path(&case.after.repository_path, "after repository path")?;
        validate_relative_path(&case.before.materialized_path, "before materialized path")?;
        validate_relative_path(&case.after.materialized_path, "after materialized path")?;
        ensure!(
            case.oracle_path
                .starts_with("hrd-oracle/adb-paper/literature-exp/")
                && case.oracle_path.ends_with("/GOD.json"),
            "case {index} has an unexpected oracle path: {}",
            case.oracle_path
        );
        let expected_before = format!("sources/{index:04}/before.source");
        let expected_after = format!("sources/{index:04}/after.source");
        ensure!(
            case.before.materialized_path == expected_before
                && case.after.materialized_path == expected_after,
            "case {index} materialized paths do not match the canonical layout"
        );
        ensure!(
            oracle_paths.insert(case.oracle_path.as_str()),
            "duplicate oracle path in case {index}: {}",
            case.oracle_path
        );
        ensure!(
            materialized_paths.insert(case.before.materialized_path.as_str())
                && materialized_paths.insert(case.after.materialized_path.as_str()),
            "duplicate materialized path in case {index}"
        );
    }
    Ok(())
}

fn prepare_case(
    index: usize,
    case: &MaterializedCase,
    checkout: &Path,
    materialization_root: &Path,
) -> PreparedCase {
    let identity = CaseIdentity {
        index,
        oracle_path: case.oracle_path.clone(),
        commit: case.commit.clone(),
        before_repository_path: case.before.repository_path.clone(),
        after_repository_path: case.after.repository_path.clone(),
        before_materialized_path: case.before.materialized_path.clone(),
        after_materialized_path: case.after.materialized_path.clone(),
    };
    match prepare_case_result(case, identity.clone(), checkout, materialization_root) {
        Ok(PreparedCase::Ready(case)) => PreparedCase::Ready(case),
        Ok(PreparedCase::Finished(report)) => PreparedCase::Finished(report),
        Err(error) => PreparedCase::Finished(Box::new(case_error(
            identity,
            CaseStage::InputValidation,
            error,
            None,
        ))),
    }
}

fn prepare_case_result(
    case: &MaterializedCase,
    identity: CaseIdentity,
    checkout: &Path,
    materialization_root: &Path,
) -> Result<PreparedCase> {
    let oracle = read_verified_file(checkout, &case.oracle_path, &case.oracle_blake3, "oracle")?;
    let before = read_verified_file(
        materialization_root,
        &case.before.materialized_path,
        &case.before.content_blake3,
        "before source",
    )?;
    let after = read_verified_file(
        materialization_root,
        &case.after.materialized_path,
        &case.after.content_blake3,
        "after source",
    )?;
    let oracle = match parse_god_report(&oracle.bytes) {
        Ok(god) => god,
        Err(error) => {
            return Ok(oracle_parse_failure(
                identity,
                &case.oracle_blake3,
                format!("{error:#}"),
            ));
        }
    };
    if is_known_malformed_source(case) {
        let error = match comparable_tree_sitter_java_nodes(&after.bytes) {
            Ok(_) => bail!(
                "pinned malformed source unexpectedly parsed successfully: {}",
                case.after.repository_path
            ),
            Err(error) => format!("{error:#}"),
        };
        return Ok(PreparedCase::Finished(Box::new(CaseReport::new(
            identity,
            CaseOutcome::KnownMalformedSource {
                side: EndpointSide::After,
                content_blake3: KNOWN_MALFORMED_SOURCE_AFTER_BLAKE3,
                error,
            },
        ))));
    }
    Ok(PreparedCase::Ready(Box::new(ReadyCase {
        identity,
        oracle,
        before_path: before.path,
        after_path: after.path,
        before_source: before.bytes,
        after_source: after.bytes,
    })))
}

fn is_known_malformed_source(case: &MaterializedCase) -> bool {
    case.oracle_path == KNOWN_MALFORMED_SOURCE_ORACLE_PATH
        && case.oracle_blake3 == KNOWN_MALFORMED_SOURCE_ORACLE_BLAKE3
        && case.commit == KNOWN_MALFORMED_SOURCE_COMMIT
        && case.parent == KNOWN_MALFORMED_SOURCE_PARENT
        && case.before.repository_path == KNOWN_MALFORMED_SOURCE_PATH
        && case.after.repository_path == KNOWN_MALFORMED_SOURCE_PATH
        && case.before.content_blake3 == KNOWN_MALFORMED_SOURCE_BEFORE_BLAKE3
        && case.after.content_blake3 == KNOWN_MALFORMED_SOURCE_AFTER_BLAKE3
}

fn oracle_parse_failure(
    identity: CaseIdentity,
    oracle_blake3: &str,
    error: String,
) -> PreparedCase {
    let outcome = if identity.oracle_path == KNOWN_MALFORMED_ORACLE_PATH
        && oracle_blake3 == KNOWN_MALFORMED_ORACLE_BLAKE3
    {
        CaseOutcome::KnownMalformedOracle { error }
    } else {
        CaseOutcome::Error {
            stage: CaseStage::OracleParse,
            error,
            analysis_latency_micros: None,
        }
    };
    PreparedCase::Finished(Box::new(CaseReport::new(identity, outcome)))
}

struct VerifiedFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

fn read_verified_file(
    root: &Path,
    relative: &str,
    expected: &str,
    label: &str,
) -> Result<VerifiedFile> {
    validate_relative_path(relative, label)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "{label} is not a regular file: {}",
        path.display()
    );
    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve {label} {}", path.display()))?;
    ensure!(
        canonical.starts_with(root),
        "{label} escapes its root: {}",
        path.display()
    );
    let bytes = fs::read(&canonical)
        .with_context(|| format!("failed to read {label} {}", canonical.display()))?;
    let actual = blake3::hash(&bytes).to_hex().to_string();
    ensure!(
        actual == expected,
        "{label} BLAKE3 mismatch for {}: expected {expected}, found {actual}",
        canonical.display()
    );
    Ok(VerifiedFile {
        path: canonical,
        bytes,
    })
}

fn validate_relative_path(value: &str, label: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{label} is empty");
    ensure!(
        !value.contains('\\'),
        "{label} must use forward slashes: {value}"
    );
    ensure!(
        !value.split('/').any(str::is_empty),
        "{label} contains an empty component: {value}"
    );
    for component in Path::new(value).components() {
        ensure!(
            matches!(component, Component::Normal(_)),
            "{label} is not a safe relative path: {value}"
        );
    }
    Ok(())
}

fn validate_sha(value: &str) -> Result<()> {
    ensure!(
        is_lower_hex(value, 40),
        "expected a lowercase 40-character hexadecimal SHA, found {value}"
    );
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    ensure!(
        is_lower_hex(value, 64),
        "expected a lowercase 64-character BLAKE3 digest, found {value}"
    );
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn enumerate_jdt(
    runtime: &JdtRuntime,
    cases: &[&ReadyCase],
) -> Result<Vec<Vec<EnumeratedJdtNode>>> {
    let mut paths = Vec::with_capacity(cases.len() * 2);
    let mut sources = Vec::with_capacity(cases.len() * 2);
    for case in cases {
        paths.push(case.before_path.as_path());
        paths.push(case.after_path.as_path());
        sources.push(case.before_source.as_slice());
        sources.push(case.after_source.as_slice());
    }
    let output = run_jdt_enumerator(runtime, &paths)?;
    let blocks = validate_jdt_process_output(output, &sources)?;
    for (index, case) in cases.iter().enumerate() {
        validate_enumerated_nodes(&blocks[index * 2], &case.before_source, &case.before_path)?;
        validate_enumerated_nodes(&blocks[index * 2 + 1], &case.after_source, &case.after_path)?;
    }
    Ok(blocks)
}

fn validate_jdt_process_output(
    output: Output,
    expected_sources: &[&[u8]],
) -> Result<Vec<Vec<EnumeratedJdtNode>>> {
    if !output.status.success() {
        bail!(
            "JDT enumerator failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    ensure!(
        output.stderr.is_empty(),
        "JDT enumerator emitted stderr despite succeeding: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    parse_enumerator_output(&output.stdout, expected_sources)
}

fn run_jdt_enumerator(runtime: &JdtRuntime, paths: &[&Path]) -> Result<Output> {
    match runtime {
        JdtRuntime::Verified(cache) => {
            let classpath = env::join_paths([
                cache.eclipse_jdt_core_jar.as_path(),
                cache.ecj_jar.as_path(),
                cache.gumtree_fat_jar.as_path(),
            ])
            .context("JDT classpath contains an unsupported path")?;
            let mut command = Command::new(&cache.java_executable);
            clear_java_environment(&mut command);
            command
                .arg(JDT_ENUMERATOR_MAX_HEAP)
                .arg("--source")
                .arg("17")
                .arg("--class-path")
                .arg(classpath)
                .arg(&cache.helper_source)
                .args(paths);
            let label = format!(
                "verified JDT helper with {}",
                cache.java_executable.display()
            );
            run_bounded_command(
                &mut command,
                &label,
                JDT_ENUMERATOR_TIMEOUT,
                JDT_ENUMERATOR_STDOUT_LIMIT,
                JDT_ENUMERATOR_STDERR_LIMIT,
            )
        }
        JdtRuntime::Unverified { executable, .. } => {
            let mut command = Command::new(executable);
            command.args(paths);
            let label = format!("unverified JDT enumerator {}", executable.display());
            run_bounded_command(
                &mut command,
                &label,
                JDT_ENUMERATOR_TIMEOUT,
                JDT_ENUMERATOR_STDOUT_LIMIT,
                JDT_ENUMERATOR_STDERR_LIMIT,
            )
        }
    }
}

fn parse_enumerator_output(
    bytes: &[u8],
    expected_sources: &[&[u8]],
) -> Result<Vec<Vec<EnumeratedJdtNode>>> {
    parse_enumerator_output_with_node_limit(bytes, expected_sources, JDT_ENUMERATOR_NODE_LIMIT)
}

fn parse_enumerator_output_with_node_limit(
    bytes: &[u8],
    expected_sources: &[&[u8]],
    node_limit: usize,
) -> Result<Vec<Vec<EnumeratedJdtNode>>> {
    let text = std::str::from_utf8(bytes).context("JDT enumerator stdout is not UTF-8")?;
    ensure!(
        text.ends_with('\n'),
        "JDT enumerator stdout does not end with a newline"
    );
    ensure!(
        !text.contains('\r'),
        "JDT enumerator stdout contains a non-canonical carriage return"
    );
    let expected_blocks = expected_sources.len();
    let mut blocks = Vec::with_capacity(expected_blocks);
    let mut current: Option<(usize, Vec<EnumeratedJdtNode>)> = None;
    let mut saw_hello = false;
    let mut saw_done = false;
    let mut total_nodes = 0_usize;
    for (line_index, raw_line) in text.split_terminator('\n').enumerate() {
        let line = raw_line;
        ensure!(
            !line.is_empty(),
            "empty JDT enumerator TSV line {}",
            line_index + 1
        );
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["HELLO", profile, protocol, count] => {
                ensure!(line_index == 0 && !saw_hello, "unexpected HELLO line");
                ensure!(
                    *profile == JDT_PROFILE,
                    "unexpected JDT profile {profile:?}"
                );
                ensure!(
                    *protocol == JDT_PROTOCOL,
                    "unexpected JDT protocol {protocol:?}"
                );
                let count = parse_decimal(count, "HELLO argument count", line_index + 1)?;
                ensure!(
                    count == expected_blocks,
                    "JDT enumerator declared {count} arguments for {expected_blocks} inputs"
                );
                saw_hello = true;
            }
            ["BEGIN", index, source_sha256] => {
                ensure!(
                    saw_hello,
                    "BEGIN precedes HELLO at TSV line {}",
                    line_index + 1
                );
                ensure!(
                    !saw_done,
                    "BEGIN follows DONE at TSV line {}",
                    line_index + 1
                );
                ensure!(
                    current.is_none(),
                    "nested BEGIN at TSV line {}",
                    line_index + 1
                );
                let index = parse_decimal(index, "BEGIN index", line_index + 1)?;
                ensure!(
                    index == blocks.len() && index < expected_blocks,
                    "unexpected BEGIN index {index} at TSV line {}",
                    line_index + 1
                );
                let expected_sha256 = sha256_bytes(expected_sources[index]);
                ensure!(
                    *source_sha256 == expected_sha256,
                    "JDT enumerator source SHA-256 mismatch for argument {index}: expected {expected_sha256}, found {source_sha256}"
                );
                current = Some((index, Vec::new()));
            }
            ["NODE", kind, start, end] => {
                ensure!(
                    saw_hello && !saw_done,
                    "unexpected NODE at TSV line {}",
                    line_index + 1
                );
                ensure!(
                    !kind.is_empty() && kind.trim() == *kind,
                    "invalid JDT kind at TSV line {}",
                    line_index + 1
                );
                let start = parse_decimal(start, "NODE start", line_index + 1)?;
                let end = parse_decimal(end, "NODE end", line_index + 1)?;
                ensure!(
                    start <= end,
                    "reversed NODE range at TSV line {}",
                    line_index + 1
                );
                let (_, nodes) = current.as_mut().with_context(|| {
                    format!("NODE outside a block at TSV line {}", line_index + 1)
                })?;
                let observed_nodes = total_nodes
                    .checked_add(nodes.len())
                    .and_then(|count| count.checked_add(1))
                    .context("JDT enumerator node count overflows usize")?;
                ensure!(
                    observed_nodes <= node_limit,
                    "JDT enumerator exceeded the {node_limit}-node limit"
                );
                nodes.push(EnumeratedJdtNode {
                    node_type: (*kind).to_owned(),
                    utf16_code_units: OffsetRange { start, end },
                });
            }
            ["END", index, node_count] => {
                ensure!(
                    saw_hello && !saw_done,
                    "unexpected END at TSV line {}",
                    line_index + 1
                );
                let index = parse_decimal(index, "END index", line_index + 1)?;
                let (begin_index, nodes) = current.take().with_context(|| {
                    format!("END outside a block at TSV line {}", line_index + 1)
                })?;
                ensure!(
                    index == begin_index,
                    "END index {index} does not match BEGIN index {begin_index} at TSV line {}",
                    line_index + 1
                );
                let node_count = parse_decimal(node_count, "END node count", line_index + 1)?;
                ensure!(
                    node_count == nodes.len(),
                    "END node count {node_count} does not match {} NODE lines for argument {index}",
                    nodes.len()
                );
                total_nodes = total_nodes
                    .checked_add(node_count)
                    .context("JDT enumerator total node count overflows usize")?;
                blocks.push(nodes);
            }
            ["DONE", block_count, declared_total_nodes] => {
                ensure!(
                    saw_hello && !saw_done,
                    "unexpected DONE at TSV line {}",
                    line_index + 1
                );
                ensure!(
                    current.is_none(),
                    "DONE occurs inside a block at TSV line {}",
                    line_index + 1
                );
                let block_count = parse_decimal(block_count, "DONE block count", line_index + 1)?;
                let declared_total_nodes = parse_decimal(
                    declared_total_nodes,
                    "DONE total node count",
                    line_index + 1,
                )?;
                ensure!(
                    block_count == expected_blocks && block_count == blocks.len(),
                    "DONE block count {block_count} does not match {expected_blocks} inputs and {} completed blocks",
                    blocks.len()
                );
                ensure!(
                    declared_total_nodes == total_nodes,
                    "DONE total node count {declared_total_nodes} does not match {total_nodes} NODE lines"
                );
                saw_done = true;
            }
            _ => bail!(
                "invalid JDT enumerator TSV line {}: {line:?}",
                line_index + 1
            ),
        }
    }
    ensure!(saw_hello, "missing JDT enumerator HELLO line");
    ensure!(current.is_none(), "unterminated JDT enumerator block");
    ensure!(saw_done, "missing JDT enumerator DONE line");
    ensure!(
        blocks.len() == expected_blocks,
        "JDT enumerator emitted {} blocks for {expected_blocks} arguments",
        blocks.len()
    );
    Ok(blocks)
}

fn validate_enumerated_nodes(
    nodes: &[EnumeratedJdtNode],
    source: &[u8],
    path: &Path,
) -> Result<()> {
    let source = std::str::from_utf8(source)
        .with_context(|| format!("enumerated Java source is not UTF-8: {}", path.display()))?;
    let mut boundaries = BTreeSet::new();
    let mut offset = 0;
    boundaries.insert(offset);
    for character in source.chars() {
        offset += character.len_utf16();
        boundaries.insert(offset);
    }
    let mut unique = BTreeSet::new();
    for node in nodes {
        ensure!(
            boundaries.contains(&node.utf16_code_units.start)
                && boundaries.contains(&node.utf16_code_units.end),
            "JDT enumerator emitted an invalid UTF-16 boundary for {}: {}[{}-{}]",
            path.display(),
            node.node_type,
            node.utf16_code_units.start,
            node.utf16_code_units.end
        );
        ensure!(
            unique.insert((
                node.node_type.as_str(),
                node.utf16_code_units.start,
                node.utf16_code_units.end,
            )),
            "JDT enumerator emitted a duplicate node for {}: {}[{}-{}]",
            path.display(),
            node.node_type,
            node.utf16_code_units.start,
            node.utf16_code_units.end
        );
    }
    Ok(())
}

fn parse_decimal(value: &str, label: &str, line: usize) -> Result<usize> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid {label} at TSV line {line}: {value:?}"
    );
    let parsed = value
        .parse::<usize>()
        .with_context(|| format!("{label} overflows usize at TSV line {line}"))?;
    ensure!(
        value == parsed.to_string(),
        "non-canonical {label} at TSV line {line}: {value:?}"
    );
    Ok(parsed)
}

fn evaluate_ready_case(
    case: ReadyCase,
    before_jdt_nodes: &[EnumeratedJdtNode],
    after_jdt_nodes: &[EnumeratedJdtNode],
) -> CaseReport {
    let combined_input_bytes = case.before_source.len() + case.after_source.len();
    let oracle = match adapt_intra_file_case(
        &case.identity.before_repository_path,
        &case.identity.after_repository_path,
        &case.before_source,
        &case.after_source,
        &case.oracle,
    ) {
        Ok(oracle) => oracle,
        Err(error) => {
            return case_error(case.identity, CaseStage::OracleAdaptation, error, None);
        }
    };

    let analysis_before = case.before_source.clone();
    let analysis_after = case.after_source.clone();
    let started = Instant::now();
    let report = analyze_bytes(
        analysis_before,
        analysis_after,
        case.identity.before_repository_path.clone(),
        case.identity.after_repository_path.clone(),
        Language::Java,
    );
    let latency = duration_micros(started.elapsed());
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            return case_error(case.identity, CaseStage::Analysis, error, Some(latency));
        }
    };
    let serialized_diff_report_bytes = match serde_json::to_vec(&report) {
        Ok(bytes) => bytes.len(),
        Err(error) => {
            return case_error(
                case.identity,
                CaseStage::ReportSerialization,
                error,
                Some(latency),
            );
        }
    };
    let verification = match verify_report_with_limits(
        &report,
        &case.before_source,
        &case.after_source,
        &VerificationLimits::default(),
    ) {
        Ok(stats) => stats,
        Err(error) => {
            return case_error(case.identity, CaseStage::Verification, error, Some(latency));
        }
    };

    let predictions = match adapt_predictions(&PredictionAdapterInput {
        before_file: &case.identity.before_repository_path,
        after_file: &case.identity.after_repository_path,
        before_source: &case.before_source,
        after_source: &case.after_source,
        before_jdt_nodes,
        after_jdt_nodes,
        oracle: &oracle,
        report: &report,
    }) {
        Ok(predictions) => predictions,
        Err(error) => {
            return case_error(
                case.identity,
                CaseStage::PredictionAdaptation,
                error,
                Some(latency),
            );
        }
    };
    let coverage = oracle.coverage.clone();
    let diagnostics = predictions.diagnostics;
    let evaluation = match evaluate_case(&CaseEvaluationInput {
        universe: predictions.universe,
        oracle: oracle.oracle_relations,
        prediction: predictions.predictions,
    }) {
        Ok(evaluation) => evaluation,
        Err(error) => {
            return case_error(case.identity, CaseStage::Evaluation, error, Some(latency));
        }
    };
    evaluated_case(
        case.identity,
        CaseMeasurements {
            analysis_latency_micros: latency,
            combined_input_bytes,
            serialized_diff_report_bytes,
            verification_work: verification.verification_work,
        },
        evaluation,
        coverage,
        diagnostics,
    )
}

fn evaluated_case(
    identity: CaseIdentity,
    measurements: CaseMeasurements,
    evaluation: stratadiff::diffbenchmark_eval::CaseEvaluation,
    coverage: CoverageLedger,
    diagnostics: PredictionAdapterDiagnostics,
) -> CaseReport {
    let program_elements =
        CaseCategorySummary::new(evaluation.program_elements, &coverage.program_elements);
    let mappings = CaseCategorySummary::new(evaluation.mappings, &coverage.mappings);
    CaseReport::new(
        identity,
        CaseOutcome::Evaluated {
            analysis_latency_micros: measurements.analysis_latency_micros,
            combined_input_bytes: measurements.combined_input_bytes,
            serialized_diff_report_bytes: measurements.serialized_diff_report_bytes,
            verification_work: measurements.verification_work,
            program_elements: Box::new(program_elements),
            mappings: Box::new(mappings),
            pooled_category_observations: Box::new(program_elements.add(mappings)),
            prediction_diagnostics: Box::new(PredictionDiagnosticsSummary::from_diagnostics(
                diagnostics,
            )),
        },
    )
}

fn case_error(
    identity: CaseIdentity,
    stage: CaseStage,
    error: impl std::fmt::Display,
    latency: Option<u64>,
) -> CaseReport {
    CaseReport::new(
        identity,
        CaseOutcome::Error {
            stage,
            error: error.to_string(),
            analysis_latency_micros: latency,
        },
    )
}

impl CaseReport {
    fn new(identity: CaseIdentity, outcome: CaseOutcome) -> Self {
        Self {
            index: identity.index,
            oracle_path: identity.oracle_path,
            commit: identity.commit,
            before_repository_path: identity.before_repository_path,
            after_repository_path: identity.after_repository_path,
            before_materialized_path: identity.before_materialized_path,
            after_materialized_path: identity.after_materialized_path,
            outcome,
        }
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn is_benchmark_complete(
    engine_provenance_complete: bool,
    jdt_cache_verified: bool,
    canonical_materialization_manifest: bool,
    full_corpus_selected: bool,
    counts: &RunCounts,
) -> bool {
    engine_provenance_complete
        && jdt_cache_verified
        && canonical_materialization_manifest
        && full_corpus_selected
        && counts.manifest_cases == DIFFBENCHMARK_LITERATURE_CASES
        && counts.selected_cases == DIFFBENCHMARK_LITERATURE_CASES
        && counts.evaluated_cases == DIFFBENCHMARK_LITERATURE_CASES - 2
        && counts.verified_reports == counts.evaluated_cases
        && counts.successful_replays == counts.evaluated_cases
        && counts.known_malformed_oracle_cases == 1
        && counts.known_malformed_source_cases == 1
        && counts.error_cases == 0
}

fn f1(counts: ExactCounts) -> Option<f64> {
    let denominator = 2 * counts.true_positives + counts.false_positives + counts.false_negatives;
    (denominator != 0).then(|| 2.0 * counts.true_positives as f64 / denominator as f64)
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros())
        .expect("one case analysis duration fits in u64 microseconds")
}

fn latency_summary(values: &[u64]) -> LatencySummary {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    LatencySummary {
        measured_cases: sorted.len(),
        p50: nearest_rank(&sorted, 50),
        p95: nearest_rank(&sorted, 95),
        max: sorted.last().copied(),
    }
}

fn size_summary(values: &[usize]) -> SizeSummary {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    SizeSummary {
        measured_cases: sorted.len(),
        p50: nearest_rank(&sorted, 50),
        p95: nearest_rank(&sorted, 95),
        max: sorted.last().copied(),
    }
}

fn nearest_rank<T: Copy>(sorted: &[T], percentile: usize) -> Option<T> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percentile * sorted.len()).div_ceil(100);
    Some(sorted[rank - 1])
}

#[cfg(target_os = "linux")]
fn process_vm_hwm_kib() -> Result<Option<u64>> {
    let status =
        fs::read_to_string("/proc/self/status").context("failed to read /proc/self/status")?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .context("/proc/self/status has no VmHWM field")?;
    let fields: Vec<_> = line.split_whitespace().collect();
    ensure!(
        fields.len() == 3 && fields[0] == "VmHWM:" && fields[2] == "kB",
        "unexpected VmHWM format: {line}"
    );
    let value = fields[1]
        .parse::<u64>()
        .context("VmHWM value is not an integer")?;
    Ok(Some(value))
}

#[cfg(not(target_os = "linux"))]
fn process_vm_hwm_kib() -> Result<Option<u64>> {
    Ok(None)
}

fn write_report(report: &EvaluationReport, output: Option<&Path>) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    if let Some(path) = output {
        fs::write(path, &bytes)
            .with_context(|| format!("failed to write evaluation report {}", path.display()))?;
    } else {
        io::stdout().lock().write_all(&bytes)?;
    }
    Ok(())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_malformed_oracle_requires_both_pins() {
        assert!(
            KNOWN_MALFORMED_ORACLE_PATH.ends_with("TestInputOutputFormat/GOD.json")
                && KNOWN_MALFORMED_ORACLE_BLAKE3
                    == "3a2a4c674f7e549421562088d73a3ef096986004923355429d5d6f99a912af9a"
        );
    }

    #[test]
    fn committed_canonical_manifest_matches_the_pinned_digest() {
        let manifest = include_bytes!("../../benchmarks/diffbenchmark-literature-manifest-v3.json");
        assert_eq!(
            blake3::hash(manifest).to_hex().as_str(),
            CANONICAL_MATERIALIZATION_MANIFEST_BLAKE3
        );
    }

    #[test]
    fn known_malformed_source_requires_every_identity_pin() {
        let mut case = MaterializedCase {
            oracle_path: KNOWN_MALFORMED_SOURCE_ORACLE_PATH.to_owned(),
            oracle_blake3: KNOWN_MALFORMED_SOURCE_ORACLE_BLAKE3.to_owned(),
            oracle_repository_url: "https://github.com/Alluxio/alluxio".to_owned(),
            fetched_repository_url: "https://github.com/Alluxio/alluxio".to_owned(),
            commit: KNOWN_MALFORMED_SOURCE_COMMIT.to_owned(),
            parent: KNOWN_MALFORMED_SOURCE_PARENT.to_owned(),
            before: stratadiff::diffbenchmark_materialization::MaterializedSource {
                repository_path: KNOWN_MALFORMED_SOURCE_PATH.to_owned(),
                materialized_path: "sources/0002/before.source".to_owned(),
                content_blake3: KNOWN_MALFORMED_SOURCE_BEFORE_BLAKE3.to_owned(),
            },
            after: stratadiff::diffbenchmark_materialization::MaterializedSource {
                repository_path: KNOWN_MALFORMED_SOURCE_PATH.to_owned(),
                materialized_path: "sources/0002/after.source".to_owned(),
                content_blake3: KNOWN_MALFORMED_SOURCE_AFTER_BLAKE3.to_owned(),
            },
        };
        assert!(is_known_malformed_source(&case));
        case.after.content_blake3 = "0".repeat(64);
        assert!(!is_known_malformed_source(&case));
    }

    #[test]
    fn known_malformed_oracle_is_finished_during_preparation() {
        let prepared = oracle_parse_failure(
            CaseIdentity {
                index: 0,
                oracle_path: KNOWN_MALFORMED_ORACLE_PATH.to_owned(),
                commit: "a".repeat(40),
                before_repository_path: "Before.java".to_owned(),
                after_repository_path: "After.java".to_owned(),
                before_materialized_path: "sources/0000/before.source".to_owned(),
                after_materialized_path: "sources/0000/after.source".to_owned(),
            },
            KNOWN_MALFORMED_ORACLE_BLAKE3,
            "invalid JSON".to_owned(),
        );

        assert!(matches!(
            prepared,
            PreparedCase::Finished(report)
                if matches!(report.outcome, CaseOutcome::KnownMalformedOracle { .. })
        ));
    }

    #[test]
    fn benchmark_completion_requires_the_exact_pinned_outcome() {
        let counts = RunCounts {
            manifest_cases: 285,
            selected_cases: 285,
            evaluated_cases: 283,
            verified_reports: 283,
            successful_replays: 283,
            known_malformed_oracle_cases: 1,
            known_malformed_source_cases: 1,
            error_cases: 0,
        };
        assert!(is_benchmark_complete(true, true, true, true, &counts));
        assert!(!is_benchmark_complete(false, true, true, true, &counts));
        assert!(!is_benchmark_complete(true, false, true, true, &counts));
        assert!(!is_benchmark_complete(true, true, false, true, &counts));
        assert!(!is_benchmark_complete(true, true, true, false, &counts));

        let incomplete = RunCounts {
            evaluated_cases: 282,
            error_cases: 1,
            ..counts
        };
        assert!(!is_benchmark_complete(true, true, true, true, &incomplete));
    }

    #[test]
    fn sha_validation_requires_lowercase_hexadecimal() {
        assert!(validate_sha(&"a".repeat(40)).is_ok());
        assert!(validate_sha(&"A".repeat(40)).is_err());
    }

    #[test]
    fn nearest_rank_percentiles_are_deterministic() {
        assert_eq!(nearest_rank::<u64>(&[], 50), None);
        assert_eq!(nearest_rank(&[1], 95), Some(1));
        assert_eq!(nearest_rank(&[1, 2, 3, 4], 50), Some(2));
        assert_eq!(nearest_rank(&(1..=20).collect::<Vec<_>>(), 95), Some(19));
    }

    #[test]
    fn representation_counts_aggregate_group_units() {
        let aggregate = RepresentationCounts {
            eligible_multi_groups: 1,
            forced_touched_multi_groups: 1,
            forced_gold_edges_in_multi_groups: 2,
            forced_false_positive_edges_incident_to_multi_groups: 0,
        }
        .add(RepresentationCounts {
            eligible_multi_groups: 2,
            forced_touched_multi_groups: 1,
            forced_gold_edges_in_multi_groups: 0,
            forced_false_positive_edges_incident_to_multi_groups: 3,
        })
        .metrics();

        assert_eq!(aggregate.eligible_multi_groups, 3);
        assert_eq!(aggregate.forced_touched_multi_groups, 2);
        assert_eq!(aggregate.forced_gold_edges_in_multi_groups, 2);
        assert_eq!(
            aggregate.forced_false_positive_edges_incident_to_multi_groups,
            3
        );
        assert_eq!(aggregate.multi_group_overclaim_rate, Some(2.0 / 3.0));
    }

    #[test]
    fn strict_cache_cli_requires_an_explicit_java_executable() {
        let error = Cli::try_parse_from([
            "stratadiff-evaluate",
            "/checkout",
            "/materialization",
            "--jdt-cache",
            "/cache",
        ])
        .unwrap_err();
        assert!(error.to_string().contains("--java-executable"));
    }

    #[cfg(unix)]
    mod jdt_runtime {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        const TEST_GUMTREE_JAR: &[u8] = b"test GumTree JAR\n";
        const TEST_JDT_JAR: &[u8] = b"test JDT JAR\n";
        const TEST_ECJ_JAR: &[u8] = b"test ECJ JAR\n";
        const TEST_HELPER_SOURCE: &[u8] = b"class EnumerateJdt {}\n";
        const TEST_SOURCE: &[u8] = b"class Demo {}\n";

        struct StrictCacheFixture {
            _temporary_directory: tempfile::TempDir,
            cache: PathBuf,
            java: PathBuf,
            source: PathBuf,
            gumtree_sha256: String,
            jdt_sha256: String,
            ecj_sha256: String,
        }

        impl StrictCacheFixture {
            fn new(enumerator_body: &str) -> Self {
                let temporary_directory = tempfile::tempdir().unwrap();
                let cache = temporary_directory.path().join("cache");
                let downloads = cache.join("downloads");
                let provenance = cache.join("provenance");
                fs::create_dir_all(&downloads).unwrap();
                fs::create_dir(&provenance).unwrap();
                fs::write(downloads.join("gumtree-3.0.0.jar"), TEST_GUMTREE_JAR).unwrap();
                fs::write(
                    downloads.join("org.eclipse.jdt.core-3.35.0.jar"),
                    TEST_JDT_JAR,
                )
                .unwrap();
                fs::write(downloads.join("ecj-3.35.0.jar"), TEST_ECJ_JAR).unwrap();
                fs::write(
                    provenance.join("EnumerateJdt.java.source"),
                    TEST_HELPER_SOURCE,
                )
                .unwrap();

                let java = temporary_directory.path().join("trusted-java");
                write_executable(
                    &java,
                    &format!(
                        "#!/bin/sh\nset -eu\nif [ \"$#\" -eq 2 ] && [ \"$1\" = '{JAVA_VERSION_MAX_HEAP}' ] && [ \"$2\" = -version ]; then\n  printf 'openjdk version \"17.0.1\"\\n' >&2\n  exit 0\nfi\n{enumerator_body}\n"
                    ),
                );
                let java = fs::canonicalize(java).unwrap();
                fs::write(
                    provenance.join("java-executable"),
                    format!("{}\n", java.display()),
                )
                .unwrap();
                let source = temporary_directory.path().join("Demo.java");
                fs::write(&source, TEST_SOURCE).unwrap();

                Self {
                    _temporary_directory: temporary_directory,
                    cache,
                    java,
                    source,
                    gumtree_sha256: sha256_bytes(TEST_GUMTREE_JAR),
                    jdt_sha256: sha256_bytes(TEST_JDT_JAR),
                    ecj_sha256: sha256_bytes(TEST_ECJ_JAR),
                }
            }

            fn validate(&self) -> Result<VerifiedJdtCache> {
                self.validate_with_java(&self.java)
            }

            fn validate_with_java(&self, java: &Path) -> Result<VerifiedJdtCache> {
                validate_jdt_cache_with_expected(
                    &self.cache,
                    java,
                    &JdtArtifactDigests {
                        gumtree_fat_jar: &self.gumtree_sha256,
                        eclipse_jdt_core_jar: &self.jdt_sha256,
                        ecj_jar: &self.ecj_sha256,
                    },
                    TEST_HELPER_SOURCE,
                )
            }

            fn run(&self, cache: VerifiedJdtCache) -> Result<Vec<Vec<EnumeratedJdtNode>>> {
                let runtime = JdtRuntime::Verified(cache);
                let output = run_jdt_enumerator(&runtime, &[self.source.as_path()])?;
                validate_jdt_process_output(output, &[TEST_SOURCE])
            }
        }

        fn write_executable(path: &Path, contents: &str) {
            fs::write(path, contents).unwrap();
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }

        fn valid_protocol(suffix: &str) -> String {
            format!(
                "test \"$1\" = '{JDT_ENUMERATOR_MAX_HEAP}'\nprintf 'HELLO\\t{JDT_PROFILE}\\t{JDT_PROTOCOL}\\t1\\nBEGIN\\t0\\t{}\\nEND\\t0\\t0\\nDONE\\t1\\t0\\n{suffix}'\n",
                sha256_bytes(TEST_SOURCE),
            )
        }

        #[test]
        fn strict_cache_uses_a_private_verified_snapshot() {
            let fixture = StrictCacheFixture::new(&valid_protocol(""));
            let cache = fixture.validate().unwrap();
            assert_eq!(
                fs::metadata(cache._runtime_directory.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            for snapshot in [
                &cache.gumtree_fat_jar,
                &cache.eclipse_jdt_core_jar,
                &cache.ecj_jar,
                &cache.helper_source,
            ] {
                assert_eq!(
                    fs::metadata(snapshot).unwrap().permissions().mode() & 0o777,
                    0o600,
                    "unexpected permissions for {}",
                    snapshot.display()
                );
            }
            assert!(
                cache
                    .gumtree_fat_jar
                    .starts_with(cache._runtime_directory.path())
            );
            assert!(
                cache
                    .helper_source
                    .starts_with(cache._runtime_directory.path())
            );
            assert!(!cache.gumtree_fat_jar.starts_with(&fixture.cache));

            fs::write(
                fixture.cache.join("downloads/gumtree-3.0.0.jar"),
                b"tampered after validation",
            )
            .unwrap();
            fs::write(
                fixture.cache.join("provenance/EnumerateJdt.java.source"),
                b"tampered after validation",
            )
            .unwrap();

            let runtime = JdtRuntime::Verified(cache);
            let provenance = serde_json::to_value(runtime.provenance()).unwrap();
            assert_eq!(provenance["javaTrustBoundary"], JAVA_TRUST_BOUNDARY);
            let output = run_jdt_enumerator(&runtime, &[fixture.source.as_path()]).unwrap();
            assert_eq!(
                validate_jdt_process_output(output, &[TEST_SOURCE]).unwrap(),
                vec![Vec::new()]
            );
        }

        #[test]
        fn strict_cache_rejects_tampered_artifacts() {
            let jar_fixture = StrictCacheFixture::new(&valid_protocol(""));
            fs::write(
                jar_fixture.cache.join("downloads/gumtree-3.0.0.jar"),
                b"tampered",
            )
            .unwrap();
            let error = jar_fixture.validate().err().unwrap().to_string();
            assert!(error.contains("cached GumTree fat JAR SHA-256 mismatch"));

            let helper_fixture = StrictCacheFixture::new(&valid_protocol(""));
            fs::write(
                helper_fixture
                    .cache
                    .join("provenance/EnumerateJdt.java.source"),
                b"tampered",
            )
            .unwrap();
            let error = helper_fixture.validate().err().unwrap().to_string();
            assert!(error.contains("cached JDT helper source does not match"));
        }

        #[test]
        fn strict_cache_must_match_the_explicit_trusted_java() {
            let fixture = StrictCacheFixture::new(&valid_protocol(""));
            let other_java = fixture
                ._temporary_directory
                .path()
                .join("other-trusted-java");
            write_executable(&other_java, "#!/bin/sh\nexit 0\n");
            let other_java = fs::canonicalize(other_java).unwrap();

            let error = fixture
                .validate_with_java(&other_java)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("does not match trusted Java executable"));
        }

        #[test]
        fn strict_cache_rejects_nonzero_missing_done_and_trailing_output() {
            let nonzero = StrictCacheFixture::new("printf 'failure\\n' >&2\nexit 23");
            let error = nonzero.run(nonzero.validate().unwrap()).unwrap_err();
            assert!(error.to_string().contains("exit status: 23"));

            let missing_done =
                StrictCacheFixture::new(&valid_protocol("").replace("DONE\\t1\\t0\\n", ""));
            let error = missing_done
                .run(missing_done.validate().unwrap())
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("missing JDT enumerator DONE line")
            );

            let trailing = StrictCacheFixture::new(&valid_protocol("TRAILING\\n"));
            let error = trailing.run(trailing.validate().unwrap()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("invalid JDT enumerator TSV line")
            );
        }

        #[test]
        fn java_environment_injection_variables_are_removed() {
            let temporary_directory = tempfile::tempdir().unwrap();
            let executable = temporary_directory.path().join("inspect-environment");
            write_executable(
                &executable,
                "#!/bin/sh\nset -eu\ntest -z \"${CLASSPATH+x}\"\ntest -z \"${JAVA_TOOL_OPTIONS+x}\"\ntest -z \"${JDK_JAVA_OPTIONS+x}\"\ntest -z \"${_JAVA_OPTIONS+x}\"\n",
            );
            let mut command = Command::new(executable);
            for name in [
                "CLASSPATH",
                "JAVA_TOOL_OPTIONS",
                "JDK_JAVA_OPTIONS",
                "_JAVA_OPTIONS",
            ] {
                command.env(name, "injected");
            }
            clear_java_environment(&mut command);

            let output = run_bounded_command(
                &mut command,
                "environment test",
                Duration::from_secs(1),
                1024,
                1024,
            )
            .unwrap();
            assert!(output.status.success());
        }

        #[test]
        fn bounded_command_rejects_timeout_and_excess_output() {
            let mut timeout_command = Command::new("/bin/sh");
            timeout_command.args(["-c", "while :; do :; done"]);
            let error = run_bounded_command(
                &mut timeout_command,
                "timeout test",
                Duration::from_millis(50),
                1024,
                1024,
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("timed out after 50 milliseconds")
            );

            let mut output_command = Command::new("/bin/sh");
            output_command.args(["-c", "printf 12345"]);
            let error = run_bounded_command(
                &mut output_command,
                "output test",
                Duration::from_secs(1),
                4,
                1024,
            )
            .unwrap_err();
            assert!(error.to_string().contains("stdout exceeded 4 bytes"));
        }

        #[test]
        fn parser_enforces_the_node_limit() {
            let source_sha256 = sha256_bytes(TEST_SOURCE);
            let output = format!(
                "HELLO\t{JDT_PROFILE}\t{JDT_PROTOCOL}\t1\nBEGIN\t0\t{source_sha256}\nNODE\tTypeDeclaration\t0\t0\nNODE\tSimpleName\t0\t0\nEND\t0\t2\nDONE\t1\t2\n"
            );
            let error =
                parse_enumerator_output_with_node_limit(output.as_bytes(), &[TEST_SOURCE], 1)
                    .unwrap_err();
            assert!(error.to_string().contains("exceeded the 1-node limit"));
        }
    }
}
