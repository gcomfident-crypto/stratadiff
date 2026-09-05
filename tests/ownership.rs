use stratadiff::{
    codeowners::CodeownerIdentity,
    ownership::{
        GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, GithubMembershipState, GithubOwnershipSnapshot,
        GithubOwnershipTeam, GithubOwnershipUser, GithubTeamMembership, GithubTeamPrivacy,
        MAX_OWNERSHIP_USERS, OwnershipBlocker, RepositoryPermission,
    },
};

fn snapshot() -> GithubOwnershipSnapshot {
    GithubOwnershipSnapshot {
        schema: GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA.to_owned(),
        provider_url: "https://github.com".to_owned(),
        repository_id: 4_242,
        base_commit: "a".repeat(40),
        api_version: "2022-11-28".to_owned(),
        observed_at: "2026-09-05T12:34:56Z".to_owned(),
        users: vec![
            GithubOwnershipUser {
                id: 11,
                login: "alice".to_owned(),
                repository_permission: RepositoryPermission::Write,
            },
            GithubOwnershipUser {
                id: 12,
                login: "bob".to_owned(),
                repository_permission: RepositoryPermission::Maintain,
            },
            GithubOwnershipUser {
                id: 13,
                login: "carol".to_owned(),
                repository_permission: RepositoryPermission::Admin,
            },
            GithubOwnershipUser {
                id: 14,
                login: "dave".to_owned(),
                repository_permission: RepositoryPermission::Read,
            },
            GithubOwnershipUser {
                id: 15,
                login: "erin".to_owned(),
                repository_permission: RepositoryPermission::Triage,
            },
        ],
        teams: vec![
            GithubOwnershipTeam {
                id: 21,
                organization_login: "acme".to_owned(),
                slug: "payments".to_owned(),
                privacy: GithubTeamPrivacy::Closed,
                repository_permission: RepositoryPermission::Write,
                members: vec![
                    GithubTeamMembership {
                        user_id: 11,
                        state: GithubMembershipState::Active,
                        inherited: false,
                    },
                    GithubTeamMembership {
                        user_id: 12,
                        state: GithubMembershipState::Active,
                        inherited: true,
                    },
                    GithubTeamMembership {
                        user_id: 14,
                        state: GithubMembershipState::Pending,
                        inherited: false,
                    },
                ],
            },
            GithubOwnershipTeam {
                id: 22,
                organization_login: "acme".to_owned(),
                slug: "security".to_owned(),
                privacy: GithubTeamPrivacy::Secret,
                repository_permission: RepositoryPermission::Admin,
                members: vec![GithubTeamMembership {
                    user_id: 13,
                    state: GithubMembershipState::Active,
                    inherited: false,
                }],
            },
        ],
    }
}

fn user(login: &str) -> CodeownerIdentity {
    CodeownerIdentity::User {
        login: login.to_owned(),
    }
}

fn team(slug: &str) -> CodeownerIdentity {
    CodeownerIdentity::Team {
        organization: "acme".to_owned(),
        slug: slug.to_owned(),
    }
}

#[test]
fn t1_valid_snapshot_resolves_a_direct_user_by_stable_id() {
    let snapshot = snapshot();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.resolve(&user("ALICE")).unwrap(), vec![11]);
}

#[test]
fn t2_direct_users_require_write_or_greater_permission() {
    let snapshot = snapshot();
    for (login, expected) in [("alice", 11), ("bob", 12), ("carol", 13)] {
        assert_eq!(snapshot.resolve(&user(login)).unwrap(), vec![expected]);
    }
    for (login, permission) in [
        ("dave", RepositoryPermission::Read),
        ("erin", RepositoryPermission::Triage),
    ] {
        assert_eq!(
            snapshot.resolve(&user(login)).unwrap_err(),
            OwnershipBlocker::InsufficientRepositoryPermission {
                identity: user(login),
                permission,
            }
        );
    }
}

#[test]
fn t3_visible_team_returns_sorted_active_direct_and_inherited_members() {
    let snapshot = snapshot();
    assert_eq!(snapshot.resolve(&team("PAYMENTS")).unwrap(), vec![11, 12]);
}

#[test]
fn t4_secret_team_is_an_explicit_blocker() {
    assert_eq!(
        snapshot().resolve(&team("security")).unwrap_err(),
        OwnershipBlocker::TeamNotVisible {
            organization: "acme".to_owned(),
            slug: "security".to_owned(),
        }
    );
}

#[test]
fn t5_team_requires_write_or_greater_repository_permission() {
    let mut snapshot = snapshot();
    snapshot.teams[0].repository_permission = RepositoryPermission::Triage;
    assert_eq!(
        snapshot.resolve(&team("payments")).unwrap_err(),
        OwnershipBlocker::InsufficientRepositoryPermission {
            identity: team("payments"),
            permission: RepositoryPermission::Triage,
        }
    );

    snapshot.teams[0].repository_permission = RepositoryPermission::Maintain;
    assert_eq!(snapshot.resolve(&team("payments")).unwrap(), vec![11, 12]);
}

