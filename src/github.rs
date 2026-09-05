use std::cmp::Ordering;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

pub const GITHUB_CHECKPOINT_SCHEMA: &str = "stratadiff-github-review-checkpoint-v1";
pub const MAX_GITHUB_REVIEWS: usize = 10_000;
pub const MAX_GITHUB_REVIEWS_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_GITHUB_RESPONSE_HEADERS_BYTES: usize = 64 * 1024;
pub const MAX_GITHUB_REVIEWS_INCLUDED_RESPONSE_BYTES: usize =
    MAX_GITHUB_RESPONSE_HEADERS_BYTES + MAX_GITHUB_REVIEWS_BYTES;
pub const MAX_GITHUB_COMMIT_OBJECT_BYTES: usize = 1024 * 1024;

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
struct GithubCommitObject {
    sha: String,
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
    let reviews: Vec<GithubReview> = serde_json::from_slice(reviews_json)
        .context("failed to decode GitHub pull request reviews")?;
    resolve_github_reviews(reviews, reviewer)
}

pub fn resolve_github_review_checkpoint_included_response(
    response: &[u8],
    reviewer: &str,
) -> Result<GithubCheckpointResolution> {
    let body = github_json_response_body(response)?;
    ensure!(
        body.len() <= MAX_GITHUB_REVIEWS_BYTES,
        "GitHub pull request reviews bytes limit exceeded: observed {}, limit {MAX_GITHUB_REVIEWS_BYTES}",
        body.len()
    );
    resolve_github_review_checkpoint(body, reviewer)
}

pub fn resolve_github_review_checkpoint_slurp_pages(
    review_pages_json: &[u8],
    reviewer: &str,
) -> Result<GithubCheckpointResolution> {
    ensure!(
        review_pages_json.len() <= MAX_GITHUB_REVIEWS_BYTES,
        "GitHub pull request reviews bytes limit exceeded: observed {}, limit {MAX_GITHUB_REVIEWS_BYTES}",
        review_pages_json.len()
    );
    let pages: Vec<Vec<GithubReview>> = serde_json::from_slice(review_pages_json)
        .context("failed to decode gh api --paginate --slurp review pages")?;
    let mut reviews = Vec::new();
    for page in pages {
        let combined_count = reviews
            .len()
            .checked_add(page.len())
            .context("GitHub review count overflow while combining pages")?;
        ensure!(
            combined_count <= MAX_GITHUB_REVIEWS,
            "GitHub review count limit exceeded: observed at least {combined_count}, limit {MAX_GITHUB_REVIEWS}"
        );
        reviews.extend(page);
    }
    resolve_github_reviews(reviews, reviewer)
}

