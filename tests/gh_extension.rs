use std::{fs, path::Path, path::PathBuf, process::Command};

fn git(directory: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}

fn assert_git_success(output: &std::process::Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn gh_extension_resume_contract_passes_with_stubbed_tools() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg(root.join("extensions/gh-stratadiff/tests/resume_test.sh"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "gh-stratadiff resume tests passed\n"
    );
}

#[test]
fn git_225_fetch_pack_import_never_touches_fetch_head_and_ref_creation_is_cas() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let provider = directory.path().join("provider.git");
    let target = directory.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();

    assert_git_success(&git(&source, &["init", "--quiet"]), "initialize source");
    fs::write(source.join("tracked.txt"), b"reviewed source\n").unwrap();
    assert_git_success(&git(&source, &["add", "tracked.txt"]), "stage source");
    assert_git_success(
        &git(
            &source,
            &[
                "-c",
                "user.name=StrataDiff Test",
                "-c",
                "user.email=stratadiff@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "checkpoint",
            ],
        ),
        "commit source",
    );
    let commit_output = git(&source, &["rev-parse", "HEAD"]);
    assert_git_success(&commit_output, "resolve source commit");
    let commit = String::from_utf8(commit_output.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let provider_ref = format!("refs/stratadiff/provider/checkpoint-{commit}");

    assert_git_success(
        &git(
            directory.path(),
            &["init", "--bare", "--quiet", provider.to_str().unwrap()],
        ),
        "initialize provider",
    );
    let refspec = format!("HEAD:{provider_ref}");
    assert_git_success(
        &git(
            &source,
            &["push", "--quiet", provider.to_str().unwrap(), &refspec],
        ),
        "seed provider",
    );
    assert_git_success(&git(&target, &["init", "--quiet"]), "initialize target");

    let fetch_head = target.join(".git/FETCH_HEAD");
    fs::write(&fetch_head, b"caller fetch state\n").unwrap();
    let fetch_pack = git(
        &target,
        &[
            "-c",
            "fetch.fsckObjects=true",
            "-c",
            "transfer.fsckObjects=true",
            "fetch-pack",
            "--no-progress",
            provider.to_str().unwrap(),
            &provider_ref,
        ],
    );
    assert_git_success(&fetch_pack, "fetch exact provider ref with fetch-pack");
    assert_eq!(
        String::from_utf8(fetch_pack.stdout).unwrap().trim(),
        format!("{commit} {provider_ref}")
    );
    assert_eq!(fs::read(&fetch_head).unwrap(), b"caller fetch state\n");

    let imported_ref = "refs/stratadiff/resume/test/checkpoint";
    assert_git_success(
        &git(
            &target,
            &["update-ref", "--no-deref", imported_ref, &commit, ""],
        ),
        "create imported ref with compare-and-swap",
    );
    let collision = git(
        &target,
        &["update-ref", "--no-deref", imported_ref, &commit, ""],
    );
    assert!(!collision.status.success());
    let resolved = git(&target, &["rev-parse", "--verify", imported_ref]);
    assert_git_success(&resolved, "resolve imported ref");
    assert_eq!(String::from_utf8(resolved.stdout).unwrap().trim(), commit);
    assert_git_success(
        &git(
            &target,
            &["update-ref", "--no-deref", "-d", imported_ref, &commit],
        ),
        "delete imported ref with expected object",
    );

    fs::remove_file(&fetch_head).unwrap();
    let second_fetch_pack = git(
        &target,
        &[
            "fetch-pack",
            "--no-progress",
            provider.to_str().unwrap(),
            &provider_ref,
        ],
    );
    assert_git_success(&second_fetch_pack, "repeat fetch-pack without FETCH_HEAD");
    assert!(!fetch_head.exists());
}
