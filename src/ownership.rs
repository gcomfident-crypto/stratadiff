use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::codeowners::CodeownerIdentity;

pub const GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/github-ownership-snapshot-v1.schema.json";
pub const MAX_OWNERSHIP_USERS: usize = 10_000;
pub const MAX_OWNERSHIP_TEAMS: usize = 2_000;
pub const MAX_TEAM_MEMBERS: usize = 10_000;
pub const MAX_TOTAL_TEAM_MEMBERSHIPS: usize = 100_000;
pub const MAX_OWNERSHIP_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

const MAX_PROVIDER_URL_BYTES: usize = 2_048;
const MAX_IDENTITY_BYTES: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubOwnershipSnapshot {
    pub schema: String,
    pub provider_url: String,
    pub repository_id: u64,
    pub base_commit: String,
    pub api_version: String,
    pub observed_at: String,
    pub users: Vec<GithubOwnershipUser>,
    pub teams: Vec<GithubOwnershipTeam>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubOwnershipUser {
    pub id: u64,
    pub login: String,
    pub repository_permission: RepositoryPermission,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubOwnershipTeam {
    pub id: u64,
    pub organization_login: String,
    pub slug: String,
    pub privacy: GithubTeamPrivacy,
    pub repository_permission: RepositoryPermission,
    pub members: Vec<GithubTeamMembership>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubTeamMembership {
    pub user_id: u64,
    pub state: GithubMembershipState,
    pub inherited: bool,
}

#[derive(Debug)]
pub struct GithubOwnershipIndex<'a> {
    users_by_id: BTreeMap<u64, &'a GithubOwnershipUser>,
    users_by_login: BTreeMap<String, &'a GithubOwnershipUser>,
    teams_by_name: BTreeMap<(String, String), &'a GithubOwnershipTeam>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryPermission {
    Read,
    Triage,
    Write,
    Maintain,
    Admin,
}

impl RepositoryPermission {
    pub fn permits_review(self) -> bool {
        self >= Self::Write
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubTeamPrivacy {
    Closed,
    Secret,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubMembershipState {
    Active,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnershipBlocker {
    InvalidSnapshot {
        reason: String,
    },
    EmailCodeownerUnsupported {
        address: String,
    },
    UserNotFound {
        login: String,
    },
    TeamNotFound {
        organization: String,
        slug: String,
    },
    InsufficientRepositoryPermission {
        identity: CodeownerIdentity,
        permission: RepositoryPermission,
    },
    TeamNotVisible {
        organization: String,
        slug: String,
    },
    NoEligibleTeamMembers {
        organization: String,
        slug: String,
    },
}

impl fmt::Display for OwnershipBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSnapshot { reason } => {
                write!(formatter, "invalid GitHub ownership snapshot: {reason}")
            }
            Self::EmailCodeownerUnsupported { address } => write!(
                formatter,
                "email CODEOWNER {address} cannot be resolved to a stable GitHub reviewer identity"
            ),
            Self::UserNotFound { login } => {
                write!(
                    formatter,
                    "GitHub CODEOWNER user @{login} is missing from the snapshot"
                )
            }
            Self::TeamNotFound { organization, slug } => write!(
                formatter,
                "GitHub CODEOWNER team @{organization}/{slug} is missing from the snapshot"
            ),
            Self::InsufficientRepositoryPermission {
                identity,
                permission,
            } => write!(
                formatter,
                "GitHub CODEOWNER {identity} has {permission:?} repository permission; write or greater is required"
            ),
            Self::TeamNotVisible { organization, slug } => write!(
                formatter,
                "GitHub CODEOWNER team @{organization}/{slug} is secret and cannot be resolved as a visible CODEOWNERS team"
            ),
            Self::NoEligibleTeamMembers { organization, slug } => write!(
                formatter,
                "GitHub CODEOWNER team @{organization}/{slug} has no active member with write or greater repository permission"
            ),
        }
    }
}

impl Error for OwnershipBlocker {}

impl fmt::Display for CodeownerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User { login } => write!(formatter, "@{login}"),
            Self::Team { organization, slug } => write!(formatter, "@{organization}/{slug}"),
            Self::Email { address } => formatter.write_str(address),
        }
    }
}

