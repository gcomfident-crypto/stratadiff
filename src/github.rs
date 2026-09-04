use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const GITHUB_CHECKPOINT_SCHEMA: &str = "stratadiff-github-review-checkpoint-v1";
pub const MAX_GITHUB_REVIEWS: usize = 100;
pub const MAX_GITHUB_REVIEWS_BYTES: usize = 8 * 1024 * 1024;

const SELECTION_POLICY: &str =
    "latest_nondismissed_human_approved_or_changes_requested_review_for_explicit_reviewer";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubReviewCheckpoint {
    pub review_id: u64,
    pub reviewer_login: String,
    pub review_state: String,
    pub commit_id: String,
    pub submitted_at: String,
    pub html_url: String,
    pub author_association: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubCheckpointResolution {
    pub schema: &'static str,
    pub selection_policy: &'static str,
    pub requested_reviewer: String,
    pub observed_reviews: usize,
    pub matching_reviewer_reviews: usize,
    pub eligible_reviews: usize,
    pub checkpoint: Option<GithubReviewCheckpoint>,
}

#[derive(Debug, Deserialize)]
struct GithubReview {
    id: u64,
    user: NullableUser,
    state: String,
    html_url: String,
    commit_id: String,
    submitted_at: NullableString,
    author_association: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NullableUser {
    User(GithubUser),
    Null,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
    #[serde(rename = "type")]
    account_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NullableString {
    String(String),
    Null,
}

pub fn resolve_github_review_checkpoint(
    reviews_json: &[u8],
    reviewer: &str,
) -> Result<GithubCheckpointResolution> {
    ensure!(
        !reviewer.is_empty(),
        "GitHub reviewer login must not be empty"
    );
    ensure!(
        reviewer.trim() == reviewer,
        "GitHub reviewer login must not contain surrounding whitespace"
    );
    let reviews: Vec<GithubReview> = serde_json::from_slice(reviews_json)
        .context("failed to decode GitHub pull request reviews")?;
    ensure!(
        reviews.len() <= MAX_GITHUB_REVIEWS,
        "GitHub review count limit exceeded: observed {}, limit {MAX_GITHUB_REVIEWS}",
        reviews.len()
    );
    let observed_reviews = reviews.len();

    let mut matching_reviewer_reviews = 0_usize;
    let mut eligible = Vec::new();
    for review in reviews {
        let NullableUser::User(user) = review.user else {
            continue;
        };
        if !user.login.eq_ignore_ascii_case(reviewer) {
            continue;
        }
        matching_reviewer_reviews += 1;
        if user.account_type != "User"
            || !matches!(review.state.as_str(), "APPROVED" | "CHANGES_REQUESTED")
        {
            continue;
        }
        let NullableString::String(submitted_at) = review.submitted_at else {
            continue;
        };
        ensure!(
            valid_github_timestamp(&submitted_at),
            "GitHub review {} has an invalid submitted_at timestamp",
            review.id
        );
        ensure!(
            is_sha1(&review.commit_id),
            "GitHub review {} has an invalid commit_id",
            review.id
        );
        ensure!(
            !review.html_url.is_empty(),
            "GitHub review {} has an empty html_url",
            review.id
        );
        eligible.push((
            submitted_at.clone(),
            review.id,
            GithubReviewCheckpoint {
                review_id: review.id,
                reviewer_login: user.login,
                review_state: review.state.to_ascii_lowercase(),
                commit_id: review.commit_id,
                submitted_at,
                html_url: review.html_url,
                author_association: review.author_association,
            },
        ));
    }
    eligible.sort_by(|left, right| (left.0.as_str(), left.1).cmp(&(right.0.as_str(), right.1)));
    let eligible_reviews = eligible.len();
    let checkpoint = eligible.pop().map(|(_, _, checkpoint)| checkpoint);

    Ok(GithubCheckpointResolution {
        schema: GITHUB_CHECKPOINT_SCHEMA,
        selection_policy: SELECTION_POLICY,
        requested_reviewer: reviewer.to_owned(),
        observed_reviews,
        matching_reviewer_reviews,
        eligible_reviews,
        checkpoint,
    })
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_latest_completed_human_review_for_the_requested_reviewer() {
        let reviews = br#"[
          {
            "id": 4,
            "user": {"login": "other", "type": "User"},
            "state": "APPROVED",
            "html_url": "https://github.example/review/4",
            "commit_id": "dddddddddddddddddddddddddddddddddddddddd",
            "submitted_at": "2026-09-04T20:03:17Z",
            "author_association": "MEMBER"
          },
          {
            "id": 1,
            "user": {"login": "Reviewer", "type": "User"},
            "state": "APPROVED",
            "html_url": "https://github.example/review/1",
            "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "submitted_at": "2026-09-04T17:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 3,
            "user": {"login": "reviewer", "type": "User"},
            "state": "COMMENTED",
            "html_url": "https://github.example/review/3",
            "commit_id": "cccccccccccccccccccccccccccccccccccccccc",
            "submitted_at": "2026-09-04T21:03:17Z",
            "author_association": "MEMBER"
          },
          {
            "id": 2,
            "user": {"login": "reviewer", "type": "User"},
            "state": "CHANGES_REQUESTED",
            "html_url": "https://github.example/review/2",
            "commit_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "submitted_at": "2026-09-04T19:03:17Z",
            "author_association": "MEMBER"
          }
        ]"#;

        let resolution = resolve_github_review_checkpoint(reviews, "reviewer").unwrap();
        assert_eq!(resolution.observed_reviews, 4);
        assert_eq!(resolution.matching_reviewer_reviews, 3);
        assert_eq!(resolution.eligible_reviews, 2);
        let checkpoint = resolution.checkpoint.unwrap();
        assert_eq!(checkpoint.review_id, 2);
        assert_eq!(checkpoint.review_state, "changes_requested");
        assert_eq!(checkpoint.commit_id, "b".repeat(40));
    }

    #[test]
    fn excludes_bots_comments_dismissals_pending_and_deleted_users() {
        let reviews = br#"[
          {
            "id": 1,
            "user": {"login": "reviewer", "type": "Bot"},
            "state": "APPROVED",
            "html_url": "https://github.example/review/1",
            "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "submitted_at": "2026-09-04T17:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 2,
            "user": {"login": "reviewer", "type": "User"},
            "state": "COMMENTED",
            "html_url": "https://github.example/review/2",
            "commit_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "submitted_at": "2026-09-04T18:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 3,
            "user": {"login": "reviewer", "type": "User"},
            "state": "DISMISSED",
            "html_url": "https://github.example/review/3",
            "commit_id": "cccccccccccccccccccccccccccccccccccccccc",
            "submitted_at": "2026-09-04T19:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 4,
            "user": {"login": "reviewer", "type": "User"},
            "state": "PENDING",
            "html_url": "https://github.example/review/4",
            "commit_id": "dddddddddddddddddddddddddddddddddddddddd",
            "submitted_at": null,
            "author_association": "MEMBER"
          },
          {
            "id": 5,
            "user": null,
            "state": "APPROVED",
            "html_url": "https://github.example/review/5",
            "commit_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "submitted_at": "2026-09-04T20:10:09Z",
            "author_association": "MEMBER"
          }
        ]"#;

        let resolution = resolve_github_review_checkpoint(reviews, "reviewer").unwrap();
        assert_eq!(resolution.observed_reviews, 5);
        assert_eq!(resolution.eligible_reviews, 0);
        assert_eq!(resolution.checkpoint, None);
    }
}
