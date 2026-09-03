#![cfg(unix)]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

const REVISION: &str = "870592abd559d0bd822a27eb5c8ea45aee47015b";
const COMMIT: &str = "1111111111111111111111111111111111111111";
const PARENT: &str = "2222222222222222222222222222222222222222";
const ORIGINAL_REPOSITORY: &str = "https://github.com/owner/repo";
const MIRROR_REPOSITORY: &str = "https://github.com/mirror/repo";
const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";
const LITERATURE_CSV: &str = "csv-outputs/adb-paper/literature-exp-INTRA_FILE_ONLY-NO_FILTER-RefOracle-NO_COMMENTS_AND_JAVADOCS-2025_04_10 18:15:50.csv";
const CASE_COUNT: usize = 285;
const BEFORE_BYTES: &[u8] = b"class Before {\n    int value = 1;\n}\n";
const AFTER_BYTES: &[u8] = b"class After {\n    int value = 2;\n}\n";

#[test]
fn materializes_unique_joins_renames_and_verified_digests() {
    let fixture = MaterializeFixture::new(REVISION);
    let repository_map = fixture.write_repository_map(&[COMMIT]);
    let first = fixture.run_with_repository_map(Some(&repository_map), true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let manifest: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(manifest["datasetRevision"], REVISION);
    assert_eq!(manifest["caseCount"], CASE_COUNT);
    assert_eq!(manifest["cases"].as_array().unwrap().len(), CASE_COUNT);
    let renamed = &manifest["cases"][0];
    assert_eq!(renamed["oracleRepositoryUrl"], ORIGINAL_REPOSITORY);
    assert_eq!(renamed["fetchedRepositoryUrl"], MIRROR_REPOSITORY);
    assert_eq!(
        renamed["before"]["repositoryPath"],
        "src/main/java/example/Case000.java"
    );
    assert_eq!(
        renamed["after"]["repositoryPath"],
        "src/main/java/example/RenamedCase000.java"
    );
    assert_eq!(
        renamed["before"]["contentBlake3"],
        blake3::hash(BEFORE_BYTES).to_hex().as_str()
    );
    assert_eq!(
        renamed["after"]["contentBlake3"],
        blake3::hash(AFTER_BYTES).to_hex().as_str()
    );
    let oracle = fixture.oracle_path(0);
    assert_eq!(
        renamed["oracleBlake3"],
        blake3::hash(&fs::read(oracle).unwrap()).to_hex().as_str()
    );
    assert_eq!(
        fs::read(fixture.output.join("sources/0000/before.java")).unwrap(),
        BEFORE_BYTES
    );
    assert_eq!(
        fs::read(fixture.output.join("sources/0000/after.java")).unwrap(),
        AFTER_BYTES
    );
    assert_eq!(
        fs::read(fixture.output.join("manifest.json")).unwrap(),
        first.stdout
    );

    let calls_after_first = fixture.gh_call_count();
    let missing_after = fixture.output.join("sources/0000/after.java");
    fs::remove_file(&missing_after).unwrap();
    let recovered = fixture.run_with_repository_map(Some(&repository_map), true);
    assert!(recovered.status.success());
    assert_eq!(recovered.stdout, first.stdout);
    assert_eq!(fs::read(missing_after).unwrap(), AFTER_BYTES);
    assert_eq!(fixture.gh_call_count(), calls_after_first + 5);

    let calls_after_recovery = fixture.gh_call_count();
    let resumed = fixture.run_with_repository_map(Some(&repository_map), true);
    assert!(resumed.status.success());
    assert_eq!(resumed.stdout, first.stdout);
    assert_eq!(fixture.gh_call_count(), calls_after_recovery);

    let before_path = fixture.output.join("sources/0000/before.java");
    fs::write(&before_path, b"tampered\n").unwrap();
    let rejected = fixture.run_with_repository_map(Some(&repository_map), true);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8(rejected.stderr)
            .unwrap()
            .contains("cached source digest mismatch")
    );
    assert_eq!(fs::read(before_path).unwrap(), b"tampered\n");
}

#[test]
fn deleted_repository_without_a_mapping_fails() {
    let fixture = MaterializeFixture::new(REVISION);
    let output = fixture.run_with_repository_map(None, true);

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("no repository mirror is configured for exact commit")
    );
    assert_eq!(fixture.gh_call_count(), 1);
}

#[test]
fn repository_mapping_for_a_different_commit_is_rejected() {
    let fixture = MaterializeFixture::new(REVISION);
    let repository_map =
        fixture.write_repository_map(&["3333333333333333333333333333333333333333"]);
    let output = fixture.run_with_repository_map(Some(&repository_map), true);

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("no repository mirror is configured for exact commit")
    );
    assert_eq!(fixture.gh_call_count(), 1);
}

