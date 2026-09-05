use std::fs;
use std::path::Path;
use std::process::Command;

use stratadiff::codeowners::{
    CodeownerIdentity, CodeownersBlocker, CodeownersLineBlockerKind, CodeownersPolicy,
    MAX_CODEOWNERS_BYTES, MAX_OWNERS_PER_RULE,
};
use tempfile::TempDir;

fn git(repository: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn init_repository() -> TempDir {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "--quiet"]);
    git(
        repository.path(),
        &["config", "user.name", "StrataDiff Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "stratadiff@example.com"],
    );
    repository
}

fn commit_all(repository: &Path, message: &str) -> String {
    git(repository, &["add", "-A"]);
    git(repository, &["commit", "--quiet", "-m", message]);
    String::from_utf8(git(repository, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn write(repository: &Path, relative: &str, contents: impl AsRef<[u8]>) {
    let path = repository.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn only_owner(policy: &CodeownersPolicy, path: &str) -> CodeownerIdentity {
    let resolution = policy.resolve_utf8_path(path).unwrap();
    let rule = resolution.matching_rule.unwrap();
    assert_eq!(rule.owner_alternatives.len(), 1);
    rule.owner_alternatives.into_iter().next().unwrap()
}

#[test]
fn source_selection_uses_github_priority_and_an_exact_commit() {
    let repository = init_repository();
    write(repository.path(), "docs/CODEOWNERS", "* @docs\n");
    write(repository.path(), "CODEOWNERS", "* @root\n");
    write(repository.path(), ".github/CODEOWNERS", "* @dot-github\n");
    let first_commit = commit_all(repository.path(), "three sources");

    let first = CodeownersPolicy::load(repository.path(), &first_commit).unwrap();
    assert_eq!(first.source().base_commit, first_commit);
    assert_eq!(first.source().path, ".github/CODEOWNERS");
    assert_eq!(first.source().byte_len, first.contents().len());
    assert_eq!(
        first.source().blake3,
        blake3::hash(first.contents()).to_hex().to_string()
    );
    assert_eq!(
        first.source().blob_oid,
        String::from_utf8(git(
            repository.path(),
            &["rev-parse", &format!("{first_commit}:.github/CODEOWNERS")],
        ))
        .unwrap()
        .trim()
    );
    assert_eq!(
        only_owner(&first, "src/lib.rs"),
        CodeownerIdentity::User {
            login: "dot-github".to_owned()
        }
    );

    fs::remove_file(repository.path().join(".github/CODEOWNERS")).unwrap();
    let second_commit = commit_all(repository.path(), "fall back to root");
    let second = CodeownersPolicy::load(repository.path(), &second_commit).unwrap();
    assert_eq!(second.source().path, "CODEOWNERS");
    assert_eq!(
        only_owner(&second, "src/lib.rs"),
        CodeownerIdentity::User {
            login: "root".to_owned()
        }
    );

    fs::remove_file(repository.path().join("CODEOWNERS")).unwrap();
    let third_commit = commit_all(repository.path(), "fall back to docs");
    let third = CodeownersPolicy::load(repository.path(), &third_commit).unwrap();
    assert_eq!(third.source().path, "docs/CODEOWNERS");

    let pinned_first = CodeownersPolicy::load(repository.path(), &first_commit).unwrap();
    assert_eq!(pinned_first.source(), first.source());
    assert_eq!(pinned_first.contents(), first.contents());
}

#[test]
fn invalid_selected_file_is_a_typed_blocker_and_never_falls_back() {
    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "* @valid-root\n");
    write(
        repository.path(),
        ".github/CODEOWNERS",
        "*.rs @valid\n!*.md @forbidden-negation\n*.txt @bad!owner\n",
    );
    let commit = commit_all(repository.path(), "invalid preferred source");

    let error = CodeownersPolicy::load(repository.path(), &commit).unwrap_err();
    let CodeownersBlocker::InvalidLines { source, errors } = error else {
        panic!("expected invalid-lines blocker");
    };
    assert_eq!(source.path, ".github/CODEOWNERS");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].line, 2);
    assert!(matches!(
        errors[0].reason,
        CodeownersLineBlockerKind::InvalidPattern { .. }
    ));
    assert_eq!(errors[1].line, 3);
    assert_eq!(
        errors[1].reason,
        CodeownersLineBlockerKind::InvalidOwner {
            token: "@bad!owner".to_owned()
        }
    );
}

#[test]
fn github_documented_patterns_preserve_last_match_and_provenance() {
    let repository = init_repository();
    let document = r#"# This is a comment.
*       @global-owner1 @global-owner2
*.js    @js-owner #This is an inline comment.
*.go docs@example.com
*.txt @octo-org/octocats
/build/logs/ @doctocat
docs/* docs@example.com
apps/ @octocat
/docs/ @doctocat
/scripts/ @doctocat @octocat
**/logs @octocat
/apps/github
"#;
    write(repository.path(), "CODEOWNERS", document);
    let commit = commit_all(repository.path(), "official examples");
    let policy = CodeownersPolicy::load(repository.path(), &commit).unwrap();

    let javascript = policy.resolve_utf8_path("web/app.js").unwrap();
    let rule = javascript.matching_rule.unwrap();
    assert_eq!(rule.line, 3);
    assert_eq!(rule.pattern, "*.js");
    assert_eq!(
        rule.owner_alternatives,
        [CodeownerIdentity::User {
            login: "js-owner".to_owned()
        }]
    );

    assert_eq!(
        only_owner(&policy, "main.go"),
        CodeownerIdentity::Email {
            address: "docs@example.com".to_owned()
        }
    );
    assert_eq!(
        only_owner(&policy, "notes.txt"),
        CodeownerIdentity::Team {
            organization: "octo-org".to_owned(),
            slug: "octocats".to_owned()
        }
    );
    assert_eq!(
        only_owner(&policy, "build/logs/deep/error.log"),
        CodeownerIdentity::User {
            login: "octocat".to_owned()
        },
        "the later **/logs rule wins"
    );
    assert_eq!(
        only_owner(&policy, "src/build/logs/error.log"),
        CodeownerIdentity::User {
            login: "octocat".to_owned()
        }
    );
    assert_eq!(
        only_owner(&policy, "docs/getting-started.md"),
        CodeownerIdentity::User {
            login: "doctocat".to_owned()
        },
        "the later rooted /docs/ rule wins"
    );
    assert_eq!(
        only_owner(&policy, "docs/build-app/troubleshooting.md"),
        CodeownerIdentity::User {
            login: "doctocat".to_owned()
        }
    );
    assert_eq!(
        only_owner(&policy, "src/apps/index.js"),
        CodeownerIdentity::User {
            login: "octocat".to_owned()
        },
        "the later apps/ match wins over *.js"
    );

    let scripts = policy.resolve_utf8_path("scripts/deploy.sh").unwrap();
    assert_eq!(
        scripts.matching_rule.unwrap().owner_alternatives,
        [
            CodeownerIdentity::User {
                login: "doctocat".to_owned()
            },
            CodeownerIdentity::User {
                login: "octocat".to_owned()
            }
        ]
    );

    let explicitly_unowned = policy.resolve_utf8_path("apps/github/index.rb").unwrap();
    let rule = explicitly_unowned.matching_rule.unwrap();
    assert_eq!(rule.line, 12);
    assert_eq!(rule.pattern, "/apps/github");
    assert!(rule.owner_alternatives.is_empty());

    let fallback = policy.resolve_utf8_path("unmatched/file.rb").unwrap();
    assert_eq!(
        fallback.matching_rule.unwrap().owner_alternatives,
        [
            CodeownerIdentity::User {
                login: "global-owner1".to_owned()
            },
            CodeownerIdentity::User {
                login: "global-owner2".to_owned()
            }
        ]
    );
}

#[test]
fn github_documented_pattern_boundaries_are_preserved() {
    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "/build/logs/ @build\n");
    let build_commit = commit_all(repository.path(), "rooted build logs");
    let build = CodeownersPolicy::load(repository.path(), &build_commit).unwrap();
    assert!(
        build
            .resolve_utf8_path("build/logs/deep/error.log")
            .unwrap()
            .matching_rule
            .is_some()
    );
    assert!(
        build
            .resolve_utf8_path("src/build/logs/error.log")
            .unwrap()
            .matching_rule
            .is_none()
    );

    write(repository.path(), "CODEOWNERS", "docs/* docs@example.com\n");
    let shallow_commit = commit_all(repository.path(), "shallow docs glob");
    let shallow = CodeownersPolicy::load(repository.path(), &shallow_commit).unwrap();
    assert!(
        shallow
            .resolve_utf8_path("docs/getting-started.md")
            .unwrap()
            .matching_rule
            .is_some()
    );
    assert!(
        shallow
            .resolve_utf8_path("docs/build-app/troubleshooting.md")
            .unwrap()
            .matching_rule
            .is_none()
    );

    write(repository.path(), "CODEOWNERS", "apps/ @apps\n");
    let apps_commit = commit_all(repository.path(), "apps at any depth");
    let apps = CodeownersPolicy::load(repository.path(), &apps_commit).unwrap();
    assert!(
        apps.resolve_utf8_path("src/apps/index.js")
            .unwrap()
            .matching_rule
            .is_some()
    );
    assert!(
        apps.resolve_utf8_path("apps")
            .unwrap()
            .matching_rule
            .is_none(),
        "a directory pattern does not match a file with the directory's name"
    );

    write(repository.path(), "CODEOWNERS", "/docs/ @root-docs\n");
    let rooted_docs_commit = commit_all(repository.path(), "rooted docs");
    let rooted_docs = CodeownersPolicy::load(repository.path(), &rooted_docs_commit).unwrap();
    assert!(
        rooted_docs
            .resolve_utf8_path("docs/deep/index.md")
            .unwrap()
            .matching_rule
            .is_some()
    );
    assert!(
        rooted_docs
            .resolve_utf8_path("src/docs/index.md")
            .unwrap()
            .matching_rule
            .is_none()
    );

    write(repository.path(), "CODEOWNERS", "**/logs @logs\n");
    let logs_commit = commit_all(repository.path(), "logs at any depth");
    let logs = CodeownersPolicy::load(repository.path(), &logs_commit).unwrap();
    for path in [
        "logs/error.log",
        "build/logs/error.log",
        "deeply/nested/logs/error.log",
    ] {
        assert!(
            logs.resolve_utf8_path(path)
                .unwrap()
                .matching_rule
                .is_some(),
            "{path}"
        );
    }
}

