#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const REVISION: &str = "870592abd559d0bd822a27eb5c8ea45aee47015b";
const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";

#[test]
fn audits_a_temporary_checkout_and_controls_invalid_exit_status() {
    let fixture = BenchmarkFixture::new(REVISION);
    fixture.write_oracle(
        "z-valid/GOD.json",
        r#"{
          "intraFileMappings": {
            "matchedElements": [
              {"left":"a","right":"b","info":"MethodDeclaration[0-1]:MysteryNode[2-3]"}
            ],
            "mappings": [
              {"left":"c","right":"d","info":"not-an-info"}
            ]
          },
          "interFileMappings": {
            "Moved to File: Other.java": {
              "matchedElements": [
                {"left":"e","right":"f","info":"UnknownNode[0-0]:UnknownNode[0-0]"}
              ],
              "mappings": [
                {"left":"g","right":"h","info":"SimpleName[4-5]:SimpleName[6-7]"}
              ]
            }
          }
        }"#,
    );
    fixture.write_oracle("b-invalid/GOD.json", "{");
    fixture.write_oracle("a-invalid/GOD.json", "not JSON");
    fixture.write_oracle("ignored/not-god.json", "not JSON");

    let rejected = fixture.run(&[]);
    assert!(!rejected.status.success());
    let report: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(report["revision"], REVISION);
    assert_eq!(report["files"]["total"], 3);
    assert_eq!(report["files"]["validJson"], 1);
    assert_eq!(report["files"]["invalidJson"], 2);
    assert_eq!(report["intraFile"]["matchedElements"], 1);
    assert_eq!(report["intraFile"]["mappings"], 1);
    assert_eq!(report["interFile"]["matchedElements"], 1);
    assert_eq!(report["interFile"]["mappings"], 1);
    assert_eq!(report["unsupportedJdtTypes"]["MysteryNode"], 1);
    assert_eq!(report["unsupportedJdtTypes"]["UnknownNode"], 2);
    assert_eq!(report["infoParseErrors"], 1);
    assert!(
        report["invalidFiles"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("a-invalid/GOD.json")
    );
    assert!(
        report["invalidFiles"][1]["path"]
            .as_str()
            .unwrap()
            .ends_with("b-invalid/GOD.json")
    );

    let allowed = fixture.run(&["--allow-invalid"]);
    assert!(allowed.status.success());
    assert_eq!(allowed.stdout, rejected.stdout);
}

#[test]
fn rejects_a_checkout_at_any_other_revision() {
    let fixture = BenchmarkFixture::new("0000000000000000000000000000000000000000");
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("DiffBenchmark revision mismatch"));
    assert!(stderr.contains(REVISION));
}

struct BenchmarkFixture {
    _temporary_directory: TempDir,
    checkout: PathBuf,
    path: std::ffi::OsString,
}

impl BenchmarkFixture {
    fn new(revision: &str) -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let checkout = temporary_directory.path().join("checkout");
        fs::create_dir_all(checkout.join(ORACLE_ROOT)).unwrap();

        let command_directory = temporary_directory.path().join("bin");
        fs::create_dir(&command_directory).unwrap();
        let git = command_directory.join("git");
        fs::write(
            &git,
            format!(
                "#!/bin/sh\nset -eu\ntest \"$1\" = -C\ntest \"$3\" = rev-parse\ntest \"$4\" = --verify\ntest \"$5\" = 'HEAD^{{commit}}'\nprintf '%s\\n' '{revision}'\n"
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&git).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&git, permissions).unwrap();

        let mut paths = vec![command_directory];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
        let path = env::join_paths(paths).unwrap();
        Self {
            _temporary_directory: temporary_directory,
            checkout,
            path,
        }
    }

    fn write_oracle(&self, relative_path: &str, contents: &str) {
        let path = self.checkout.join(ORACLE_ROOT).join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn run(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_stratadiff-benchmark"))
            .arg(&self.checkout)
            .args(arguments)
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }
}
