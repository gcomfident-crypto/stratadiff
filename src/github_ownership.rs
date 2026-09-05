use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, de::DeserializeOwned};
use tempfile::Builder;

use crate::{
    codeowners::CodeownerIdentity,
    ownership::{
        GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, GithubMembershipState, GithubOwnershipSnapshot,
        GithubOwnershipTeam, GithubOwnershipUser, GithubTeamMembership, GithubTeamPrivacy,
        MAX_OWNERSHIP_SNAPSHOT_BYTES, MAX_OWNERSHIP_TEAMS, MAX_OWNERSHIP_USERS, MAX_TEAM_MEMBERS,
        MAX_TOTAL_TEAM_MEMBERSHIPS, RepositoryPermission, github_provider_hostname,
    },
};

pub const GITHUB_API_VERSION: &str = "2022-11-28";
pub const MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_GITHUB_OWNERSHIP_API_REQUESTS: usize = 5_000;

const GITHUB_PAGE_SIZE: usize = 100;
const MAX_GITHUB_LINK_HEADER_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubOwnershipMediaType {
    Json,
    TeamRepository,
}

impl GithubOwnershipMediaType {
    pub fn accept_header(self) -> &'static str {
        match self {
            Self::Json => "application/vnd.github+json",
            Self::TeamRepository => "application/vnd.github.v3.repository+json",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubOwnershipApiResponse {
    pub body: Vec<u8>,
    pub link_header: Option<String>,
}

pub trait GithubOwnershipApi {
    fn get(
        &mut self,
        endpoint: &str,
        media_type: GithubOwnershipMediaType,
    ) -> Result<GithubOwnershipApiResponse>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CollectionPlan {
    direct_users: BTreeSet<String>,
    teams: BTreeSet<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepositoryCoordinates {
    owner: String,
    name: String,
    encoded_owner: String,
    encoded_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnershipObservation {
    repository_id: u64,
    users: Vec<GithubOwnershipUser>,
    teams: Vec<GithubOwnershipTeam>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiRepository {
    id: u64,
    full_name: String,
    html_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiCommit {
    sha: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiPermissionBits {
    pull: bool,
    triage: Option<bool>,
    push: bool,
    maintain: Option<bool>,
    admin: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiUser {
    id: u64,
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiUserPermission {
    permission: String,
    role_name: String,
    user: ApiUser,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiOrganization {
    login: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiTeam {
    id: u64,
    slug: String,
    privacy: GithubTeamPrivacy,
    organization: ApiOrganization,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiTeamRepository {
    id: u64,
    full_name: String,
    role_name: String,
    permissions: ApiPermissionBits,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiTeamMember {
    id: u64,
    login: String,
    #[serde(rename = "type")]
    kind: String,
    role: String,
    inherited: bool,
}

#[derive(Debug)]
struct ApiBudget {
    requests: usize,
    response_bytes: usize,
}

impl ApiBudget {
    fn new() -> Self {
        Self {
            requests: 0,
            response_bytes: 0,
        }
    }

    fn get<A: GithubOwnershipApi>(
        &mut self,
        api: &mut A,
        endpoint: &str,
        media_type: GithubOwnershipMediaType,
    ) -> Result<GithubOwnershipApiResponse> {
        self.requests = self
            .requests
            .checked_add(1)
            .context("GitHub ownership API request count overflow")?;
        ensure!(
            self.requests <= MAX_GITHUB_OWNERSHIP_API_REQUESTS,
            "GitHub ownership API request limit exceeded: observed {}, limit {MAX_GITHUB_OWNERSHIP_API_REQUESTS}",
            self.requests
        );
        let response = api
            .get(endpoint, media_type)
            .with_context(|| format!("GitHub ownership request failed for {endpoint}"))?;
        ensure!(
            response.body.len() <= MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES,
            "GitHub ownership response bytes limit exceeded for {endpoint}: observed {}, limit {MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES}",
            response.body.len()
        );
        if let Some(link_header) = &response.link_header {
            ensure!(
                link_header.len() <= MAX_GITHUB_LINK_HEADER_BYTES,
                "GitHub Link header bytes limit exceeded for {endpoint}: observed {}, limit {MAX_GITHUB_LINK_HEADER_BYTES}",
                link_header.len()
            );
        }
        self.response_bytes = self
            .response_bytes
            .checked_add(response.body.len())
            .context("GitHub ownership response byte count overflow")?;
        ensure!(
            self.response_bytes <= MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES,
            "GitHub ownership total response bytes limit exceeded: observed {}, limit {MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES}",
            self.response_bytes
        );
        Ok(response)
    }

    fn ensure_second_observation_fits(&self) -> Result<()> {
        ensure!(
            self.requests <= MAX_GITHUB_OWNERSHIP_API_REQUESTS / 2,
            "one GitHub ownership observation required {} requests, so the required second observation would exceed the {MAX_GITHUB_OWNERSHIP_API_REQUESTS}-request limit",
            self.requests
        );
        ensure!(
            self.response_bytes <= MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES / 2,
            "one GitHub ownership observation returned {} bytes, so the required second observation would exceed the {MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES}-byte limit",
            self.response_bytes
        );
        Ok(())
    }
}

pub fn collect_github_ownership_snapshot<A: GithubOwnershipApi>(
    provider_url: &str,
    repository: &str,
    base_commit: &str,
    identities: &[CodeownerIdentity],
    api: &mut A,
) -> Result<GithubOwnershipSnapshot> {
    github_provider_hostname(provider_url)?;
    ensure!(
        is_object_id(base_commit),
        "base commit must be a full lowercase Git object ID"
    );
    let coordinates = parse_repository(repository)?;
    let plan = CollectionPlan::new(identities)?;
    ensure!(
        plan.direct_users.len() <= MAX_OWNERSHIP_USERS,
        "direct CODEOWNER user count limit exceeded: observed {}, limit {MAX_OWNERSHIP_USERS}",
        plan.direct_users.len()
    );
    ensure!(
        plan.teams.len() <= MAX_OWNERSHIP_TEAMS,
        "CODEOWNER team count limit exceeded: observed {}, limit {MAX_OWNERSHIP_TEAMS}",
        plan.teams.len()
    );
    let minimum_requests = minimum_collection_requests(&plan)?;
    ensure!(
        minimum_requests <= MAX_GITHUB_OWNERSHIP_API_REQUESTS,
        "GitHub ownership collection requires at least {minimum_requests} requests for two observations, limit {MAX_GITHUB_OWNERSHIP_API_REQUESTS}"
    );

    let mut budget = ApiBudget::new();
    let first = collect_observation(
        &coordinates,
        provider_url,
        base_commit,
        &plan,
        api,
        &mut budget,
    )?;
    budget.ensure_second_observation_fits()?;
    let second = collect_observation(
        &coordinates,
        provider_url,
        base_commit,
        &plan,
        api,
        &mut budget,
    )?;
    ensure!(
        first == second,
        "GitHub ownership facts changed between two consecutive observations; no snapshot was written"
    );

    let snapshot = GithubOwnershipSnapshot {
        schema: GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA.to_owned(),
        provider_url: provider_url.to_owned(),
        repository_id: second.repository_id,
        base_commit: base_commit.to_owned(),
        api_version: GITHUB_API_VERSION.to_owned(),
        observed_at: current_utc_timestamp()?,
        users: second.users,
        teams: second.teams,
    };
    snapshot.validate()?;
    let encoded = serde_json::to_vec(&snapshot)?;
    ensure!(
        encoded.len() <= MAX_OWNERSHIP_SNAPSHOT_BYTES,
        "generated GitHub ownership snapshot exceeds the byte limit"
    );
    Ok(snapshot)
}

fn minimum_collection_requests(plan: &CollectionPlan) -> Result<usize> {
    let per_observation = plan
        .teams
        .len()
        .checked_mul(3)
        .and_then(|team_requests| team_requests.checked_add(plan.direct_users.len()))
        .and_then(|planned_requests| planned_requests.checked_add(4))
        .context("minimum GitHub ownership API request count overflow")?;
    per_observation
        .checked_mul(2)
        .context("minimum GitHub ownership API request count overflow")
}

pub fn write_github_ownership_snapshot(
    output: &Path,
    snapshot: &GithubOwnershipSnapshot,
) -> Result<()> {
    snapshot.validate()?;
    let encoded = serde_json::to_vec(snapshot)?;
    ensure!(
        encoded.len() <= MAX_OWNERSHIP_SNAPSHOT_BYTES,
        "generated GitHub ownership snapshot exceeds the byte limit"
    );

    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = Builder::new()
        .prefix(".stratadiff-ownership-")
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create a private temporary file in {}",
                parent.display()
            )
        })?;

    temporary.write_all(&encoded).with_context(|| {
        format!(
            "failed to write temporary ownership snapshot in {}",
            parent.display()
        )
    })?;
    temporary
        .flush()
        .context("failed to flush temporary ownership snapshot")?;
    temporary
        .as_file()
        .sync_all()
        .context("failed to sync temporary ownership snapshot")?;

    let mut written = Vec::with_capacity(encoded.len());
    File::open(temporary.path())
        .context("failed to reopen temporary ownership snapshot")?
        .take((MAX_OWNERSHIP_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut written)
        .context("failed to reread temporary ownership snapshot")?;
    ensure!(
        written.len() <= MAX_OWNERSHIP_SNAPSHOT_BYTES,
        "temporary GitHub ownership snapshot exceeds the byte limit"
    );
    let decoded: GithubOwnershipSnapshot = serde_json::from_slice(&written)
        .context("failed to decode temporary GitHub ownership snapshot")?;
    decoded.validate()?;
    ensure!(
        decoded == *snapshot,
        "temporary GitHub ownership snapshot changed while it was written"
    );

    temporary
        .persist(output)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", output.display()))?;
    #[cfg(unix)]
    File::open(parent)
        .with_context(|| format!("failed to open output directory {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync output directory {}", parent.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(output)
            .with_context(|| format!("failed to inspect {}", output.display()))?
            .permissions()
            .mode()
            & 0o777;
        ensure!(
            mode == 0o600,
            "ownership snapshot permissions are {mode:o}, expected 600"
        );
    }
    Ok(())
}

fn collect_observation<A: GithubOwnershipApi>(
    coordinates: &RepositoryCoordinates,
    provider_url: &str,
    base_commit: &str,
    plan: &CollectionPlan,
    api: &mut A,
    budget: &mut ApiBudget,
) -> Result<OwnershipObservation> {
    let repository_endpoint = coordinates.repository_endpoint();
    let repository: ApiRepository = get_json(
        api,
        budget,
        &repository_endpoint,
        GithubOwnershipMediaType::Json,
    )?;
    validate_repository_response(&repository, coordinates, provider_url)?;

    let commit_endpoint = format!(
        "{repository_endpoint}/git/commits/{}",
        encode_path_component(base_commit)
    );
    let commit: ApiCommit = get_json(
        api,
        budget,
        &commit_endpoint,
        GithubOwnershipMediaType::Json,
    )?;
    ensure!(
        commit.sha == base_commit,
        "GitHub base commit response resolved to {}, expected {base_commit}",
        commit.sha
    );

    let mut teams = Vec::with_capacity(plan.teams.len());
    let mut member_logins = BTreeMap::new();
    let mut member_ids_by_login = BTreeMap::new();
    let mut total_memberships = 0_usize;
    for (organization, slug) in &plan.teams {
        let encoded_organization = encode_path_component(organization);
        let encoded_slug = encode_path_component(slug);
        let team_endpoint = format!("orgs/{encoded_organization}/teams/{encoded_slug}");
        let team: ApiTeam = get_json(api, budget, &team_endpoint, GithubOwnershipMediaType::Json)?;
        ensure!(team.id != 0, "GitHub team ID must be nonzero");
        ensure!(
            team.organization.login.eq_ignore_ascii_case(organization)
                && team.slug.eq_ignore_ascii_case(slug),
            "GitHub team response identity {}/{} does not match requested {organization}/{slug}",
            team.organization.login,
            team.slug
        );
        ensure!(
            team.privacy == GithubTeamPrivacy::Closed,
            "GitHub CODEOWNER team @{organization}/{slug} is secret; no ownership snapshot was written"
        );

        let team_repository_endpoint = format!(
            "{team_endpoint}/repos/{}/{}",
            coordinates.encoded_owner, coordinates.encoded_name
        );
        let team_repository: ApiTeamRepository = get_json(
            api,
            budget,
            &team_repository_endpoint,
            GithubOwnershipMediaType::TeamRepository,
        )?;
        ensure!(
            team_repository.id == repository.id
                && team_repository
                    .full_name
                    .eq_ignore_ascii_case(&repository.full_name),
            "GitHub team repository response does not match {}",
            repository.full_name
        );
        let repository_permission = permission_from_bits(
            &team_repository.permissions,
            &team_repository.role_name,
            &format!("team @{organization}/{slug}"),
        )?;

        let members_endpoint = format!("{team_endpoint}/members?role=all");
        let raw_members = get_paginated::<ApiTeamMember, A>(
            api,
            budget,
            provider_url,
            &members_endpoint,
            MAX_TEAM_MEMBERS,
            &format!("team @{organization}/{slug} members"),
        )?;
        total_memberships = total_memberships
            .checked_add(raw_members.len())
            .context("total GitHub team membership count overflow")?;
        ensure!(
            total_memberships <= MAX_TOTAL_TEAM_MEMBERSHIPS,
            "total GitHub team membership count limit exceeded: observed {total_memberships}, limit {MAX_TOTAL_TEAM_MEMBERSHIPS}"
        );

        let mut members = Vec::with_capacity(raw_members.len());
        let mut team_member_ids = BTreeSet::new();
        for member in raw_members {
            ensure!(member.id != 0, "GitHub team member ID must be nonzero");
            ensure!(
                member.kind == "User",
                "GitHub team member @{} has unsupported type {}",
                member.login,
                member.kind
            );
            ensure!(
                matches!(member.role.as_str(), "member" | "maintainer"),
                "GitHub team member @{} has unsupported team role {}",
                member.login,
                member.role
            );
            ensure!(
                team_member_ids.insert(member.id),
                "GitHub team @{organization}/{slug} returned duplicate member ID {}",
                member.id
            );
            let normalized_login = normalize_identity(&member.login, "team member login")?;
            if let Some(previous) = member_logins.insert(member.id, normalized_login.clone()) {
                ensure!(
                    previous == normalized_login,
                    "GitHub user ID {} had conflicting logins across team responses",
                    member.id
                );
            }
            if let Some(previous_id) = member_ids_by_login.insert(normalized_login, member.id) {
                ensure!(
                    previous_id == member.id,
                    "GitHub login {} referred to conflicting user IDs {previous_id} and {} across team responses",
                    member.login,
                    member.id
                );
            }
            members.push(GithubTeamMembership {
                user_id: member.id,
                state: GithubMembershipState::Active,
                inherited: member.inherited,
            });
        }
        members.sort_by_key(|member| member.user_id);
        teams.push(GithubOwnershipTeam {
            id: team.id,
            organization_login: normalize_identity(
                &team.organization.login,
                "team organization login",
            )?,
            slug: normalize_identity(&team.slug, "team slug")?,
            privacy: team.privacy,
            repository_permission,
            members,
        });
    }

    let mut principal_logins = plan.direct_users.clone();
    principal_logins.extend(member_ids_by_login.keys().cloned());
    ensure!(
        principal_logins.len() <= MAX_OWNERSHIP_USERS,
        "GitHub ownership user count limit exceeded: observed {}, limit {MAX_OWNERSHIP_USERS}",
        principal_logins.len()
    );

    let mut users = Vec::with_capacity(principal_logins.len());
    let mut user_ids = BTreeSet::new();
    for requested_login in principal_logins {
        let endpoint = format!(
            "{repository_endpoint}/collaborators/{}/permission",
            encode_path_component(&requested_login)
        );
        let response: ApiUserPermission =
            get_json(api, budget, &endpoint, GithubOwnershipMediaType::Json)?;
        ensure!(response.user.id != 0, "GitHub user ID must be nonzero");
        ensure!(
            response.user.kind == "User",
            "GitHub user @{} has unsupported type {}",
            response.user.login,
            response.user.kind
        );
        let response_login = normalize_identity(&response.user.login, "GitHub user login")?;
        ensure!(
            response_login == requested_login,
            "GitHub permission response login {response_login} does not match requested {requested_login}"
        );
        if let Some(expected_id) = member_ids_by_login.get(&requested_login) {
            ensure!(
                response.user.id == *expected_id,
                "GitHub permission response for @{requested_login} returned user ID {}, but team membership returned {expected_id}",
                response.user.id
            );
        }
        ensure!(
            user_ids.insert(response.user.id),
            "GitHub permission responses contain duplicate user ID {}",
            response.user.id
        );
        users.push(GithubOwnershipUser {
            id: response.user.id,
            login: response_login,
            repository_permission: permission_from_user_response(
                &response.permission,
                &response.role_name,
                &format!("user @{}", response.user.login),
            )?,
        });
    }
    users.sort_by_key(|user| user.id);
    teams.sort_by_key(|team| team.id);

    let observation = OwnershipObservation {
        repository_id: repository.id,
        users,
        teams,
    };
    let final_repository: ApiRepository = get_json(
        api,
        budget,
        &repository_endpoint,
        GithubOwnershipMediaType::Json,
    )?;
    validate_repository_response(&final_repository, coordinates, provider_url)?;
    ensure!(
        final_repository.id == repository.id,
        "GitHub repository identity changed during one ownership observation"
    );
    let final_commit: ApiCommit = get_json(
        api,
        budget,
        &commit_endpoint,
        GithubOwnershipMediaType::Json,
    )?;
    ensure!(
        final_commit.sha == base_commit,
        "GitHub base commit response changed during one ownership observation"
    );
    let provisional = GithubOwnershipSnapshot {
        schema: GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA.to_owned(),
        provider_url: provider_url.to_owned(),
        repository_id: observation.repository_id,
        base_commit: base_commit.to_owned(),
        api_version: GITHUB_API_VERSION.to_owned(),
        observed_at: "1970-01-01T00:00:00Z".to_owned(),
        users: observation.users.clone(),
        teams: observation.teams.clone(),
    };
    provisional.validate()?;
    Ok(observation)
}

impl CollectionPlan {
    fn new(identities: &[CodeownerIdentity]) -> Result<Self> {
        let mut direct_users = BTreeSet::new();
        let mut teams = BTreeSet::new();
        for identity in identities {
            match identity {
                CodeownerIdentity::User { login } => {
                    direct_users.insert(normalize_identity(login, "CODEOWNER user login")?);
                }
                CodeownerIdentity::Team { organization, slug } => {
                    teams.insert((
                        normalize_identity(organization, "CODEOWNER team organization")?,
                        normalize_identity(slug, "CODEOWNER team slug")?,
                    ));
                }
                CodeownerIdentity::Email { address } => bail!(
                    "email CODEOWNER {address} cannot be collected as a stable GitHub reviewer identity"
                ),
            }
        }
        Ok(Self {
            direct_users,
            teams,
        })
    }
}

impl RepositoryCoordinates {
    fn repository_endpoint(&self) -> String {
        format!("repos/{}/{}", self.encoded_owner, self.encoded_name)
    }
}

fn parse_repository(repository: &str) -> Result<RepositoryCoordinates> {
    let (owner, name) = repository
        .split_once('/')
        .context("GitHub repository must use OWNER/REPO form")?;
    ensure!(
        !owner.is_empty() && !name.is_empty() && !name.contains('/'),
        "GitHub repository must use OWNER/REPO form"
    );
    let owner = normalize_identity(owner, "repository owner")?;
    let name = normalize_identity(name, "repository name")?;
    Ok(RepositoryCoordinates {
        encoded_owner: encode_path_component(&owner),
        encoded_name: encode_path_component(&name),
        owner,
        name,
    })
}

fn validate_repository_response(
    repository: &ApiRepository,
    coordinates: &RepositoryCoordinates,
    provider_url: &str,
) -> Result<()> {
    ensure!(repository.id != 0, "GitHub repository ID must be nonzero");
    let expected_full_name = format!("{}/{}", coordinates.owner, coordinates.name);
    ensure!(
        repository
            .full_name
            .eq_ignore_ascii_case(&expected_full_name),
        "GitHub repository response {} does not match requested {expected_full_name}",
        repository.full_name
    );
    let expected_html_url = format!("{provider_url}/{expected_full_name}");
    ensure!(
        repository.html_url.eq_ignore_ascii_case(&expected_html_url),
        "GitHub repository URL {} does not match {expected_html_url}",
        repository.html_url
    );
    Ok(())
}

fn permission_from_bits(
    permissions: &ApiPermissionBits,
    role_name: &str,
    label: &str,
) -> Result<RepositoryPermission> {
    ensure!(
        !role_name.is_empty() && role_name.trim() == role_name,
        "GitHub {label} returned an empty or malformed repository role"
    );
    ensure!(
        !permissions.admin || (permissions.push && permissions.pull),
        "GitHub {label} returned non-monotonic admin permission bits"
    );
    ensure!(
        permissions.maintain != Some(true) || (permissions.push && permissions.pull),
        "GitHub {label} returned non-monotonic maintain permission bits"
    );
    ensure!(
        !permissions.push || permissions.pull,
        "GitHub {label} returned non-monotonic write permission bits"
    );
    ensure!(
        permissions.triage != Some(true) || permissions.pull,
        "GitHub {label} returned non-monotonic triage permission bits"
    );

    let base_permission = if permissions.admin {
        RepositoryPermission::Admin
    } else if permissions.push {
        RepositoryPermission::Write
    } else if permissions.pull {
        RepositoryPermission::Read
    } else {
        bail!("GitHub {label} has no recognized repository permission")
    };

    let standard_role = match role_name.to_ascii_lowercase().as_str() {
        "admin" => Some(RepositoryPermission::Admin),
        "maintain" => Some(RepositoryPermission::Maintain),
        "write" | "push" => Some(RepositoryPermission::Write),
        "triage" => Some(RepositoryPermission::Triage),
        "read" | "pull" => Some(RepositoryPermission::Read),
        _ => None,
    };
    if let Some(standard_role) = standard_role {
        let expected_base = legacy_base_permission(standard_role);
        ensure!(
            expected_base == base_permission,
            "GitHub {label} returned conflicting role {role_name} and permission bits"
        );
        if standard_role == RepositoryPermission::Maintain {
            ensure!(
                permissions.maintain != Some(false),
                "GitHub {label} returned conflicting role {role_name} and maintain permission bit"
            );
        } else if standard_role == RepositoryPermission::Write {
            ensure!(
                permissions.maintain != Some(true),
                "GitHub {label} returned conflicting role {role_name} and maintain permission bit"
            );
        } else if standard_role == RepositoryPermission::Triage {
            ensure!(
                permissions.triage != Some(false),
                "GitHub {label} returned conflicting role {role_name} and triage permission bit"
            );
        } else if standard_role == RepositoryPermission::Read {
            ensure!(
                permissions.triage != Some(true),
                "GitHub {label} returned conflicting role {role_name} and triage permission bit"
            );
        }
        return Ok(standard_role);
    }

    Ok(match base_permission {
        RepositoryPermission::Admin => RepositoryPermission::Admin,
        RepositoryPermission::Write if permissions.maintain == Some(true) => {
            RepositoryPermission::Maintain
        }
        RepositoryPermission::Read if permissions.triage == Some(true) => {
            RepositoryPermission::Triage
        }
        permission => permission,
    })
}

fn legacy_base_permission(permission: RepositoryPermission) -> RepositoryPermission {
    match permission {
        RepositoryPermission::Admin => RepositoryPermission::Admin,
        RepositoryPermission::Maintain | RepositoryPermission::Write => RepositoryPermission::Write,
        RepositoryPermission::Triage | RepositoryPermission::Read => RepositoryPermission::Read,
    }
}

fn permission_from_user_response(
    permission: &str,
    role_name: &str,
    label: &str,
) -> Result<RepositoryPermission> {
    ensure!(
        !permission.is_empty() && permission.trim() == permission,
        "GitHub {label} returned an empty or malformed base permission"
    );
    ensure!(
        !role_name.is_empty() && role_name.trim() == role_name,
        "GitHub {label} returned an empty or malformed repository role"
    );

    let base_permission = match permission.to_ascii_lowercase().as_str() {
        "admin" => RepositoryPermission::Admin,
        "write" | "push" => RepositoryPermission::Write,
        "read" | "pull" => RepositoryPermission::Read,
        "none" => bail!("GitHub {label} is not an effective repository collaborator"),
        _ => bail!("GitHub {label} returned unsupported base permission {permission}"),
    };
    let role_permission = match role_name.to_ascii_lowercase().as_str() {
        "admin" => Some(RepositoryPermission::Admin),
        "maintain" => Some(RepositoryPermission::Maintain),
        "write" | "push" => Some(RepositoryPermission::Write),
        "triage" => Some(RepositoryPermission::Triage),
        "read" | "pull" => Some(RepositoryPermission::Read),
        _ => None,
    };
    if let Some(role_permission) = role_permission {
        let expected_base = legacy_base_permission(role_permission);
        ensure!(
            base_permission == expected_base,
            "GitHub {label} returned conflicting role {role_name} and base permission {permission}"
        );
        Ok(role_permission)
    } else {
        Ok(base_permission)
    }
}

fn get_json<T: DeserializeOwned, A: GithubOwnershipApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    endpoint: &str,
    media_type: GithubOwnershipMediaType,
) -> Result<T> {
    let response = budget.get(api, endpoint, media_type)?;
    ensure!(
        response.link_header.is_none(),
        "GitHub ownership response for non-paginated endpoint {endpoint} contained a Link header"
    );
    ensure!(
        !response.body.is_empty(),
        "GitHub ownership response for {endpoint} is empty"
    );
    serde_json::from_slice(&response.body)
        .with_context(|| format!("failed to decode GitHub ownership response for {endpoint}"))
}

fn get_paginated<T: DeserializeOwned, A: GithubOwnershipApi>(
    api: &mut A,
    budget: &mut ApiBudget,
    provider_url: &str,
    endpoint: &str,
    max_items: usize,
    label: &str,
) -> Result<Vec<T>> {
    let mut items = Vec::new();
    let max_pages = max_items.div_ceil(GITHUB_PAGE_SIZE);
    let api_origin = github_api_origin(provider_url);
    for page_number in 1..=max_pages + 1 {
        let separator = if endpoint.contains('?') { '&' } else { '?' };
        let page_endpoint =
            format!("{endpoint}{separator}per_page={GITHUB_PAGE_SIZE}&page={page_number}");
        let response = budget.get(api, &page_endpoint, GithubOwnershipMediaType::Json)?;
        ensure!(
            !response.body.is_empty(),
            "GitHub ownership response for {page_endpoint} is empty"
        );
        let page: Vec<T> = serde_json::from_slice(&response.body).with_context(|| {
            format!("failed to decode GitHub ownership response for {page_endpoint}")
        })?;
        ensure!(
            page.len() <= GITHUB_PAGE_SIZE,
            "GitHub {label} page {page_number} returned {} items, limit {GITHUB_PAGE_SIZE}",
            page.len()
        );
        let has_next = validate_next_link(
            response.link_header.as_deref(),
            &api_origin,
            endpoint,
            page_number,
        )?;
        ensure!(
            page.len() == GITHUB_PAGE_SIZE || !has_next,
            "GitHub {label} page {page_number} advertised a next page after returning only {} items",
            page.len()
        );
        let next_len = items
            .len()
            .checked_add(page.len())
            .context("GitHub paginated item count overflow")?;
        ensure!(
            next_len <= max_items,
            "GitHub {label} count limit exceeded: observed at least {next_len}, limit {max_items}"
        );
        let page_len = page.len();
        items.extend(page);
        if page_len < GITHUB_PAGE_SIZE {
            return Ok(items);
        }
    }
    unreachable!("the limit-plus-one page must return or exceed the item limit")
}

fn github_api_origin(provider_url: &str) -> String {
    if provider_url == "https://github.com" {
        "https://api.github.com".to_owned()
    } else {
        format!("{provider_url}/api/v3")
    }
}

fn validate_next_link(
    link_header: Option<&str>,
    api_origin: &str,
    endpoint: &str,
    page_number: usize,
) -> Result<bool> {
    let Some(link_header) = link_header else {
        return Ok(false);
    };
    ensure!(
        !link_header.is_empty()
            && !link_header.contains(['\r', '\n'])
            && link_header.trim() == link_header,
        "GitHub pagination Link header is empty or malformed"
    );

    let mut next_target = None;
    for raw_link in link_header.split(',') {
        let raw_link = raw_link.trim();
        let target_end = raw_link
            .find('>')
            .context("GitHub pagination Link entry is missing closing angle bracket")?;
        let target = raw_link
            .strip_prefix('<')
            .context("GitHub pagination Link entry is missing opening angle bracket")?
            .get(..target_end - 1)
            .context("GitHub pagination Link entry has an invalid target")?;
        ensure!(!target.is_empty(), "GitHub pagination Link target is empty");

        let parameters = &raw_link[target_end + 1..];
        let mut relations = None;
        for parameter in parameters
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (name, value) = parameter
                .split_once('=')
                .context("GitHub pagination Link parameter is malformed")?;
            if name.eq_ignore_ascii_case("rel") {
                ensure!(
                    relations.is_none(),
                    "GitHub pagination Link entry repeats rel"
                );
                let value = value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .context("GitHub pagination Link rel must be quoted")?;
                relations = Some(value);
            }
        }
        if relations
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .any(|relation| relation.eq_ignore_ascii_case("next"))
        {
            ensure!(
                next_target.replace(target).is_none(),
                "GitHub pagination Link header contains multiple next relations"
            );
        }
    }

    if let Some(next_target) = next_target {
        validate_next_target(next_target, api_origin, endpoint, page_number + 1)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn validate_next_target(
    target: &str,
    api_origin: &str,
    endpoint: &str,
    expected_page: usize,
) -> Result<()> {
    let prefix = format!("{api_origin}/");
    let relative = target
        .strip_prefix(&prefix)
        .context("GitHub pagination next link changed API origin")?;
    ensure!(
        !relative.contains('#'),
        "GitHub pagination next link contains a fragment"
    );
    let (target_path, target_query) = relative
        .split_once('?')
        .context("GitHub pagination next link is missing its query")?;
    let (expected_path, base_query) = endpoint.split_once('?').unwrap_or((endpoint, ""));
    ensure!(
        target_path == expected_path,
        "GitHub pagination next link changed endpoint path"
    );

    let mut expected_query = parse_query(base_query)?;
    ensure!(
        expected_query
            .insert("per_page".to_owned(), GITHUB_PAGE_SIZE.to_string())
            .is_none(),
        "GitHub paginated endpoint already contains per_page"
    );
    ensure!(
        expected_query
            .insert("page".to_owned(), expected_page.to_string())
            .is_none(),
        "GitHub paginated endpoint already contains page"
    );
    let actual_query = parse_query(target_query)?;
    ensure!(
        actual_query == expected_query,
        "GitHub pagination next link changed the expected query"
    );
    Ok(())
}

fn parse_query(query: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    if query.is_empty() {
        return Ok(values);
    }
    for pair in query.split('&') {
        let (name, value) = pair
            .split_once('=')
            .context("GitHub pagination query parameter is malformed")?;
        ensure!(
            !name.is_empty() && !value.is_empty(),
            "GitHub pagination query parameter is empty"
        );
        ensure!(
            values.insert(name.to_owned(), value.to_owned()).is_none(),
            "GitHub pagination query repeats parameter {name}"
        );
    }
    Ok(values)
}

fn normalize_identity(value: &str, label: &str) -> Result<String> {
    ensure!(
        !value.is_empty()
            && value.len() <= 100
            && value.trim() == value
            && value.bytes().all(|byte| {
                byte.is_ascii() && !byte.is_ascii_control() && !byte.is_ascii_whitespace()
            })
            && !value.contains(['@', '/']),
        "{label} is empty, malformed, or exceeds its byte limit"
    );
    Ok(value.to_ascii_lowercase())
}

fn encode_path_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn current_utc_timestamp() -> Result<String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let days = i64::try_from(seconds / 86_400).context("system time exceeds supported range")?;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_date_from_unix_days(days)?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

// Howard Hinnant's civil-from-days transform avoids locale and external clock tools.
fn civil_date_from_unix_days(days: i64) -> Result<(i64, i64, i64)> {
    let shifted = days
        .checked_add(719_468)
        .context("system time exceeds supported civil-date range")?;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    ensure!(
        (0..=9_999).contains(&year),
        "system time is outside the four-digit UTC year range"
    );
    Ok((year, month, day))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        ApiBudget, CollectionPlan, GithubOwnershipApi, GithubOwnershipApiResponse,
        GithubOwnershipMediaType, MAX_GITHUB_LINK_HEADER_BYTES, MAX_GITHUB_OWNERSHIP_API_REQUESTS,
        MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES, MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES,
        civil_date_from_unix_days, encode_path_component, minimum_collection_requests,
        permission_from_bits, permission_from_user_response,
    };
    use crate::{github_ownership::ApiPermissionBits, ownership::RepositoryPermission};

    struct OneResponseApi(Option<GithubOwnershipApiResponse>);

    impl GithubOwnershipApi for OneResponseApi {
        fn get(
            &mut self,
            _endpoint: &str,
            _media_type: GithubOwnershipMediaType,
        ) -> anyhow::Result<GithubOwnershipApiResponse> {
            Ok(self.0.take().unwrap())
        }
    }

    #[test]
    fn civil_date_conversion_covers_epoch_and_leap_boundaries() {
        assert_eq!(civil_date_from_unix_days(0).unwrap(), (1970, 1, 1));
        assert_eq!(civil_date_from_unix_days(11_016).unwrap(), (2000, 2, 29));
        assert_eq!(civil_date_from_unix_days(20_453).unwrap(), (2025, 12, 31));
    }

    #[test]
    fn path_components_are_encoded_without_query_injection() {
        assert_eq!(encode_path_component("team?role=all"), "team%3Frole%3Dall");
    }

    #[test]
    fn permission_bits_and_standard_role_must_agree() {
        let write = ApiPermissionBits {
            pull: true,
            triage: Some(true),
            push: true,
            maintain: Some(false),
            admin: false,
        };
        assert_eq!(
            permission_from_bits(&write, "write", "fixture").unwrap(),
            RepositoryPermission::Write
        );
        assert!(permission_from_bits(&write, "admin", "fixture").is_err());
        assert_eq!(
            permission_from_bits(&write, "custom-reviewer", "fixture").unwrap(),
            RepositoryPermission::Write
        );

        let legacy_maintain = ApiPermissionBits {
            pull: true,
            triage: None,
            push: true,
            maintain: None,
            admin: false,
        };
        assert_eq!(
            permission_from_bits(&legacy_maintain, "maintain", "fixture").unwrap(),
            RepositoryPermission::Maintain
        );

        let custom_admin = ApiPermissionBits {
            pull: true,
            triage: None,
            push: true,
            maintain: None,
            admin: true,
        };
        assert_eq!(
            permission_from_bits(&custom_admin, "security-admin", "fixture").unwrap(),
            RepositoryPermission::Admin
        );
    }

    #[test]
    fn user_permission_preserves_standard_role_and_checks_legacy_base() {
        assert_eq!(
            permission_from_user_response("write", "maintain", "fixture").unwrap(),
            RepositoryPermission::Maintain
        );
        assert_eq!(
            permission_from_user_response("read", "triage", "fixture").unwrap(),
            RepositoryPermission::Triage
        );
        assert_eq!(
            permission_from_user_response("write", "custom-reviewer", "fixture").unwrap(),
            RepositoryPermission::Write
        );
        assert!(permission_from_user_response("read", "write", "fixture").is_err());
        assert!(permission_from_user_response("none", "none", "fixture").is_err());
    }

    #[test]
    fn api_budget_accepts_exact_response_and_link_limits() {
        let mut budget = ApiBudget::new();
        let mut api = OneResponseApi(Some(GithubOwnershipApiResponse {
            body: vec![0; MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES],
            link_header: Some("x".repeat(MAX_GITHUB_LINK_HEADER_BYTES)),
        }));

        budget
            .get(&mut api, "fixture", GithubOwnershipMediaType::Json)
            .unwrap();

        assert_eq!(budget.requests, 1);
        assert_eq!(
            budget.response_bytes,
            MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES
        );
    }

    #[test]
    fn api_budget_rejects_response_and_link_limit_plus_one() {
        let mut response_budget = ApiBudget::new();
        let mut response_api = OneResponseApi(Some(GithubOwnershipApiResponse {
            body: vec![0; MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES + 1],
            link_header: None,
        }));
        assert!(
            response_budget
                .get(&mut response_api, "fixture", GithubOwnershipMediaType::Json)
                .is_err()
        );

        let mut link_budget = ApiBudget::new();
        let mut link_api = OneResponseApi(Some(GithubOwnershipApiResponse {
            body: Vec::new(),
            link_header: Some("x".repeat(MAX_GITHUB_LINK_HEADER_BYTES + 1)),
        }));
        assert!(
            link_budget
                .get(&mut link_api, "fixture", GithubOwnershipMediaType::Json)
                .is_err()
        );
    }

    #[test]
    fn repeatability_preflight_accepts_exact_half_budgets() {
        ApiBudget {
            requests: MAX_GITHUB_OWNERSHIP_API_REQUESTS / 2,
            response_bytes: MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES / 2,
        }
        .ensure_second_observation_fits()
        .unwrap();

        assert!(
            ApiBudget {
                requests: MAX_GITHUB_OWNERSHIP_API_REQUESTS / 2 + 1,
                response_bytes: 0,
            }
            .ensure_second_observation_fits()
            .is_err()
        );
        assert!(
            ApiBudget {
                requests: 0,
                response_bytes: MAX_GITHUB_OWNERSHIP_API_TOTAL_BYTES / 2 + 1,
            }
            .ensure_second_observation_fits()
            .is_err()
        );
    }

    #[test]
    fn static_request_preflight_accounts_for_two_complete_observations() {
        let direct_users = (0..2_496).map(|index| format!("user-{index}")).collect();
        let exact = CollectionPlan {
            direct_users,
            teams: BTreeSet::new(),
        };
        assert_eq!(
            minimum_collection_requests(&exact).unwrap(),
            MAX_GITHUB_OWNERSHIP_API_REQUESTS
        );

        let mut too_many = exact;
        too_many.direct_users.insert("one-more".to_owned());
        assert!(
            minimum_collection_requests(&too_many).unwrap() > MAX_GITHUB_OWNERSHIP_API_REQUESTS
        );
    }
}