impl GithubOwnershipSnapshot {
    pub fn validate(&self) -> Result<(), OwnershipBlocker> {
        require(
            self.schema == GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA,
            "unsupported schema",
        )?;
        validate_provider_url(&self.provider_url)?;
        require(self.repository_id != 0, "repository_id must be nonzero")?;
        require(
            is_object_id(&self.base_commit),
            "base_commit must be a full lowercase Git object ID",
        )?;
        require(
            valid_date(&self.api_version),
            "api_version must use YYYY-MM-DD",
        )?;
        require(
            valid_github_timestamp(&self.observed_at),
            "observed_at must use UTC YYYY-MM-DDTHH:MM:SSZ",
        )?;
        require(
            self.users.len() <= MAX_OWNERSHIP_USERS,
            format!(
                "user count limit exceeded: observed {}, limit {MAX_OWNERSHIP_USERS}",
                self.users.len()
            ),
        )?;
        require(
            self.teams.len() <= MAX_OWNERSHIP_TEAMS,
            format!(
                "team count limit exceeded: observed {}, limit {MAX_OWNERSHIP_TEAMS}",
                self.teams.len()
            ),
        )?;

        let mut user_ids = BTreeSet::new();
        let mut user_logins = BTreeSet::new();
        let mut previous_user_id = None;
        for user in &self.users {
            require(user.id != 0, "user id must be nonzero")?;
            validate_identity_component(&user.login, "user login")?;
            require(
                user_ids.insert(user.id),
                format!("duplicate user id {}", user.id),
            )?;
            require(
                user_logins.insert(user.login.to_ascii_lowercase()),
                format!("duplicate user login {}", user.login),
            )?;
            if let Some(previous) = previous_user_id {
                require(
                    previous < user.id,
                    "users must be strictly sorted by stable id",
                )?;
            }
            previous_user_id = Some(user.id);
        }

        let mut team_ids = BTreeSet::new();
        let mut team_names = BTreeSet::new();
        let mut previous_team_id = None;
        let mut total_memberships = 0_usize;
        for team in &self.teams {
            require(team.id != 0, "team id must be nonzero")?;
            validate_identity_component(&team.organization_login, "team organization login")?;
            validate_identity_component(&team.slug, "team slug")?;
            require(
                team_ids.insert(team.id),
                format!("duplicate team id {}", team.id),
            )?;
            let team_name = format!(
                "{}/{}",
                team.organization_login.to_ascii_lowercase(),
                team.slug.to_ascii_lowercase()
            );
            require(
                team_names.insert(team_name),
                format!(
                    "duplicate team identity {}/{}",
                    team.organization_login, team.slug
                ),
            )?;
            if let Some(previous) = previous_team_id {
                require(
                    previous < team.id,
                    "teams must be strictly sorted by stable id",
                )?;
            }
            previous_team_id = Some(team.id);

            require(
                team.members.len() <= MAX_TEAM_MEMBERS,
                format!(
                    "team {} member count limit exceeded: observed {}, limit {MAX_TEAM_MEMBERS}",
                    team.id,
                    team.members.len()
                ),
            )?;
            total_memberships = total_memberships
                .checked_add(team.members.len())
                .ok_or_else(|| invalid("total team membership count overflow"))?;
            require(
                total_memberships <= MAX_TOTAL_TEAM_MEMBERSHIPS,
                format!(
                    "total team membership count limit exceeded: observed {total_memberships}, limit {MAX_TOTAL_TEAM_MEMBERSHIPS}"
                ),
            )?;

            let mut member_ids = BTreeSet::new();
            let mut previous_member_id = None;
            for member in &team.members {
                require(member.user_id != 0, "team member user_id must be nonzero")?;
                require(
                    member_ids.insert(member.user_id),
                    format!(
                        "duplicate member {} in team {}/{}",
                        member.user_id, team.organization_login, team.slug
                    ),
                )?;
                if let Some(previous) = previous_member_id {
                    require(
                        previous < member.user_id,
                        format!(
                            "members of team {}/{} must be strictly sorted by stable user id",
                            team.organization_login, team.slug
                        ),
                    )?;
                }
                previous_member_id = Some(member.user_id);
                require(
                    user_ids.contains(&member.user_id),
                    format!(
                        "team {}/{} references missing user id {}",
                        team.organization_login, team.slug, member.user_id
                    ),
                )?;
            }
        }
        Ok(())
    }

    pub fn resolve(&self, identity: &CodeownerIdentity) -> Result<Vec<u64>, OwnershipBlocker> {
        self.index()?.resolve(identity)
    }

    pub fn index(&self) -> Result<GithubOwnershipIndex<'_>, OwnershipBlocker> {
        self.validate()?;
        Ok(GithubOwnershipIndex {
            users_by_id: self.users.iter().map(|user| (user.id, user)).collect(),
            users_by_login: self
                .users
                .iter()
                .map(|user| (user.login.to_ascii_lowercase(), user))
                .collect(),
            teams_by_name: self
                .teams
                .iter()
                .map(|team| {
                    (
                        (
                            team.organization_login.to_ascii_lowercase(),
                            team.slug.to_ascii_lowercase(),
                        ),
                        team,
                    )
                })
                .collect(),
        })
    }
}