#[test]
fn explicit_unowned_unmatched_and_reowned_are_distinct() {
    let repository = init_repository();
    write(
        repository.path(),
        "CODEOWNERS",
        "/apps/ @octocat\n/apps/github\n/apps/github/actions @doctocat\n",
    );
    let commit = commit_all(repository.path(), "ownership exceptions");
    let policy = CodeownersPolicy::load(repository.path(), &commit).unwrap();

    assert!(
        policy
            .resolve_utf8_path("README.md")
            .unwrap()
            .matching_rule
            .is_none()
    );
    assert!(
        policy
            .resolve_utf8_path("apps/github/api.rs")
            .unwrap()
            .matching_rule
            .unwrap()
            .owner_alternatives
            .is_empty()
    );
    assert_eq!(
        only_owner(&policy, "apps/github/actions/build.rs"),
        CodeownerIdentity::User {
            login: "doctocat".to_owned()
        }
    );
}

#[test]
fn escaped_leading_hash_is_a_typed_blocker() {
    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "\\#config @owner\n");
    let commit = commit_all(repository.path(), "unsupported escape");

    let error = CodeownersPolicy::load(repository.path(), &commit).unwrap_err();
    let CodeownersBlocker::InvalidLines { errors, .. } = error else {
        panic!("expected invalid-lines blocker");
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 1);
    assert!(matches!(
        errors[0].reason,
        CodeownersLineBlockerKind::InvalidPattern { .. }
    ));
}

