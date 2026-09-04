use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const UNAVAILABLE: &str = "unavailable";
const GIT_REVISION_ENV: &str = "STRATADIFF_BUILD_GIT_REVISION";
const GIT_DIRTY_ENV: &str = "STRATADIFF_BUILD_GIT_DIRTY";
const CARGO_LOCK_SHA256_ENV: &str = "STRATADIFF_BUILD_CARGO_LOCK_SHA256";
const BUILD_PROFILE_ENV: &str = "STRATADIFF_BUILD_PROFILE";
const RUSTC_VERSION_ENV: &str = "STRATADIFF_BUILD_RUSTC_VERSION";

struct GitProvenance {
    revision: String,
    dirty: bool,
}

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must set CARGO_MANIFEST_DIR"),
    );
    emit_rerun_directives(&manifest_dir);

    let git = read_git_provenance(&manifest_dir);
    let (git_revision, git_dirty) = match git {
        Some(git) => (git.revision, git.dirty.to_string()),
        None => {
            println!("cargo:warning=Git provenance is unavailable; embedding explicit sentinels");
            (UNAVAILABLE.to_owned(), UNAVAILABLE.to_owned())
        }
    };
    let cargo_lock_sha256 = sha256_file(&manifest_dir.join("Cargo.lock")).unwrap_or_else(|| {
        println!("cargo:warning=Cargo.lock provenance is unavailable; embedding a sentinel");
        UNAVAILABLE.to_owned()
    });
    let build_profile = env::var("PROFILE")
        .ok()
        .filter(|profile| is_safe_profile(profile))
        .unwrap_or_else(|| {
            println!("cargo:warning=Cargo build profile is unavailable; embedding a sentinel");
            UNAVAILABLE.to_owned()
        });
    let rustc_version = read_rustc_version().unwrap_or_else(|| {
        println!("cargo:warning=rustc provenance is unavailable; embedding a sentinel");
        UNAVAILABLE.to_owned()
    });

    emit_rustc_env(GIT_REVISION_ENV, &git_revision);
    emit_rustc_env(GIT_DIRTY_ENV, &git_dirty);
    emit_rustc_env(CARGO_LOCK_SHA256_ENV, &cargo_lock_sha256);
    emit_rustc_env(BUILD_PROFILE_ENV, &build_profile);
    emit_rustc_env(RUSTC_VERSION_ENV, &rustc_version);
}

fn read_rustc_version() -> Option<String> {
    let rustc = env::var_os("RUSTC")?;
    let output = Command::new(rustc).arg("--version").output().ok()?;
    if !output.status.success() || !output.stderr.is_empty() {
        return None;
    }
    let version = std::str::from_utf8(&output.stdout)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    is_safe_metadata_line(version).then(|| version.to_owned())
}

fn read_git_provenance(manifest_dir: &Path) -> Option<GitProvenance> {
    let repository_root = PathBuf::from(git_line(manifest_dir, &["rev-parse", "--show-toplevel"])?);
    if fs::canonicalize(repository_root).ok()? != fs::canonicalize(manifest_dir).ok()? {
        return None;
    }
    let revision = git_line(manifest_dir, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !is_lower_hex(&revision, 40) {
        return None;
    }

    let status = git_output(
        manifest_dir,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    Some(GitProvenance {
        revision,
        dirty: !status.is_empty(),
    })
}

fn git_line(manifest_dir: &Path, arguments: &[&str]) -> Option<String> {
    let output = git_output(manifest_dir, arguments)?;
    let value = std::str::from_utf8(&output)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    (!value.is_empty() && !value.contains(['\r', '\n'])).then(|| value.to_owned())
}

fn git_output(manifest_dir: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(arguments)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn sha256_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_safe_profile(profile: &str) -> bool {
    !profile.is_empty()
        && profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn is_safe_metadata_line(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

fn emit_rerun_directives(manifest_dir: &Path) {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=Cargo.lock");
    if let Some(paths) = git_worktree_paths(manifest_dir) {
        for path in paths {
            println!(
                "cargo:rerun-if-changed={}",
                manifest_dir.join(path).display()
            );
        }
    }
    for name in ["HEAD", "index", "packed-refs", "logs/HEAD"] {
        if let Some(path) = git_line(manifest_dir, &["rev-parse", "--git-path", name]) {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                manifest_dir.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    if let Some(reference) = git_line(manifest_dir, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git_line(manifest_dir, &["rev-parse", "--git-path", &reference])
    {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            manifest_dir.join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git_worktree_paths(manifest_dir: &Path) -> Option<Vec<String>> {
    let output = git_output(
        manifest_dir,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let paths = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .filter_map(|path| std::str::from_utf8(path).ok())
        .filter(|path| !path.contains(['\r', '\n']))
        .map(str::to_owned)
        .collect();
    Some(paths)
}

fn emit_rustc_env(name: &str, value: &str) {
    println!("cargo:rustc-env={name}={value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_requires_exact_lowercase_sha1() {
        assert!(is_lower_hex(&"a".repeat(40), 40));
        assert!(!is_lower_hex(&"A".repeat(40), 40));
        assert!(!is_lower_hex(&"a".repeat(39), 40));
        assert!(!is_lower_hex(&format!("{}g", "a".repeat(39)), 40));
    }

    #[test]
    fn profile_is_safe_for_rustc_environment_output() {
        assert!(is_safe_profile("release"));
        assert!(is_safe_profile("bench_release-1"));
        assert!(!is_safe_profile(""));
        assert!(!is_safe_profile("release\ninjected"));
    }

    #[test]
    fn metadata_line_rejects_empty_and_control_characters() {
        assert!(is_safe_metadata_line("rustc 1.90.0 (1159e78c4 2025-09-14)"));
        assert!(!is_safe_metadata_line(""));
        assert!(!is_safe_metadata_line("rustc 1.90.0\ninjected"));
    }
}
