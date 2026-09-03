#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};
use stratadiff::diffbenchmark::OffsetRange;
use stratadiff::diffbenchmark_materialization::{
    DIFFBENCHMARK_LITERATURE_CASES, DIFFBENCHMARK_REVISION, MATERIALIZATION_MANIFEST_SCHEMA,
    MaterializationManifest, MaterializedCase, MaterializedSource,
};
use tempfile::TempDir;

const BEFORE: &str = "class Demo { int oldName; }\n";
const AFTER: &str = "class Demo { int newName; }\n";
const COMMIT: &str = "1111111111111111111111111111111111111111";
const PARENT: &str = "2222222222222222222222222222222222222222";
const JDT_PROFILE: &str = "gumtree-3.0.0-jdt-core-3.35.0-ecj-3.35.0-helper-v3";
const JDT_PROTOCOL: &str = "stratadiff-jdt-tsv-v2";

#[test]
fn evaluates_a_limited_manifest_with_one_enumerator_process() {
    let fixture = EvaluationFixture::new();
    let output = fixture.run(&[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.ends_with(b"\n"));
    assert!(output.stdout.starts_with(b"{\n  \"schema\""));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "stratadiff-diffbenchmark-evaluation-v3");
    assert!(report["engineVersion"].is_string());
    assert!(report["engineProvenance"]["gitRevision"].is_string());
    assert!(
        report["engineProvenance"]["gitDirty"].is_boolean()
            || report["engineProvenance"]["gitDirty"].is_null()
    );
    assert!(report["engineProvenance"]["cargoLockSha256"].is_string());
    assert_eq!(report["engineProvenance"]["buildProfile"], "debug");
    assert_eq!(
        report["engineProvenance"]["executableSha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(report["engineProvenanceComplete"], false);
    assert_eq!(report["datasetRevision"], DIFFBENCHMARK_REVISION);
    assert_eq!(report["counts"]["manifestCases"], 285);
    assert_eq!(report["counts"]["selectedCases"], 1);
    assert_eq!(report["counts"]["evaluatedCases"], 1);
    assert_eq!(report["counts"]["verifiedReports"], 1);
    assert_eq!(report["counts"]["successfulReplays"], 1);
    assert_eq!(report["counts"]["knownMalformedOracleCases"], 0);
    assert_eq!(report["counts"]["knownMalformedSourceCases"], 0);
    assert_eq!(report["counts"]["errorCases"], 0);
    assert_eq!(report["fullCorpusSelected"], false);
    assert_eq!(report["canonicalMaterializationManifest"], false);
    assert_eq!(report["benchmarkComplete"], false);
    assert_eq!(report["cases"][0]["outcome"]["status"], "evaluated");
    assert_eq!(
        report["cases"][0]["outcome"]["combinedInputBytes"],
        BEFORE.len() + AFTER.len()
    );
    assert!(report["cases"][0]["outcome"]["serializedDiffReportBytes"].is_number());
    assert_eq!(
        report["aggregate"]["programElements"]["micro"]["truePositives"],
        1
    );
    assert_eq!(report["aggregate"]["mappings"]["micro"]["truePositives"], 1);
    assert_eq!(
        report["aggregate"]["pooledCategoryObservations"]["micro"]["truePositives"],
        2
    );
    assert_eq!(
        report["aggregate"]["predictionDiagnostics"]["combinedBridge"]["enumeratedNodes"],
        4
    );
    assert_eq!(
        report["aggregate"]["programElements"]["ambiguityCoveredGoldRelationRate"]["numerator"],
        0
    );
    assert_eq!(
        report["aggregate"]["programElements"]["ambiguityCoveredGoldRelationRate"]["denominator"],
        1
    );
    assert_eq!(
        report["aggregate"]["programElements"]["ambiguityCoveredGoldRelationRate"]["value"],
        0.0
    );
    assert_eq!(
        report["aggregate"]["programElements"]["micro"]["unforcedGoldRelationRate"],
        0.0
    );
    assert!(
        report["aggregate"]["programElements"]["micro"]
            .get("forcedRelationMissRate")
            .is_none()
    );
    assert!(
        report["aggregate"]["programElements"]["micro"]
            .get("forcedRelationAbstentionRate")
            .is_none()
    );
    assert_eq!(
        report["aggregate"]["programElements"]["representationWarning"]["multiGroupOverclaimRate"],
        Value::Null
    );
    assert_eq!(
        report["aggregate"]["programElements"]["representationWarning"]["eligibleMultiGroups"],
        0
    );
    assert_eq!(
        report["aggregate"]["programElements"]["perfectExactForcedGoldBearingCases"]["numerator"],
        1
    );
    assert_eq!(
        report["aggregate"]["serializedDiffReportBytes"]["measuredCases"],
        1
    );
    assert_eq!(report["provenance"]["status"], "unverified_executable");
    assert_eq!(
        report["provenance"]["executable"],
        fixture.enumerator.to_str().unwrap()
    );
    assert!(report["provenance"]["executableBlake3"].is_string());
    assert_eq!(report["provenance"].as_object().unwrap().len(), 3);
    assert_eq!(report["benchmarkComplete"], false);
    assert!(report["resources"]["processVmHwmKib"].is_number());

    let report_path = fixture.temporary_directory.path().join("evaluation.json");
    let written = fixture.run(&["--output", report_path.to_str().unwrap()]);
    assert!(written.status.success(), "{}", stderr(&written));
    assert!(written.stdout.is_empty());
    let written_report: Value = serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
    assert_eq!(written_report["counts"]["evaluatedCases"], 1);
}

#[test]
fn require_complete_fails_after_writing_an_incomplete_report() {
    let fixture = EvaluationFixture::new();
    let output = fixture.run(&["--require-complete"]);

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["counts"]["errorCases"], 0);
    assert_eq!(report["benchmarkComplete"], false);
}

#[test]
fn rejects_an_unverified_enumerator_without_explicit_permission() {
    let fixture = EvaluationFixture::new();
    let output = fixture.run_without_unverified_permission(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("--allow-unverified-jdt-enumerator"));
}

#[test]
fn rejects_noncanonical_enumerator_framing() {
    let fixture = EvaluationFixture::new();
    fixture.write_enumerator(&format!(
        "#!/bin/sh\nprintf 'HELLO\\t{JDT_PROFILE}\\t{JDT_PROTOCOL}\\t2\\nBEGIN\\t1\\t{}\\nEND\\t1\\t0\\nDONE\\t2\\t0\\n'\n",
        sha256(BEFORE.as_bytes()),
    ));
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("unexpected BEGIN index 1"));
}