impl GithubOwnershipIndex<'_> {
    pub fn user(&self, id: u64) -> Option<&GithubOwnershipUser> {
        self.users_by_id.get(&id).copied()
    }

    pub fn resolve(&self, identity: &CodeownerIdentity) -> Result<Vec<u64>, OwnershipBlocker> {
        match identity {
            CodeownerIdentity::Email { address } => {
                Err(OwnershipBlocker::EmailCodeownerUnsupported {
                    address: address.clone(),
                })
            }
            CodeownerIdentity::User { login } => {
                let user = self
                    .users_by_login
                    .get(&login.to_ascii_lowercase())
                    .copied()
                    .ok_or_else(|| OwnershipBlocker::UserNotFound {
                        login: login.clone(),
                    })?;
                if !user.repository_permission.permits_review() {
                    return Err(OwnershipBlocker::InsufficientRepositoryPermission {
                        identity: identity.clone(),
                        permission: user.repository_permission,
                    });
                }
                Ok(vec![user.id])
            }
            CodeownerIdentity::Team { organization, slug } => {
                let team = self
                    .teams_by_name
                    .get(&(organization.to_ascii_lowercase(), slug.to_ascii_lowercase()))
                    .copied()
                    .ok_or_else(|| OwnershipBlocker::TeamNotFound {
                        organization: organization.clone(),
                        slug: slug.clone(),
                    })?;
                if team.privacy != GithubTeamPrivacy::Closed {
                    return Err(OwnershipBlocker::TeamNotVisible {
                        organization: organization.clone(),
                        slug: slug.clone(),
                    });
                }
                if !team.repository_permission.permits_review() {
                    return Err(OwnershipBlocker::InsufficientRepositoryPermission {
                        identity: identity.clone(),
                        permission: team.repository_permission,
                    });
                }

                let candidates = team
                    .members
                    .iter()
                    .filter(|member| member.state == GithubMembershipState::Active)
                    .filter_map(|member| self.users_by_id.get(&member.user_id).copied())
                    .filter(|user| user.repository_permission.permits_review())
                    .map(|user| user.id)
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    return Err(OwnershipBlocker::NoEligibleTeamMembers {
                        organization: organization.clone(),
                        slug: slug.clone(),
                    });
                }
                Ok(candidates)
            }
        }
    }
}

fn invalid(reason: impl Into<String>) -> OwnershipBlocker {
    OwnershipBlocker::InvalidSnapshot {
        reason: reason.into(),
    }
}

fn require(condition: bool, reason: impl Into<String>) -> Result<(), OwnershipBlocker> {
    if condition {
        Ok(())
    } else {
        Err(invalid(reason))
    }
}

pub fn github_provider_hostname(value: &str) -> Result<&str, OwnershipBlocker> {
    require(
        !value.is_empty() && value.len() <= MAX_PROVIDER_URL_BYTES,
        "provider_url is empty or exceeds its byte limit",
    )?;
    let authority = value
        .strip_prefix("https://")
        .ok_or_else(|| invalid("provider_url must be a canonical HTTPS origin"))?;
    require(
        !authority.is_empty()
            && !authority.ends_with('/')
            && !authority.bytes().any(|byte| {
                byte.is_ascii_whitespace() || matches!(byte, b'/' | b'?' | b'#' | b'@')
            }),
        "provider_url must be a canonical HTTPS origin without credentials, path, query, or fragment",
    )?;
    require(
        value.bytes().all(|byte| !byte.is_ascii_uppercase()),
        "provider_url must be lowercase for deterministic provider binding",
    )?;
    Ok(authority)
}

fn validate_provider_url(value: &str) -> Result<(), OwnershipBlocker> {
    github_provider_hostname(value).map(|_| ())
}

fn validate_identity_component(value: &str, label: &str) -> Result<(), OwnershipBlocker> {
    require(
        !value.is_empty()
            && value.len() <= MAX_IDENTITY_BYTES
            && value.trim() == value
            && value.bytes().all(|byte| {
                byte.is_ascii() && !byte.is_ascii_control() && !byte.is_ascii_whitespace()
            })
            && !value.contains(['@', '/']),
        format!("{label} is empty, malformed, or exceeds its byte limit"),
    )
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn valid_github_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}
