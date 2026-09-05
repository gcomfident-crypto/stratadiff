use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use codeowner::{CodeOwners, ErrorKind, Owner, ParseError, SEARCH_PATHS};
use serde::{Deserialize, Serialize};

pub const MAX_CODEOWNERS_BYTES: usize = codeowner::MAX_FILE_SIZE;
pub const MAX_CODEOWNERS_RULES: usize = 10_000;
pub const MAX_OWNERS_PER_RULE: usize = 100;
pub const MAX_CODEOWNERS_OWNER_TOKENS: usize = 50_000;

const MAX_GIT_METADATA_BYTES: usize = 64 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeownersSource {
    pub base_commit: String,
    pub path: String,
    pub blob_oid: String,
    pub byte_len: usize,
    pub blake3: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CodeownerIdentity {
    User { login: String },
    Team { organization: String, slug: String },
    Email { address: String },
}

impl From<&Owner> for CodeownerIdentity {
    fn from(owner: &Owner) -> Self {
        match owner {
            Owner::User(login) => Self::User {
                login: login.clone(),
            },
            Owner::Team { org, team } => Self::Team {
                organization: org.clone(),
                slug: team.clone(),
            },
            Owner::Email(address) => Self::Email {
                address: address.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeownerRuleMatch {
    pub line: usize,
    pub pattern: String,
    /// GitHub treats owners on one matching line as alternatives: any one can
    /// satisfy the native code-owner review requirement.
    pub owner_alternatives: Vec<CodeownerIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeownerPathResolution {
    pub source: CodeownersSource,
    pub path: String,
    /// `None` means no rule matched. `Some` with no owner alternatives means
    /// the last matching rule deliberately cleared ownership.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_rule: Option<CodeownerRuleMatch>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CodeownersLineBlockerKind {
    InvalidPattern { message: String },
    InvalidOwner { token: String },
    UnsupportedParserError { message: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeownersLineBlocker {
    pub line: usize,
    pub text: String,
    pub reason: CodeownersLineBlockerKind,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CodeownersBlocker {
    InvalidBaseCommit {
        value: String,
    },
    BaseObjectIsNotCommit {
        base_commit: String,
        object_type: String,
    },
    NotFound {
        base_commit: String,
        searched_paths: Vec<String>,
    },
    UnsupportedTreeEntry {
        base_commit: String,
        path: String,
        mode: String,
        object_type: String,
    },
    FileTooLarge {
        base_commit: String,
        path: String,
        blob_oid: String,
        byte_len: u64,
        exclusive_limit: usize,
    },
    NonUtf8Contents {
        base_commit: String,
        path: String,
        blob_oid: String,
    },
    InvalidLines {
        source: Box<CodeownersSource>,
        errors: Vec<CodeownersLineBlocker>,
    },
    RuleLimitExceeded {
        observed: usize,
        limit: usize,
    },
    OwnersPerRuleLimitExceeded {
        line: usize,
        observed: usize,
        limit: usize,
    },
    OwnerTokenLimitExceeded {
        observed: usize,
        limit: usize,
    },
    NonUtf8Path {
        byte_len: usize,
        blake3: String,
    },
    InvalidRepositoryPath {
        path: String,
        reason: String,
    },
    GitSpawn {
        operation: String,
        message: String,
    },
    GitOutputLimit {
        operation: String,
        stream: String,
        limit: usize,
    },
    GitFailed {
        operation: String,
        status: String,
        stderr: String,
    },
    MalformedGitOutput {
        operation: String,
        message: String,
    },
}

impl fmt::Display for CodeownersBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBaseCommit { value } => write!(
                formatter,
                "CODEOWNERS base commit must be a full 40- or 64-character object ID, found {value:?}"
            ),
            Self::BaseObjectIsNotCommit {
                base_commit,
                object_type,
            } => write!(
                formatter,
                "CODEOWNERS base object {base_commit} is {object_type}, not a commit"
            ),
            Self::NotFound {
                base_commit,
                searched_paths,
            } => write!(
                formatter,
                "no CODEOWNERS file exists at commit {base_commit} in {}",
                searched_paths.join(", ")
            ),
            Self::UnsupportedTreeEntry {
                base_commit,
                path,
                mode,
                object_type,
            } => write!(
                formatter,
                "CODEOWNERS entry {base_commit}:{path} has unsupported mode {mode} and type {object_type}"
            ),
            Self::FileTooLarge {
                base_commit,
                path,
                byte_len,
                exclusive_limit,
                ..
            } => write!(
                formatter,
                "CODEOWNERS file {base_commit}:{path} has {byte_len} bytes; it must be strictly smaller than {exclusive_limit} bytes"
            ),
            Self::NonUtf8Contents {
                base_commit, path, ..
            } => write!(
                formatter,
                "CODEOWNERS file {base_commit}:{path} is not valid UTF-8"
            ),
            Self::InvalidLines { source, errors } => write!(
                formatter,
                "CODEOWNERS file {}:{} has {} invalid line(s)",
                source.base_commit,
                source.path,
                errors.len()
            ),
            Self::RuleLimitExceeded { observed, limit } => write!(
                formatter,
                "CODEOWNERS rule count limit exceeded: observed at least {observed}, limit {limit}"
            ),
            Self::OwnersPerRuleLimitExceeded {
                line,
                observed,
                limit,
            } => write!(
                formatter,
                "CODEOWNERS line {line} owner count limit exceeded: observed {observed}, limit {limit}"
            ),
            Self::OwnerTokenLimitExceeded { observed, limit } => write!(
                formatter,
                "CODEOWNERS owner token limit exceeded: observed at least {observed}, limit {limit}"
            ),
            Self::NonUtf8Path { byte_len, blake3 } => write!(
                formatter,
                "repository path is not valid UTF-8 ({byte_len} bytes, BLAKE3 {blake3})"
            ),
            Self::InvalidRepositoryPath { path, reason } => {
                write!(formatter, "invalid repository path {path:?}: {reason}")
            }
            Self::GitSpawn { operation, message } => {
                write!(formatter, "failed to run git {operation}: {message}")
            }
            Self::GitOutputLimit {
                operation,
                stream,
                limit,
            } => write!(
                formatter,
                "git {operation} {stream} exceeded the {limit} byte limit"
            ),
            Self::GitFailed {
                operation,
                status,
                stderr,
            } => write!(
                formatter,
                "git {operation} failed with {status}: {}",
                stderr.trim_end()
            ),
            Self::MalformedGitOutput { operation, message } => {
                write!(
                    formatter,
                    "git {operation} returned malformed output: {message}"
                )
            }
        }
    }
}

impl std::error::Error for CodeownersBlocker {}

#[derive(Clone, Debug)]
pub struct CodeownersPolicy {
    source: CodeownersSource,
    contents: Vec<u8>,
    parsed: CodeOwners,
}

impl CodeownersPolicy {
    /// Load the first CODEOWNERS file in GitHub's documented search order from
    /// one explicit commit. References, abbreviated object IDs, and tag
    /// objects are rejected so the resulting evidence cannot drift.
    pub fn load(repository: &Path, exact_base_commit: &str) -> Result<Self, CodeownersBlocker> {
        let base_commit = validate_exact_object_id(exact_base_commit)?;
        verify_full_object_id(repository, &base_commit)?;
        verify_commit(repository, &base_commit)?;

        let entry = find_source_entry(repository, &base_commit)?.ok_or_else(|| {
            CodeownersBlocker::NotFound {
                base_commit: base_commit.clone(),
                searched_paths: SEARCH_PATHS.iter().map(|path| (*path).to_owned()).collect(),
            }
        })?;

        if entry.object_type != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            return Err(CodeownersBlocker::UnsupportedTreeEntry {
                base_commit,
                path: entry.path,
                mode: entry.mode,
                object_type: entry.object_type,
            });
        }

        let byte_len = blob_size(repository, &entry.oid)?;
        if byte_len >= MAX_CODEOWNERS_BYTES as u64 {
            return Err(CodeownersBlocker::FileTooLarge {
                base_commit,
                path: entry.path,
                blob_oid: entry.oid,
                byte_len,
                exclusive_limit: MAX_CODEOWNERS_BYTES,
            });
        }

        let operation = format!("cat-file blob {}", entry.oid);
        let contents = run_git_bounded(
            repository,
            &["cat-file", "blob", &entry.oid],
            byte_len as usize,
            &operation,
        )?;
        if contents.len() != byte_len as usize {
            return Err(CodeownersBlocker::MalformedGitOutput {
                operation,
                message: format!(
                    "blob size changed between metadata and read: expected {byte_len}, found {}",
                    contents.len()
                ),
            });
        }

        let source = CodeownersSource {
            base_commit,
            path: entry.path,
            blob_oid: entry.oid,
            byte_len: contents.len(),
            blake3: blake3::hash(&contents).to_hex().to_string(),
        };
        let text =
            std::str::from_utf8(&contents).map_err(|_| CodeownersBlocker::NonUtf8Contents {
                base_commit: source.base_commit.clone(),
                path: source.path.clone(),
                blob_oid: source.blob_oid.clone(),
            })?;
        let parsed = parse_policy(text)?;
        let mut errors: Vec<_> = parsed.errors().iter().map(line_blocker).collect();
        errors.extend(unsupported_escape_blockers(text));
        errors.sort_by_key(|error| error.line);
        if !errors.is_empty() {
            return Err(CodeownersBlocker::InvalidLines {
                source: Box::new(source),
                errors,
            });
        }

        Ok(Self {
            source,
            contents,
            parsed,
        })
    }

    pub fn source(&self) -> &CodeownersSource {
        &self.source
    }

    /// Exact bytes read from the recorded Git blob. A passport may attach
    /// these bytes or retrieve the blob by OID and verify `source.blake3`.
    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    /// Resolve a raw repository-relative Git path. Git permits non-UTF-8
    /// names, while GitHub CODEOWNERS patterns are textual; such paths are a
    /// blocker rather than an implicit lossy conversion.
    pub fn resolve_git_path(
        &self,
        path: &[u8],
    ) -> Result<CodeownerPathResolution, CodeownersBlocker> {
        let path = std::str::from_utf8(path).map_err(|_| CodeownersBlocker::NonUtf8Path {
            byte_len: path.len(),
            blake3: blake3::hash(path).to_hex().to_string(),
        })?;
        self.resolve_utf8_path(path)
    }

    /// Resolve a platform path after proving that every component is a
    /// repository-relative UTF-8 component.
    pub fn resolve_path(&self, path: &Path) -> Result<CodeownerPathResolution, CodeownersBlocker> {
        let mut normalized = String::new();
        for component in path.components() {
            let Component::Normal(component) = component else {
                return Err(CodeownersBlocker::InvalidRepositoryPath {
                    path: path.to_string_lossy().into_owned(),
                    reason: "path must contain only repository-relative normal components"
                        .to_owned(),
                });
            };
            let component = component.to_str().ok_or_else(|| {
                let bytes = component.as_encoded_bytes();
                CodeownersBlocker::NonUtf8Path {
                    byte_len: bytes.len(),
                    blake3: blake3::hash(bytes).to_hex().to_string(),
                }
            })?;
            if !normalized.is_empty() {
                normalized.push('/');
            }
            normalized.push_str(component);
        }
        self.resolve_utf8_path(&normalized)
    }

    pub fn resolve_utf8_path(
        &self,
        path: &str,
    ) -> Result<CodeownerPathResolution, CodeownersBlocker> {
        validate_repository_path(path)?;
        let matching_rule = self.parsed.rule_for(path).map(|rule| CodeownerRuleMatch {
            line: rule.line,
            pattern: rule.pattern.as_str().to_owned(),
            owner_alternatives: rule.owners.iter().map(CodeownerIdentity::from).collect(),
        });
        Ok(CodeownerPathResolution {
            source: self.source.clone(),
            path: path.to_owned(),
            matching_rule,
        })
    }

    pub(crate) fn matching_owner_count(
        &self,
        path: &str,
    ) -> Result<Option<usize>, CodeownersBlocker> {
        validate_repository_path(path)?;
        Ok(self.parsed.rule_for(path).map(|rule| rule.owners.len()))
    }
}

fn validate_policy_limits(parsed: &CodeOwners) -> Result<(), CodeownersBlocker> {
    validate_policy_counts(
        parsed
            .rules()
            .iter()
            .map(|rule| (rule.line, rule.owners.len())),
    )
}

fn parse_policy(text: &str) -> Result<CodeOwners, CodeownersBlocker> {
    parse_policy_with(text, CodeOwners::parse)
}

fn parse_policy_with(
    text: &str,
    parser: impl FnOnce(&str) -> CodeOwners,
) -> Result<CodeOwners, CodeownersBlocker> {
    validate_lexical_policy_limits(text)?;
    let parsed = parser(text);
    validate_policy_limits(&parsed)?;
    Ok(parsed)
}

fn validate_lexical_policy_limits(text: &str) -> Result<(), CodeownersBlocker> {
    let mut rule_count = 0_usize;
    let mut owner_token_count = 0_usize;

    for (index, raw_line) in text.lines().enumerate() {
        let content = raw_line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }

        rule_count += 1;
        if rule_count > MAX_CODEOWNERS_RULES {
            return Err(CodeownersBlocker::RuleLimitExceeded {
                observed: rule_count,
                limit: MAX_CODEOWNERS_RULES,
            });
        }

        let mut tokens = content.split_whitespace();
        let _pattern = tokens.next();
        let mut owners_on_line = 0_usize;
        for _owner in tokens {
            owners_on_line += 1;
            if owners_on_line > MAX_OWNERS_PER_RULE {
                return Err(CodeownersBlocker::OwnersPerRuleLimitExceeded {
                    line: index + 1,
                    observed: owners_on_line,
                    limit: MAX_OWNERS_PER_RULE,
                });
            }

            owner_token_count += 1;
            if owner_token_count > MAX_CODEOWNERS_OWNER_TOKENS {
                return Err(CodeownersBlocker::OwnerTokenLimitExceeded {
                    observed: owner_token_count,
                    limit: MAX_CODEOWNERS_OWNER_TOKENS,
                });
            }
        }
    }

    Ok(())
}

fn validate_policy_counts(
    rule_owner_counts: impl IntoIterator<Item = (usize, usize)>,
) -> Result<(), CodeownersBlocker> {
    let mut rule_count = 0_usize;
    let mut owner_token_count = 0_usize;
    for (line, owner_count) in rule_owner_counts {
        rule_count += 1;
        if rule_count > MAX_CODEOWNERS_RULES {
            return Err(CodeownersBlocker::RuleLimitExceeded {
                observed: rule_count,
                limit: MAX_CODEOWNERS_RULES,
            });
        }
        if owner_count > MAX_OWNERS_PER_RULE {
            return Err(CodeownersBlocker::OwnersPerRuleLimitExceeded {
                line,
                observed: owner_count,
                limit: MAX_OWNERS_PER_RULE,
            });
        }
        owner_token_count += owner_count;
        if owner_token_count > MAX_CODEOWNERS_OWNER_TOKENS {
            return Err(CodeownersBlocker::OwnerTokenLimitExceeded {
                observed: owner_token_count,
                limit: MAX_CODEOWNERS_OWNER_TOKENS,
            });
        }
    }
    Ok(())
}

fn line_blocker(error: &ParseError) -> CodeownersLineBlocker {
    let reason = match &error.kind {
        ErrorKind::BadPattern(error) => CodeownersLineBlockerKind::InvalidPattern {
            message: error.to_string(),
        },
        ErrorKind::BadOwner(token) => CodeownersLineBlockerKind::InvalidOwner {
            token: token.clone(),
        },
        other => CodeownersLineBlockerKind::UnsupportedParserError {
            message: other.to_string(),
        },
    };
    CodeownersLineBlocker {
        line: error.line,
        text: error.text.clone(),
        reason,
    }
}

fn unsupported_escape_blockers(text: &str) -> Vec<CodeownersLineBlocker> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            trimmed.starts_with(r"\#").then(|| CodeownersLineBlocker {
                line: index + 1,
                text: trimmed.to_owned(),
                reason: CodeownersLineBlockerKind::InvalidPattern {
                    message: "escaping a leading # is not supported in CODEOWNERS".to_owned(),
                },
            })
        })
        .collect()
}

fn validate_repository_path(path: &str) -> Result<(), CodeownersBlocker> {
    let reason = if path.is_empty() {
        Some("path must not be empty")
    } else if path.starts_with('/') {
        Some("path must not be absolute")
    } else if path.ends_with('/') {
        Some("path must name a file, not a directory")
    } else if path.contains('\0') {
        Some("path must not contain NUL")
    } else if path
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        Some("path contains an empty, current-directory, or parent-directory component")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(CodeownersBlocker::InvalidRepositoryPath {
            path: path.to_owned(),
            reason: reason.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug)]
struct TreeEntry {
    mode: String,
    object_type: String,
    oid: String,
    path: String,
}

fn find_source_entry(
    repository: &Path,
    base_commit: &str,
) -> Result<Option<TreeEntry>, CodeownersBlocker> {
    for path in SEARCH_PATHS {
        let operation = format!("ls-tree {base_commit} -- {path}");
        let output = run_git_bounded(
            repository,
            &["ls-tree", "-z", "--full-tree", base_commit, "--", path],
            MAX_GIT_METADATA_BYTES,
            &operation,
        )?;
        if output.is_empty() {
            continue;
        }
        return parse_tree_entry(&output, base_commit, path, &operation).map(Some);
    }
    Ok(None)
}

fn parse_tree_entry(
    output: &[u8],
    base_commit: &str,
    expected_path: &str,
    operation: &str,
) -> Result<TreeEntry, CodeownersBlocker> {
    let Some(record) = output.strip_suffix(&[0]) else {
        return malformed(operation, "ls-tree record is not NUL terminated");
    };
    if record.contains(&0) {
        return malformed(operation, "ls-tree returned more than one record");
    }
    let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
        return malformed(operation, "ls-tree record has no path separator");
    };
    let header = std::str::from_utf8(&record[..tab])
        .map_err(|_| malformed_value(operation, "ls-tree header is not UTF-8"))?;
    let path = std::str::from_utf8(&record[tab + 1..])
        .map_err(|_| malformed_value(operation, "selected CODEOWNERS path is not UTF-8"))?;
    if path != expected_path {
        return malformed(
            operation,
            &format!("expected path {expected_path:?}, found {path:?}"),
        );
    }
    let columns: Vec<_> = header.split_ascii_whitespace().collect();
    if columns.len() != 3 {
        return malformed(
            operation,
            &format!("expected three metadata fields, found {}", columns.len()),
        );
    }
    if !is_object_id(columns[2]) || columns[2].len() != base_commit.len() {
        return malformed(operation, "tree entry contains an invalid object ID");
    }
    Ok(TreeEntry {
        mode: columns[0].to_owned(),
        object_type: columns[1].to_owned(),
        oid: columns[2].to_ascii_lowercase(),
        path: path.to_owned(),
    })
}

fn blob_size(repository: &Path, oid: &str) -> Result<u64, CodeownersBlocker> {
    let operation = format!("cat-file -s {oid}");
    let output = run_git_bounded(repository, &["cat-file", "-s", oid], 128, &operation)?;
    let text = std::str::from_utf8(&output)
        .map_err(|_| malformed_value(&operation, "blob size is not UTF-8"))?;
    let text = trim_line_ending(text);
    text.parse::<u64>()
        .map_err(|_| malformed_value(&operation, "blob size is not an unsigned integer"))
}

fn verify_commit(repository: &Path, base_commit: &str) -> Result<(), CodeownersBlocker> {
    let operation = format!("cat-file -t {base_commit}");
    let output = run_git_bounded(repository, &["cat-file", "-t", base_commit], 64, &operation)?;
    let object_type = std::str::from_utf8(&output)
        .map_err(|_| malformed_value(&operation, "object type is not UTF-8"))?;
    let object_type = trim_line_ending(object_type);
    if object_type != "commit" {
        return Err(CodeownersBlocker::BaseObjectIsNotCommit {
            base_commit: base_commit.to_owned(),
            object_type: object_type.to_owned(),
        });
    }
    Ok(())
}

fn verify_full_object_id(repository: &Path, base_commit: &str) -> Result<(), CodeownersBlocker> {
    let operation = format!("rev-parse --verify {base_commit}");
    let output = run_git_bounded(
        repository,
        &["rev-parse", "--verify", "--end-of-options", base_commit],
        128,
        &operation,
    )?;
    let resolved = std::str::from_utf8(&output)
        .map_err(|_| malformed_value(&operation, "resolved object ID is not UTF-8"))?;
    let resolved = trim_line_ending(resolved);
    if !is_object_id(resolved) {
        return malformed(&operation, "resolved object ID is invalid");
    }
    if resolved.to_ascii_lowercase() != base_commit {
        return Err(CodeownersBlocker::InvalidBaseCommit {
            value: base_commit.to_owned(),
        });
    }
    Ok(())
}

fn validate_exact_object_id(value: &str) -> Result<String, CodeownersBlocker> {
    if !is_object_id(value) {
        return Err(CodeownersBlocker::InvalidBaseCommit {
            value: value.to_owned(),
        });
    }
    Ok(value.to_ascii_lowercase())
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn trim_line_ending(value: &str) -> &str {
    value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value)
}

fn malformed<T>(operation: &str, message: &str) -> Result<T, CodeownersBlocker> {
    Err(malformed_value(operation, message))
}

fn malformed_value(operation: &str, message: &str) -> CodeownersBlocker {
    CodeownersBlocker::MalformedGitOutput {
        operation: operation.to_owned(),
        message: message.to_owned(),
    }
}

fn run_git_bounded(
    repository: &Path,
    arguments: &[&str],
    max_stdout_bytes: usize,
    operation: &str,
) -> Result<Vec<u8>, CodeownersBlocker> {
    let mut child = isolated_git_command(repository)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CodeownersBlocker::GitSpawn {
            operation: operation.to_owned(),
            message: error.to_string(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| malformed_value(operation, "failed to capture stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| malformed_value(operation, "failed to capture stderr"))?;
    let stdout_reader = std::thread::spawn(move || read_bounded(stdout, max_stdout_bytes));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr, MAX_GIT_STDERR_BYTES));
    let stdout = join_reader(stdout_reader, operation, "stdout")?;
    if stdout.exceeded {
        match child.kill() {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
            Err(error) => {
                return Err(CodeownersBlocker::GitSpawn {
                    operation: operation.to_owned(),
                    message: format!("failed to stop command after stdout overflow: {error}"),
                });
            }
        }
    }
    let status = child.wait().map_err(|error| CodeownersBlocker::GitSpawn {
        operation: operation.to_owned(),
        message: format!("failed to wait for command: {error}"),
    })?;
    let stderr = join_reader(stderr_reader, operation, "stderr")?;
    if stdout.exceeded {
        return Err(CodeownersBlocker::GitOutputLimit {
            operation: operation.to_owned(),
            stream: "stdout".to_owned(),
            limit: max_stdout_bytes,
        });
    }
    if stderr.exceeded {
        return Err(CodeownersBlocker::GitOutputLimit {
            operation: operation.to_owned(),
            stream: "stderr".to_owned(),
            limit: MAX_GIT_STDERR_BYTES,
        });
    }
    if !status.success() {
        return Err(CodeownersBlocker::GitFailed {
            operation: operation.to_owned(),
            status: status.to_string(),
            stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        });
    }
    Ok(stdout.bytes)
}

struct BoundedRead {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedRead> {
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(BoundedRead {
                bytes,
                exceeded: false,
            });
        }
        let remaining = limit.saturating_sub(bytes.len());
        let retained = remaining.min(count);
        bytes.extend_from_slice(&buffer[..retained]);
        if retained != count {
            return Ok(BoundedRead {
                bytes,
                exceeded: true,
            });
        }
    }
}

fn join_reader(
    reader: std::thread::JoinHandle<io::Result<BoundedRead>>,
    operation: &str,
    stream: &str,
) -> Result<BoundedRead, CodeownersBlocker> {
    reader
        .join()
        .map_err(|_| CodeownersBlocker::GitSpawn {
            operation: operation.to_owned(),
            message: format!("{stream} reader panicked"),
        })?
        .map_err(|error| CodeownersBlocker::GitSpawn {
            operation: operation.to_owned(),
            message: format!("failed to read {stream}: {error}"),
        })
}

fn isolated_git_command(repository: &Path) -> Command {
    let mut command = Command::new("git");
    for (name, _) in env::vars_os() {
        if unsafe_git_environment(&name) {
            command.env_remove(name);
        }
    }
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "USERPROFILE",
        "GIT_ASKPASS",
        "GIT_PROXY_COMMAND",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "SSH_ASKPASS",
        "SSH_ASKPASS_REQUIRE",
    ] {
        command.env_remove(name);
    }
    command
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", null_device())
        .env("GIT_GRAFT_FILE", "")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("--no-replace-objects")
        .arg("--literal-pathspecs")
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
    use super::{
        CodeownersBlocker, MAX_CODEOWNERS_OWNER_TOKENS, MAX_CODEOWNERS_RULES, MAX_OWNERS_PER_RULE,
        parse_policy_with, read_bounded, validate_exact_object_id, validate_policy_counts,
        validate_repository_path,
    };
    use codeowner::CodeOwners;
    use std::cell::Cell;

    #[test]
    fn bounded_reader_distinguishes_exact_limit_from_overflow() {
        let exact = read_bounded(std::io::Cursor::new(b"abc"), 3).unwrap();
        assert_eq!(exact.bytes, b"abc");
        assert!(!exact.exceeded);

        let overflow = read_bounded(std::io::Cursor::new(b"abcd"), 3).unwrap();
        assert_eq!(overflow.bytes, b"abc");
        assert!(overflow.exceeded);
    }

    #[test]
    fn revisions_must_be_full_object_ids() {
        assert!(validate_exact_object_id(&"a".repeat(40)).is_ok());
        assert!(validate_exact_object_id(&"B".repeat(64)).is_ok());
        assert!(validate_exact_object_id("HEAD").is_err());
        assert!(validate_exact_object_id(&"a".repeat(39)).is_err());
        assert!(validate_exact_object_id(&"g".repeat(40)).is_err());
    }

    #[test]
    fn repository_paths_are_canonical_and_file_like() {
        for valid in ["README.md", "docs/guide.md", "space dir/file.txt"] {
            validate_repository_path(valid).unwrap();
        }
        for invalid in ["", "/README.md", "docs/", "a//b", "./a", "a/../b"] {
            assert!(validate_repository_path(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn policy_limits_accept_exact_boundaries() {
        validate_policy_counts((1..=MAX_CODEOWNERS_RULES).map(|line| {
            let owners = usize::from(line <= MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE)
                * MAX_OWNERS_PER_RULE;
            (line, owners)
        }))
        .unwrap();
    }

    #[test]
    fn policy_limits_reject_one_over_each_boundary() {
        let rules = validate_policy_counts((1..=MAX_CODEOWNERS_RULES + 1).map(|line| (line, 0)))
            .unwrap_err();
        assert!(matches!(
            rules,
            CodeownersBlocker::RuleLimitExceeded {
                observed,
                limit: MAX_CODEOWNERS_RULES,
            } if observed == MAX_CODEOWNERS_RULES + 1
        ));

        let owners = validate_policy_counts([(7, MAX_OWNERS_PER_RULE + 1)]).unwrap_err();
        assert!(matches!(
            owners,
            CodeownersBlocker::OwnersPerRuleLimitExceeded {
                line: 7,
                observed,
                limit: MAX_OWNERS_PER_RULE,
            } if observed == MAX_OWNERS_PER_RULE + 1
        ));

        let owner_tokens = validate_policy_counts(
            (1..=MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE)
                .map(|line| (line, MAX_OWNERS_PER_RULE))
                .chain(std::iter::once((
                    MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE + 1,
                    1,
                ))),
        )
        .unwrap_err();
        assert!(matches!(
            owner_tokens,
            CodeownersBlocker::OwnerTokenLimitExceeded {
                observed,
                limit: MAX_CODEOWNERS_OWNER_TOKENS,
            } if observed == MAX_CODEOWNERS_OWNER_TOKENS + 1
        ));
    }

    #[test]
    fn lexical_preflight_accepts_exact_boundaries_with_codeowners_comments_and_whitespace() {
        let owners = std::iter::repeat_n("@owner", MAX_OWNERS_PER_RULE)
            .collect::<Vec<_>>()
            .join("\t");
        let owner_line = format!("  *\t{owners} # ignored @comment-owner\r\n");
        let mut text = owner_line.repeat(MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE);
        text.push_str(
            &" * # an owner-looking comment does not count\n"
                .repeat(MAX_CODEOWNERS_RULES - MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE),
        );

        let called = Cell::new(false);
        parse_policy_with(&text, |text| {
            called.set(true);
            CodeOwners::parse(text)
        })
        .unwrap();
        assert!(called.get());
    }

    #[test]
    fn lexical_preflight_rejects_rule_limit_before_calling_parser() {
        let text = "* # comment\n".repeat(MAX_CODEOWNERS_RULES + 1);
        let called = Cell::new(false);
        let error = parse_policy_with(&text, |_| {
            called.set(true);
            CodeOwners::default()
        })
        .unwrap_err();

        assert!(!called.get());
        assert!(matches!(
            error,
            CodeownersBlocker::RuleLimitExceeded {
                observed,
                limit: MAX_CODEOWNERS_RULES,
            } if observed == MAX_CODEOWNERS_RULES + 1
        ));
    }

    #[test]
    fn lexical_preflight_rejects_per_rule_owner_limit_before_calling_parser() {
        let owners = std::iter::repeat_n("@owner", MAX_OWNERS_PER_RULE + 1)
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!("# ignored @owner\n  * {owners} # ignored @owner\n");
        let called = Cell::new(false);
        let error = parse_policy_with(&text, |_| {
            called.set(true);
            CodeOwners::default()
        })
        .unwrap_err();

        assert!(!called.get());
        assert!(matches!(
            error,
            CodeownersBlocker::OwnersPerRuleLimitExceeded {
                line: 2,
                observed,
                limit: MAX_OWNERS_PER_RULE,
            } if observed == MAX_OWNERS_PER_RULE + 1
        ));
    }

    #[test]
    fn lexical_preflight_rejects_total_owner_limit_before_calling_parser() {
        let owners = std::iter::repeat_n("@owner", MAX_OWNERS_PER_RULE)
            .collect::<Vec<_>>()
            .join(" ");
        let full_lines = MAX_CODEOWNERS_OWNER_TOKENS / MAX_OWNERS_PER_RULE;
        let mut text = format!("* {owners}\n").repeat(full_lines);
        text.push_str("* @owner\n");
        let called = Cell::new(false);
        let error = parse_policy_with(&text, |_| {
            called.set(true);
            CodeOwners::default()
        })
        .unwrap_err();

        assert!(!called.get());
        assert!(matches!(
            error,
            CodeownersBlocker::OwnerTokenLimitExceeded {
                observed,
                limit: MAX_CODEOWNERS_OWNER_TOKENS,
            } if observed == MAX_CODEOWNERS_OWNER_TOKENS + 1
        ));
    }
}