#[test]
fn rejects_an_enumerator_source_digest_mismatch() {
    let fixture = EvaluationFixture::new();
    let script = enumerator_script().replace(&sha256(BEFORE.as_bytes()), &"0".repeat(64));
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("source SHA-256 mismatch for argument 0"));
}

#[test]
fn rejects_an_enumerator_end_count_mismatch() {
    let fixture = EvaluationFixture::new();
    let script = enumerator_script().replace("END\\t0\\t2\\n", "END\\t0\\t3\\n");
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("END node count 3 does not match 2 NODE lines"));
}

#[test]
fn rejects_an_enumerator_done_total_mismatch() {
    let fixture = EvaluationFixture::new();
    let script = enumerator_script().replace("DONE\\t2\\t4\\n", "DONE\\t2\\t5\\n");
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("DONE total node count 5 does not match 4 NODE lines"));
}

#[test]
fn rejects_duplicate_nodes_from_an_unverified_enumerator() {
    let fixture = EvaluationFixture::new();
    let before_name = range(BEFORE, "oldName");
    let original = format!(
        "NODE\\tSimpleName\\t{}\\t{}\\nEND\\t0\\t2\\n",
        before_name.start, before_name.end
    );
    let duplicate = format!(
        "NODE\\tSimpleName\\t{}\\t{}\\nNODE\\tSimpleName\\t{}\\t{}\\nEND\\t0\\t3\\n",
        before_name.start, before_name.end, before_name.start, before_name.end
    );
    let script = enumerator_script()
        .replace(&original, &duplicate)
        .replace("DONE\\t2\\t4\\n", "DONE\\t2\\t5\\n");
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("JDT enumerator emitted a duplicate node"));
}