#[test]
fn exact_size_limit_is_rejected_before_parsing() {
    let repository = init_repository();
    write(
        repository.path(),
        "CODEOWNERS",
        vec![b'a'; MAX_CODEOWNERS_BYTES],
    );
    let commit = commit_all(repository.path(), "limit");

    let error = CodeownersPolicy::load(repository.path(), &commit).unwrap_err();
    assert!(matches!(
        error,
        CodeownersBlocker::FileTooLarge {
            byte_len,
            exclusive_limit: MAX_CODEOWNERS_BYTES,
            ..
        } if byte_len == MAX_CODEOWNERS_BYTES as u64
    ));
}

#[test]
fn duplicate_owner_tokens_count_toward_the_per_rule_limit() {
    let repository = init_repository();
    let owners = std::iter::repeat_n("@owner", MAX_OWNERS_PER_RULE + 1)
        .collect::<Vec<_>>()
        .join(" ");
    write(repository.path(), "CODEOWNERS", format!("/src/ {owners}\n"));
    let commit = commit_all(repository.path(), "duplicate owner limit");

    assert!(matches!(
        CodeownersPolicy::load(repository.path(), &commit).unwrap_err(),
        CodeownersBlocker::OwnersPerRuleLimitExceeded {
            line: 1,
            observed,
            limit: MAX_OWNERS_PER_RULE,
        } if observed == MAX_OWNERS_PER_RULE + 1
    ));
}