#[test]
fn t6_email_codeowner_is_a_typed_blocker() {
    let identity = CodeownerIdentity::Email {
        address: "owner@example.com".to_owned(),
    };
    assert_eq!(
        snapshot().resolve(&identity).unwrap_err(),
        OwnershipBlocker::EmailCodeownerUnsupported {
            address: "owner@example.com".to_owned(),
        }
    );
}

#[test]
fn t7_duplicate_or_unsorted_user_identity_fails_closed() {
    let mut duplicate_id = snapshot();
    duplicate_id.users[1].id = duplicate_id.users[0].id;
    assert!(duplicate_id.validate().is_err());

    let mut duplicate_login = snapshot();
    duplicate_login.users[1].login = "ALICE".to_owned();
    assert!(duplicate_login.validate().is_err());

    let mut unsorted = snapshot();
    unsorted.users.swap(0, 1);
    assert!(unsorted.validate().is_err());
}

#[test]
fn t8_duplicate_or_unsorted_team_and_member_identity_fails_closed() {
    let mut duplicate_team_id = snapshot();
    duplicate_team_id.teams[1].id = duplicate_team_id.teams[0].id;
    assert!(duplicate_team_id.validate().is_err());

    let mut duplicate_team_name = snapshot();
    duplicate_team_name.teams[1].slug = "PAYMENTS".to_owned();
    assert!(duplicate_team_name.validate().is_err());

    let mut duplicate_member = snapshot();
    duplicate_member.teams[0].members[1].user_id = 11;
    assert!(duplicate_member.validate().is_err());

    let mut unsorted_teams = snapshot();
    unsorted_teams.teams.swap(0, 1);
    assert!(unsorted_teams.validate().is_err());

    let mut unsorted_members = snapshot();
    unsorted_members.teams[0].members.swap(0, 1);
    assert!(unsorted_members.validate().is_err());
}

#[test]
fn t9_missing_members_and_codeowner_identities_fail_closed() {
    let mut missing_member = snapshot();
    missing_member.teams[0].members[0].user_id = 999;
    assert!(missing_member.validate().is_err());

    assert_eq!(
        snapshot().resolve(&user("missing")).unwrap_err(),
        OwnershipBlocker::UserNotFound {
            login: "missing".to_owned(),
        }
    );
    assert_eq!(
        snapshot().resolve(&team("missing")).unwrap_err(),
        OwnershipBlocker::TeamNotFound {
            organization: "acme".to_owned(),
            slug: "missing".to_owned(),
        }
    );

    let mut no_candidates = snapshot();
    no_candidates.teams[0].members[0].state = GithubMembershipState::Pending;
    no_candidates.teams[0].members[1].state = GithubMembershipState::Pending;
    assert_eq!(
        no_candidates.resolve(&team("payments")).unwrap_err(),
        OwnershipBlocker::NoEligibleTeamMembers {
            organization: "acme".to_owned(),
            slug: "payments".to_owned(),
        }
    );
}

#[test]
fn t10_schema_metadata_unknown_fields_and_resource_limits_fail_closed() {
    let fixture = snapshot();
    let encoded = serde_json::to_value(&fixture).unwrap();
    let decoded: GithubOwnershipSnapshot = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, fixture);

    let mut unknown = encoded.clone();
    unknown["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<GithubOwnershipSnapshot>(unknown).is_err());

    let mut missing = encoded;
    missing.as_object_mut().unwrap().remove("repository_id");
    assert!(serde_json::from_value::<GithubOwnershipSnapshot>(missing).is_err());

    let mut invalid_metadata = snapshot();
    invalid_metadata.provider_url = "http://github.com".to_owned();
    assert!(invalid_metadata.validate().is_err());
    invalid_metadata = snapshot();
    invalid_metadata.base_commit = "NOT-A-COMMIT".to_owned();
    assert!(invalid_metadata.validate().is_err());
    invalid_metadata = snapshot();
    invalid_metadata.observed_at = "2026-09-05".to_owned();
    assert!(invalid_metadata.validate().is_err());

    let mut oversized = snapshot();
    oversized.users = (1..=(MAX_OWNERSHIP_USERS as u64 + 1))
        .map(|id| GithubOwnershipUser {
            id,
            login: format!("user-{id}"),
            repository_permission: RepositoryPermission::Write,
        })
        .collect();
    assert!(oversized.validate().is_err());
}

#[test]
fn t11_validated_index_resolves_repeated_user_and_team_queries() {
    let snapshot = snapshot();
    let index = snapshot.index().unwrap();

    assert_eq!(index.resolve(&user("ALICE")).unwrap(), vec![11]);
    assert_eq!(index.resolve(&team("PAYMENTS")).unwrap(), vec![11, 12]);
    assert_eq!(index.user(12).unwrap().login, "bob");
    assert!(index.user(999).is_none());
}