#[test]
fn rejects_a_nonzero_enumerator_exit() {
    let fixture = EvaluationFixture::new();
    fixture.write_enumerator("#!/bin/sh\nprintf 'enumerator failed\\n' >&2\nexit 23\n");
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("JDT enumerator failed with exit status: 23"));
    assert!(stderr(&output).contains("enumerator failed"));
}

#[test]
fn rejects_a_successful_enumerator_that_emits_stderr() {
    let fixture = EvaluationFixture::new();
    let script = format!("{}printf 'unexpected stderr\\n' >&2\n", enumerator_script());
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("emitted stderr despite succeeding"));
}

#[test]
fn rejects_an_enumerator_without_done() {
    let fixture = EvaluationFixture::new();
    let script = enumerator_script().replace("DONE\\t2\\t4\\n", "");
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("missing JDT enumerator DONE line"));
}

#[test]
fn rejects_output_after_done() {
    let fixture = EvaluationFixture::new();
    let script = enumerator_script().replace("DONE\\t2\\t4\\n", "DONE\\t2\\t4\\nTRAILING\\n");
    fixture.write_enumerator(&script);
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("invalid JDT enumerator TSV line"));
}

#[test]
fn records_case_input_errors_and_exits_unsuccessfully() {
    let fixture = EvaluationFixture::new();
    fs::write(
        fixture.materialization.join("sources/0000/before.source"),
        "class Tampered {}\n",
    )
    .unwrap();
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["counts"]["evaluatedCases"], 0);
    assert_eq!(report["counts"]["errorCases"], 1);
    assert_eq!(report["cases"][0]["outcome"]["status"], "error");
    assert_eq!(report["cases"][0]["outcome"]["stage"], "input_validation");
    assert!(
        report["cases"][0]["outcome"]["error"]
            .as_str()
            .unwrap()
            .contains("BLAKE3 mismatch")
    );
}

#[test]
fn rejects_dirty_tracked_oracle_before_evaluation() {
    let fixture = EvaluationFixture::new();
    fs::write(fixture.checkout.join(oracle_path(0)), "dirty\n").unwrap();
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("working tree changes in benchmark inputs"));
}

#[test]
fn rejects_staged_tracked_oracle_before_evaluation() {
    let fixture = EvaluationFixture::new();
    let relative = oracle_path(0);
    fs::write(fixture.checkout.join(&relative), "staged\n").unwrap();
    let status = Command::new(executable_on_path("git"))
        .arg("-C")
        .arg(&fixture.checkout)
        .arg("add")
        .arg(&relative)
        .status()
        .unwrap();
    assert!(status.success());

    let output = fixture.run(&[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("staged changes in benchmark inputs"));
}

#[test]
fn invalid_oracle_is_recorded_without_invoking_the_enumerator() {
    let fixture = EvaluationFixture::new();
    fixture.replace_first_oracle(b"{\n");
    fixture.write_enumerator("#!/bin/sh\nexit 99\n");
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["cases"][0]["outcome"]["status"], "error");
    assert_eq!(report["cases"][0]["outcome"]["stage"], "oracle_parse");
}

struct EvaluationFixture {
    temporary_directory: TempDir,
    checkout: PathBuf,
    materialization: PathBuf,
    enumerator: PathBuf,
    path: std::ffi::OsString,
}