#[test]
fn non_utf8_and_noncanonical_paths_fail_closed() {
    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "* @owner\n");
    let commit = commit_all(repository.path(), "policy");
    let policy = CodeownersPolicy::load(repository.path(), &commit).unwrap();

    assert!(matches!(
        policy.resolve_git_path(b"src/\xff.rs").unwrap_err(),
        CodeownersBlocker::NonUtf8Path { .. }
    ));
    for path in ["", "/src/lib.rs", "src//lib.rs", "src/../lib.rs", "src/"] {
        assert!(matches!(
            policy.resolve_utf8_path(path).unwrap_err(),
            CodeownersBlocker::InvalidRepositoryPath { .. }
        ));
    }
}

#[test]
fn references_and_non_commit_objects_are_rejected() {
    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "* @owner\n");
    let commit = commit_all(repository.path(), "policy");
    let blob = String::from_utf8(git(
        repository.path(),
        &["rev-parse", &format!("{commit}:CODEOWNERS")],
    ))
    .unwrap()
    .trim()
    .to_owned();

    assert!(matches!(
        CodeownersPolicy::load(repository.path(), "HEAD").unwrap_err(),
        CodeownersBlocker::InvalidBaseCommit { .. }
    ));
    assert!(matches!(
        CodeownersPolicy::load(repository.path(), &blob).unwrap_err(),
        CodeownersBlocker::BaseObjectIsNotCommit { .. }
    ));
}

#[test]
fn missing_file_reports_every_searched_location() {
    let repository = init_repository();
    write(repository.path(), "README.md", "empty policy\n");
    let commit = commit_all(repository.path(), "no codeowners");

    let error = CodeownersPolicy::load(repository.path(), &commit).unwrap_err();
    let CodeownersBlocker::NotFound { searched_paths, .. } = error else {
        panic!("expected not-found blocker");
    };
    assert_eq!(
        searched_paths,
        [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"]
    );
}

#[cfg(unix)]
#[test]
fn selected_symlink_is_rejected_without_falling_back() {
    use std::os::unix::fs::symlink;

    let repository = init_repository();
    write(repository.path(), "CODEOWNERS", "* @root\n");
    fs::create_dir_all(repository.path().join(".github")).unwrap();
    symlink(
        "../CODEOWNERS",
        repository.path().join(".github/CODEOWNERS"),
    )
    .unwrap();
    let commit = commit_all(repository.path(), "symlink");

    assert!(matches!(
        CodeownersPolicy::load(repository.path(), &commit).unwrap_err(),
        CodeownersBlocker::UnsupportedTreeEntry {
            mode,
            object_type,
            ..
        } if mode == "120000" && object_type == "blob"
    ));
}