#[test]
fn mirror_response_for_a_different_commit_is_rejected() {
    let fixture = MaterializeFixture::new(REVISION);
    let repository_map = fixture.write_repository_map(&[COMMIT]);
    for page in &fixture.page_paths {
        let mut response: Value = serde_json::from_slice(&fs::read(page).unwrap()).unwrap();
        response["sha"] = json!("3333333333333333333333333333333333333333");
        fs::write(page, serde_json::to_vec(&response).unwrap()).unwrap();
    }

    let output = fixture.run_with_repository_map(Some(&repository_map), true);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("GitHub commit response SHA mismatch")
    );
    assert_eq!(fixture.gh_call_count(), 2);
}

#[test]
fn rejects_revision_mismatch_before_materialization() {
    let fixture = MaterializeFixture::new("0000000000000000000000000000000000000000");
    let output = fixture.run();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("DiffBenchmark revision mismatch")
    );
    assert!(!fixture.output.exists());
    assert_eq!(fixture.gh_call_count(), 0);
}

#[test]
fn rejects_duplicate_literature_join() {
    let fixture = MaterializeFixture::new(REVISION);
    let duplicate = format!(
        "https://github.com/owner/repo/commit/{COMMIT},{}\n",
        encoded_name(0)
    );
    let path = fixture.checkout.join(LITERATURE_CSV);
    let mut csv = fs::read_to_string(&path).unwrap();
    csv.push_str(&duplicate);
    fs::write(path, csv).unwrap();

    let output = fixture.run();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("duplicate literature CSV join key")
    );
    assert!(!fixture.output.exists());
}

#[test]
fn rejects_missing_info_join() {
    let fixture = MaterializeFixture::new(REVISION);
    let path = fixture.checkout.join("info.csv");
    let csv = fs::read_to_string(&path).unwrap();
    let filtered = csv
        .lines()
        .filter(|line| !line.ends_with("Case000.java"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(path, filtered).unwrap();

    let output = fixture.run();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("missing info.csv join")
    );
    assert!(!fixture.output.exists());
}

#[test]
fn rejects_malformed_csv_records() {
    let fixture = MaterializeFixture::new(REVISION);
    fs::write(
        fixture.checkout.join(LITERATURE_CSV),
        format!(
            "url,srcFileName\nhttps://github.com/owner/repo/commit/{COMMIT},{},extra\n",
            encoded_name(0)
        ),
    )
    .unwrap();

    let output = fixture.run();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("failed to parse literature CSV")
    );
    assert!(!fixture.output.exists());
}

#[test]
fn rejects_a_nonempty_unrecognized_output_directory() {
    let fixture = MaterializeFixture::new(REVISION);
    fs::create_dir(&fixture.output).unwrap();
    fs::write(fixture.output.join("foreign.txt"), b"foreign\n").unwrap();

    let output = fixture.run();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("refusing non-empty output directory")
    );
    assert_eq!(
        fs::read(fixture.output.join("foreign.txt")).unwrap(),
        b"foreign\n"
    );
    assert_eq!(fixture.gh_call_count(), 0);
}

struct MaterializeFixture {
    _temporary_directory: TempDir,
    checkout: PathBuf,
    output: PathBuf,
    command_path: std::ffi::OsString,
    page_paths: [PathBuf; 3],
    before_path: PathBuf,
    after_path: PathBuf,
    gh_log: PathBuf,
}

impl MaterializeFixture {
    fn new(revision: &str) -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let checkout = temporary_directory.path().join("checkout");
        let output = temporary_directory.path().join("output");
        fs::create_dir_all(&checkout).unwrap();
        write_dataset(&checkout);

        let page_paths = write_commit_pages(temporary_directory.path());
        let before_path = temporary_directory.path().join("before.java");
        let after_path = temporary_directory.path().join("after.java");
        fs::write(&before_path, BEFORE_BYTES).unwrap();
        fs::write(&after_path, AFTER_BYTES).unwrap();
        let gh_log = temporary_directory.path().join("gh.log");

        let command_directory = temporary_directory.path().join("bin");
        fs::create_dir(&command_directory).unwrap();
        write_executable(
            &command_directory.join("git"),
            &format!(
                "#!/bin/sh\nset -eu\ntest \"$1\" = -C\ntest \"$3\" = rev-parse\ntest \"$4\" = --verify\ntest \"$5\" = 'HEAD^{{commit}}'\nprintf '%s\\n' '{revision}'\n"
            ),
        );
        write_executable(
            &command_directory.join("gh"),
            r#"#!/bin/sh
set -eu
test "$1" = api
endpoint=
for argument in "$@"; do
  endpoint=$argument
done
printf '%s\n' "$endpoint" >> "$GH_LOG"
case "$endpoint" in
  'repos/owner/repo/commits/'*)
    if [ "${GH_FAIL_ORIGINAL-0}" = 1 ]; then
      printf 'original repository unavailable\n' >&2
      exit 4
    fi
    ;;
esac
case "$endpoint" in
  *'/commits/'*'page=1') exec /bin/cat "$GH_PAGE_1" ;;
  *'/commits/'*'page=2') exec /bin/cat "$GH_PAGE_2" ;;
  *'/commits/'*'page=3') exec /bin/cat "$GH_PAGE_3" ;;
  *"?ref=$GH_PARENT") exec /bin/cat "$GH_BEFORE" ;;
  *"?ref=$GH_COMMIT") exec /bin/cat "$GH_AFTER" ;;
  *) printf 'unexpected gh endpoint: %s\n' "$endpoint" >&2; exit 9 ;;