impl EvaluationFixture {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let checkout = temporary_directory.path().join("checkout");
        let materialization = temporary_directory.path().join("materialization");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&materialization).unwrap();

        let first_oracle_path = oracle_path(0);
        let oracle = oracle_json(BEFORE, AFTER);
        write_file(&checkout.join(&first_oracle_path), oracle.as_bytes());
        write_file(
            &materialization.join("sources/0000/before.source"),
            BEFORE.as_bytes(),
        );
        write_file(
            &materialization.join("sources/0000/after.source"),
            AFTER.as_bytes(),
        );

        let mut cases = Vec::with_capacity(DIFFBENCHMARK_LITERATURE_CASES);
        cases.push(MaterializedCase {
            oracle_path: first_oracle_path,
            oracle_blake3: digest(oracle.as_bytes()),
            oracle_repository_url: "https://github.com/example/project".to_owned(),
            fetched_repository_url: "https://github.com/example/project".to_owned(),
            commit: COMMIT.to_owned(),
            parent: PARENT.to_owned(),
            before: MaterializedSource {
                repository_path: "Demo.java".to_owned(),
                materialized_path: "sources/0000/before.source".to_owned(),
                content_blake3: digest(BEFORE.as_bytes()),
            },
            after: MaterializedSource {
                repository_path: "Demo.java".to_owned(),
                materialized_path: "sources/0000/after.source".to_owned(),
                content_blake3: digest(AFTER.as_bytes()),
            },
        });
        for index in 1..DIFFBENCHMARK_LITERATURE_CASES {
            cases.push(MaterializedCase {
                oracle_path: oracle_path(index),
                oracle_blake3: "0".repeat(64),
                oracle_repository_url: "https://github.com/example/project".to_owned(),
                fetched_repository_url: "https://github.com/example/project".to_owned(),
                commit: COMMIT.to_owned(),
                parent: PARENT.to_owned(),
                before: MaterializedSource {
                    repository_path: format!("src/Before{index}.java"),
                    materialized_path: format!("sources/{index:04}/before.source"),
                    content_blake3: "0".repeat(64),
                },
                after: MaterializedSource {
                    repository_path: format!("src/After{index}.java"),
                    materialized_path: format!("sources/{index:04}/after.source"),
                    content_blake3: "0".repeat(64),
                },
            });
        }
        let manifest = MaterializationManifest {
            schema: MATERIALIZATION_MANIFEST_SCHEMA.to_owned(),
            dataset_revision: DIFFBENCHMARK_REVISION.to_owned(),
            case_count: DIFFBENCHMARK_LITERATURE_CASES,
            cases,
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        manifest_bytes.push(b'\n');
        fs::write(materialization.join("manifest.json"), manifest_bytes).unwrap();

        let real_git = executable_on_path("git");
        initialize_git_checkout(&real_git, &checkout);

        let command_directory = temporary_directory.path().join("bin");
        fs::create_dir(&command_directory).unwrap();
        let git = command_directory.join("git");
        write_executable(
            &git,
            &format!(
                "#!/bin/sh\nset -eu\ntest \"$1\" = -C\nif [ \"$3\" = rev-parse ]; then\n  test \"$4\" = --verify\n  test \"$5\" = 'HEAD^{{commit}}'\n  printf '%s\\n' '{DIFFBENCHMARK_REVISION}'\nelse\n  exec '{}' \"$@\"\nfi\n",
                real_git.display(),
            ),
        );
        let enumerator = command_directory.join("enumerate-jdt");
        let fixture = Self {
            temporary_directory,
            checkout,
            materialization,
            enumerator,
            path: joined_path(&command_directory),
        };
        fixture.write_enumerator(&enumerator_script());
        fixture
    }

    fn write_enumerator(&self, contents: &str) {
        write_executable(&self.enumerator, contents);
    }

    fn replace_first_oracle(&self, contents: &[u8]) {
        fs::write(self.checkout.join(oracle_path(0)), contents).unwrap();
        commit_checkout(&executable_on_path("git"), &self.checkout, "replace oracle");

        let manifest_path = self.materialization.join("manifest.json");
        let mut manifest: MaterializationManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.cases[0].oracle_blake3 = digest(contents);
        let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        bytes.push(b'\n');
        fs::write(manifest_path, bytes).unwrap();
    }

    fn run(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stratadiff-evaluate"))
            .arg(&self.checkout)
            .arg(&self.materialization)
            .arg("--jdt-enumerator")
            .arg(&self.enumerator)
            .arg("--allow-unverified-jdt-enumerator")
            .arg("--limit")
            .arg("1")
            .args(extra)
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }

    fn run_without_unverified_permission(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stratadiff-evaluate"))
            .arg(&self.checkout)
            .arg(&self.materialization)
            .arg("--jdt-enumerator")
            .arg(&self.enumerator)
            .arg("--limit")
            .arg("1")
            .args(extra)
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }
}