fn resolve_github_reviews(
    reviews: Vec<GithubReview>,
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
            bail!(
                "GitHub completed review {} is missing its submitted_at timestamp",
                review.id
            );
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
    eligible.sort_by(|left, right| {
        compare_github_timestamps(&left.0, &right.0).then_with(|| left.1.cmp(&right.1))
    });
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

fn github_json_response_body(response: &[u8]) -> Result<&[u8]> {
    let (status_line, mut cursor) = next_http_line(response, 0)?;
    ensure!(
        valid_success_status_line(status_line),
        "GitHub included response does not contain one HTTP 200 status line"
    );

    let mut content_type_seen = false;
    loop {
        ensure!(
            cursor <= MAX_GITHUB_RESPONSE_HEADERS_BYTES,
            "GitHub response headers bytes limit exceeded: observed at least {cursor}, limit {MAX_GITHUB_RESPONSE_HEADERS_BYTES}"
        );
        let (line, next_cursor) = next_http_line(response, cursor)?;
        cursor = next_cursor;
        ensure!(
            cursor <= MAX_GITHUB_RESPONSE_HEADERS_BYTES,
            "GitHub response headers bytes limit exceeded: observed at least {cursor}, limit {MAX_GITHUB_RESPONSE_HEADERS_BYTES}"
        );
        if line.is_empty() {
            break;
        }
        ensure!(
            !matches!(line.first(), Some(b' ' | b'\t')),
            "GitHub included response contains an obsolete folded header"
        );
        let separator = line
            .iter()
            .position(|byte| *byte == b':')
            .context("GitHub included response contains a malformed header")?;
        let name = &line[..separator];
        let value = trim_optional_whitespace(&line[separator + 1..]);
        ensure!(
            valid_http_header_name(name) && valid_http_header_value(value),
            "GitHub included response contains a malformed header"
        );
        ensure!(
            !name.eq_ignore_ascii_case(b"link"),
            "GitHub review pagination is incomplete: Link header present; use gh api --paginate --slurp"
        );
        if name.eq_ignore_ascii_case(b"content-type") {
            ensure!(
                !content_type_seen,
                "GitHub included response contains duplicate Content-Type headers"
            );
            let media_type = trim_optional_whitespace(
                value
                    .split(|byte| *byte == b';')
                    .next()
                    .expect("split always returns one item"),
            );
            ensure!(
                media_type.eq_ignore_ascii_case(b"application/json"),
                "GitHub included response Content-Type is not application/json"
            );
            content_type_seen = true;
        }
    }
    ensure!(
        content_type_seen,
        "GitHub included response is missing Content-Type"
    );
    Ok(&response[cursor..])
}

fn next_http_line(bytes: &[u8], start: usize) -> Result<(&[u8], usize)> {
    ensure!(
        start < bytes.len(),
        "GitHub included response is missing the header/body separator"
    );
    let relative_end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .context("GitHub included response contains an unterminated header line")?;
    let end = start + relative_end;
    let mut line = &bytes[start..end];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    ensure!(
        !line.contains(&b'\r'),
        "GitHub included response contains an invalid carriage return"
    );
    Ok((line, end + 1))
}

fn valid_success_status_line(line: &[u8]) -> bool {
    let Ok(line) = std::str::from_utf8(line) else {
        return false;
    };
    let mut fields = line.split_ascii_whitespace();
    let Some(protocol) = fields.next() else {
        return false;
    };
    let Some(status) = fields.next() else {
        return false;
    };
    line.as_bytes().first() == Some(&b'H')
        && protocol.strip_prefix("HTTP/").is_some_and(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        && status == "200"
}

fn trim_optional_whitespace(mut value: &[u8]) -> &[u8] {
    while matches!(value.first(), Some(b' ' | b'\t')) {
        value = &value[1..];
    }
    while matches!(value.last(), Some(b' ' | b'\t')) {
        value = &value[..value.len() - 1];
    }
    value
}

fn valid_http_header_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_http_header_value(value: &[u8]) -> bool {
    value
        .iter()
        .all(|byte| *byte == b'\t' || (b' '..=b'~').contains(byte) || *byte >= 0x80)
}

pub fn verify_github_commit_object(commit_json: &[u8], expected_sha: &str) -> Result<()> {
    ensure!(
        is_sha1(expected_sha),
        "expected GitHub commit ID must be a full lowercase SHA-1 object ID"
    );
    let commit: GithubCommitObject =
        serde_json::from_slice(commit_json).context("failed to decode GitHub Git commit object")?;
    ensure!(
        is_sha1(&commit.sha),
        "GitHub Git commit object has an invalid sha"
    );
    ensure!(
        commit.sha == expected_sha,
        "GitHub Git commit object resolved to {}, expected {expected_sha}",
        commit.sha
    );
    Ok(())
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_github_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    let valid_base = bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[..19]
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit());
    if !valid_base {
        return false;
    }
    if bytes.len() == 20 {
        return bytes[19] == b'Z';
    }
    let fraction_digits = bytes.len() - 21;
    (1..=9).contains(&fraction_digits)
        && bytes[19] == b'.'
        && bytes[bytes.len() - 1] == b'Z'
        && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
}

fn compare_github_timestamps(left: &str, right: &str) -> Ordering {
    let base = left[..19].cmp(&right[..19]);
    if base != Ordering::Equal {
        return base;
    }
    let left_fraction = if left.len() == 20 {
        &[][..]
    } else {
        &left.as_bytes()[20..left.len() - 1]
    };
    let right_fraction = if right.len() == 20 {
        &[][..]
    } else {
        &right.as_bytes()[20..right.len() - 1]
    };
    (0..9)
        .map(|index| {
            let left_digit = left_fraction.get(index).copied().unwrap_or(b'0');
            let right_digit = right_fraction.get(index).copied().unwrap_or(b'0');
            left_digit.cmp(&right_digit)
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INCLUDED_REVIEW_BODY: &[u8] = br#"[{
      "id": 7,
      "user": {"login": "reviewer", "type": "User"},
      "state": "APPROVED",
      "html_url": "https://github.example/review/7",
      "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "submitted_at": "2026-09-04T17:10:09Z",
      "author_association": "MEMBER",
      "body": "Link: this is review text, not an HTTP header"
    }]"#;

    fn included_response(headers: &[u8], body: &[u8]) -> Vec<u8> {
        let mut response =
            b"HTTP/2.0 200 OK\nContent-Type: application/json; charset=utf-8\r\n".to_vec();
        response.extend_from_slice(headers);
        response.extend_from_slice(b"\r\n");
        response.extend_from_slice(body);
        response
    }

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
    fn fractional_timestamp_orders_the_same_second_before_the_id_tiebreaker() {
        let reviews = br#"[
          {
            "id": 9,
            "user": {"login": "reviewer", "type": "User"},
            "state": "APPROVED",
            "html_url": "https://github.example/review/9",
            "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "submitted_at": "2026-09-04T17:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 1,
            "user": {"login": "reviewer", "type": "User"},
            "state": "CHANGES_REQUESTED",
            "html_url": "https://github.example/review/1",
            "commit_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "submitted_at": "2026-09-04T17:10:09.000000001Z",
            "author_association": "MEMBER"
          }
        ]"#;

        let resolution = resolve_github_review_checkpoint(reviews, "reviewer").unwrap();
        assert_eq!(resolution.checkpoint.unwrap().review_id, 1);
        assert!(valid_github_timestamp("2026-09-04T17:10:09.123456789Z"));
        assert!(!valid_github_timestamp("2026-09-04T17:10:09.1234567890Z"));
    }

    #[test]
    fn included_response_accepts_gh_mixed_line_endings_and_ignores_body_header_text() {
        let response = included_response(b"Etag: \"review-snapshot\"\r\n", INCLUDED_REVIEW_BODY);
        let resolution =
            resolve_github_review_checkpoint_included_response(&response, "reviewer").unwrap();

        assert_eq!(resolution.observed_reviews, 1);
        assert_eq!(resolution.checkpoint.unwrap().review_id, 7);
    }

    #[test]
    fn included_response_rejects_every_pagination_link_before_decoding_the_body() {
        for name in [b"Link".as_slice(), b"link", b"LiNk"] {
            let mut headers = name.to_vec();
            headers.extend_from_slice(
                b": <https://api.github.example/reviews?page=2>; rel=\"next\"\r\n",
            );
            let response = included_response(&headers, b"not json");
            let error = resolve_github_review_checkpoint_included_response(&response, "reviewer")
                .unwrap_err();
            assert!(error.to_string().contains("pagination is incomplete"));
            assert!(error.to_string().contains("--paginate --slurp"));
            assert!(!error.to_string().contains("count limit exceeded"));
        }
    }

    #[test]
    fn included_response_rejects_malformed_http_envelopes() {
        let malformed = [
            b"HTTP/2.0 500 Internal Server Error\nContent-Type: application/json\r\n\r\n[]"
                .as_slice(),
            b"HTTP/2.0 200 OK\nContent-Type: text/plain\r\n\r\n[]".as_slice(),
            b"HTTP/2.0 200 OK\nX-Test: value\r\n\r\n[]".as_slice(),
            b"HTTP/2.0 200 OK\nContent-Type: application/json\r\n folded\r\n\r\n[]"
                .as_slice(),
            b"HTTP/2.0 200 OK\nContent Type: application/json\r\n\r\n[]".as_slice(),
            b"HTTP/2.0 200 OK\nContent-Type: application/json\r\n[]".as_slice(),
            b"HTTP/2.0 200 OK\nContent-Type: application/json\r\nContent-Type: application/json\r\n\r\n[]"
                .as_slice(),
            b"HTTP/2.0 200 OK\nContent-Type: application/json\r\n\r\nHTTP/2.0 200 OK\n\r\n[]"
                .as_slice(),
        ];
        for response in malformed {
            assert!(
                resolve_github_review_checkpoint_included_response(response, "reviewer").is_err()
            );
        }
    }

    #[test]
    fn included_response_enforces_independent_header_and_body_byte_limits() {
        let mut oversized_header = b"HTTP/2.0 200 OK\nX-Test: ".to_vec();
        oversized_header.extend(std::iter::repeat_n(b'a', MAX_GITHUB_RESPONSE_HEADERS_BYTES));
        oversized_header.extend_from_slice(b"\r\nContent-Type: application/json\r\n\r\n[]");
        let header_error =
            resolve_github_review_checkpoint_included_response(&oversized_header, "reviewer")
                .unwrap_err();
        assert!(
            header_error
                .to_string()
                .contains("headers bytes limit exceeded")
        );

        let oversized_body = vec![b' '; MAX_GITHUB_REVIEWS_BYTES + 1];
        let slurp_error =
            resolve_github_review_checkpoint_slurp_pages(&oversized_body, "reviewer").unwrap_err();
        assert!(
            slurp_error
                .to_string()
                .contains("reviews bytes limit exceeded")
        );
        let response = included_response(b"", &oversized_body);
        let body_error =
            resolve_github_review_checkpoint_included_response(&response, "reviewer").unwrap_err();
        assert!(
            body_error
                .to_string()
                .contains("reviews bytes limit exceeded")
        );
    }

    #[test]
    fn included_response_accepts_one_full_review_page_without_a_link() {
        let reviews = (1..=100)
            .map(|id| {
                format!(
                    r#"{{"id":{id},"user":{{"login":"reviewer","type":"User"}},"state":"APPROVED","html_url":"https://github.example/review/{id}","commit_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","submitted_at":"2026-09-04T17:10:09Z","author_association":"MEMBER"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let response = included_response(b"", format!("[{reviews}]").as_bytes());
        let resolution =
            resolve_github_review_checkpoint_included_response(&response, "reviewer").unwrap();

        assert_eq!(resolution.observed_reviews, 100);
    }

    #[test]
    fn verifies_provider_commit_object_against_the_review_sha() {
        let sha = "a".repeat(40);
        let object = format!(r#"{{"sha":"{sha}","message":"reviewed"}}"#);
        verify_github_commit_object(object.as_bytes(), &sha).unwrap();

        let error = verify_github_commit_object(object.as_bytes(), &"b".repeat(40)).unwrap_err();
        assert!(error.to_string().contains("resolved to"));
        assert!(error.to_string().contains("expected"));
    }

    #[test]
    fn rejects_malformed_provider_commit_objects() {
        let sha = "a".repeat(40);
        for object in [
            br#"{}"#.as_slice(),
            br#"{"sha":null}"#.as_slice(),
            br#"{"sha":"not-an-object-id"}"#.as_slice(),
        ] {
            assert!(verify_github_commit_object(object, &sha).is_err());
        }
        assert!(
            verify_github_commit_object(
                br#"{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
                "HEAD"
            )
            .is_err()
        );
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

    #[test]
    fn matching_completed_review_without_a_timestamp_fails_closed() {
        let reviews = br#"[
          {
            "id": 1,
            "user": {"login": "reviewer", "type": "User"},
            "state": "APPROVED",
            "html_url": "https://github.example/review/1",
            "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "submitted_at": "2026-09-04T17:10:09Z",
            "author_association": "MEMBER"
          },
          {
            "id": 2,
            "user": {"login": "reviewer", "type": "User"},
            "state": "CHANGES_REQUESTED",
            "html_url": "https://github.example/review/2",
            "commit_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "submitted_at": null,
            "author_association": "MEMBER"
          }
        ]"#;

        let error = resolve_github_review_checkpoint(reviews, "reviewer").unwrap_err();
        assert!(error.to_string().contains("missing its submitted_at"));
    }
}