esac
"#,
        );

        let mut paths = vec![command_directory];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
        let command_path = env::join_paths(paths).unwrap();
        Self {
            _temporary_directory: temporary_directory,
            checkout,
            output,
            command_path,
            page_paths,
            before_path,
            after_path,
            gh_log,
        }
    }

    fn run(&self) -> Output {
        self.run_with_repository_map(None, false)
    }

    fn run_with_repository_map(
        &self,
        repository_map: Option<&Path>,
        fail_original: bool,
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff-materialize"));
        command
            .arg(&self.checkout)
            .arg(&self.output)
            .env("PATH", &self.command_path)
            .env("GH_PAGE_1", &self.page_paths[0])
            .env("GH_PAGE_2", &self.page_paths[1])
            .env("GH_PAGE_3", &self.page_paths[2])
            .env("GH_BEFORE", &self.before_path)
            .env("GH_AFTER", &self.after_path)
            .env("GH_LOG", &self.gh_log)
            .env("GH_COMMIT", COMMIT)
            .env("GH_PARENT", PARENT)
            .env("GH_FAIL_ORIGINAL", if fail_original { "1" } else { "0" });
        if let Some(repository_map) = repository_map {
            command.arg("--repository-map").arg(repository_map);
        }
        command.output().unwrap()
    }

    fn write_repository_map(&self, commits: &[&str]) -> PathBuf {
        let path = self._temporary_directory.path().join("repository-map.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "stratadiff-repository-mirrors-v1",
                "mirrors": [{
                    "originalRepositoryUrl": ORIGINAL_REPOSITORY,
                    "mirrorRepositoryUrl": MIRROR_REPOSITORY,
                    "commits": commits,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        path
    }

    fn oracle_path(&self, index: usize) -> PathBuf {
        self.checkout
            .join(ORACLE_ROOT)
            .join("owner.repo")
            .join(COMMIT)
            .join(encoded_name(index))
            .join("GOD.json")
    }

    fn gh_call_count(&self) -> usize {
        match fs::read_to_string(&self.gh_log) {
            Ok(log) => log.lines().count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("failed to read gh log: {error}"),
        }
    }
}

fn write_dataset(checkout: &Path) {
    let mut literature = String::from("url,srcFileName\n");
    let mut info = String::from("commit,file\n");
    for index in 0..CASE_COUNT {
        let encoded = encoded_name(index);
        writeln!(
            literature,
            "https://github.com/owner/repo/commit/{COMMIT},{encoded}"
        )
        .unwrap();
        writeln!(info, "{COMMIT},{}", repository_path(index)).unwrap();
        let oracle = checkout
            .join(ORACLE_ROOT)
            .join("owner.repo")
            .join(COMMIT)
            .join(encoded)
            .join("GOD.json");
        fs::create_dir_all(oracle.parent().unwrap()).unwrap();
        fs::write(oracle, format!("oracle {index:03}\n")).unwrap();
    }
    let literature_path = checkout.join(LITERATURE_CSV);
    fs::create_dir_all(literature_path.parent().unwrap()).unwrap();
    fs::write(literature_path, literature).unwrap();
    fs::write(checkout.join("info.csv"), info).unwrap();
}

fn write_commit_pages(root: &Path) -> [PathBuf; 3] {
    let mut files = Vec::new();
    for index in 0..CASE_COUNT {
        if index == 0 {
            files.push(json!({
                "filename": "src/main/java/example/RenamedCase000.java",
                "status": "renamed",
                "previous_filename": repository_path(index),
            }));
        } else {
            files.push(json!({
                "filename": repository_path(index),
                "status": "modified",
            }));
        }
    }

    let page_paths = [
        root.join("commit-page-1.json"),
        root.join("commit-page-2.json"),
        root.join("commit-page-3.json"),
    ];
    for (page, chunk) in page_paths.iter().zip(files.chunks(100)) {
        fs::write(
            page,
            serde_json::to_vec(&json!({
                "sha": COMMIT,
                "parents": [{"sha": PARENT}],
                "files": chunk,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    page_paths
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn repository_path(index: usize) -> String {
    format!("src/main/java/example/Case{index:03}.java")
}

fn encoded_name(index: usize) -> String {
    format!("src.main.java.example.Case{index:03}")
}