fn executable_on_path(name: &str) -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap()
}

fn initialize_git_checkout(git: &Path, checkout: &Path) {
    for arguments in [vec!["init", "--quiet"], vec!["add", "."]] {
        let status = Command::new(git)
            .arg("-C")
            .arg(checkout)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }
    commit_checkout(git, checkout, "fixture");
}

fn commit_checkout(git: &Path, checkout: &Path, message: &str) {
    let status = Command::new(git)
        .arg("-C")
        .arg(checkout)
        .args([
            "-c",
            "user.name=StrataDiff Test",
            "-c",
            "user.email=stratadiff@example.invalid",
            "commit",
            "--quiet",
            "-am",
            message,
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

fn oracle_json(before: &str, after: &str) -> String {
    let before_field = range(before, "int oldName;");
    let after_field = range(after, "int newName;");
    let before_name = range(before, "oldName");
    let after_name = range(after, "newName");
    format!(
        r#"{{
  "intraFileMappings": {{
    "matchedElements": [{{"left":"","right":"","info":"FieldDeclaration[{}-{}]:FieldDeclaration[{}-{}]"}}],
    "mappings": [{{"left":"","right":"","info":"SimpleName[{}-{}]:SimpleName[{}-{}]"}}]
  }},
  "interFileMappings": {{}}
}}"#,
        before_field.start,
        before_field.end,
        after_field.start,
        after_field.end,
        before_name.start,
        before_name.end,
        after_name.start,
        after_name.end,
    )
}

fn enumerator_script() -> String {
    let before_field = range(BEFORE, "int oldName;");
    let after_field = range(AFTER, "int newName;");
    let before_name = range(BEFORE, "oldName");
    let after_name = range(AFTER, "newName");
    format!(
        "#!/bin/sh\nset -eu\ntest \"$#\" -eq 2\ncase \"$1\" in */sources/0000/before.source) ;; *) exit 11 ;; esac\ncase \"$2\" in */sources/0000/after.source) ;; *) exit 12 ;; esac\nprintf 'HELLO\\t{JDT_PROFILE}\\t{JDT_PROTOCOL}\\t2\\nBEGIN\\t0\\t{}\\nNODE\\tFieldDeclaration\\t{}\\t{}\\nNODE\\tSimpleName\\t{}\\t{}\\nEND\\t0\\t2\\nBEGIN\\t1\\t{}\\nNODE\\tFieldDeclaration\\t{}\\t{}\\nNODE\\tSimpleName\\t{}\\t{}\\nEND\\t1\\t2\\nDONE\\t2\\t4\\n'\n",
        sha256(BEFORE.as_bytes()),
        before_field.start,
        before_field.end,
        before_name.start,
        before_name.end,
        sha256(AFTER.as_bytes()),
        after_field.start,
        after_field.end,
        after_name.start,
        after_name.end,
    )
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn range(source: &str, fragment: &str) -> OffsetRange {
    let start = source.find(fragment).unwrap();
    OffsetRange {
        start: source[..start].encode_utf16().count(),
        end: source[..start + fragment.len()].encode_utf16().count(),
    }
}

fn oracle_path(index: usize) -> String {
    format!("hrd-oracle/adb-paper/literature-exp/example.project/{COMMIT}/Case{index}/GOD.json")
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn write_file(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn joined_path(command_directory: &Path) -> std::ffi::OsString {
    let mut paths = vec![command_directory.to_owned()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
    env::join_paths(paths).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}
