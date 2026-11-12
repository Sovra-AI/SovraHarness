//! One function per [`GitHubRequest`]. Each turns GitHub's JSON into a compact
//! result for the model. Text other people wrote (issues, comments, code) is
//! clipped and sits under an `untrusted` notice.

use base64::Engine as _;
use serde_json::{Value, json};
use sovra_core::github::{GitHubRequest, RepoName, SearchKind, UNTRUSTED, clip_text, search_query};

use crate::http::{CallError, api_headers, read_json};

const BODY_CHARS: usize = 8_000;
const COMMENT_CHARS: usize = 1_500;
const PREVIEW_CHARS: usize = 300;
const FILE_CHARS: usize = 30_000;
const PATCH_CHARS: usize = 1_500;
/// The latest comments shown with an issue; earlier ones are counted, not shown.
const COMMENTS_SHOWN: usize = 20;
const FILES_SHOWN: usize = 60;
const PATCHES_SHOWN: usize = 15;
const FOLDER_ENTRIES: usize = 300;
const TITLE_CHARS: usize = 256;
const WRITE_BODY_CHARS: usize = 20_000;

type Result<T> = std::result::Result<T, CallError>;

struct Api<'a> {
    http: &'a reqwest::Client,
    base: &'a str,
    token: &'a str,
}

impl Api<'_> {
    fn url(&self, segments: &[&str]) -> Result<url::Url> {
        address(self.base, segments)
    }

    fn repo_url(&self, repo: &RepoName, rest: &[&str]) -> Result<url::Url> {
        let mut segments = vec!["repos", repo.owner.as_str(), repo.name.as_str()];
        segments.extend_from_slice(rest);
        self.url(&segments)
    }

    async fn get(&self, url: url::Url, query: &[(&str, String)]) -> Result<Value> {
        read_json(
            self.http
                .get(url)
                .headers(api_headers(self.token))
                .query(query),
        )
        .await
    }

    async fn post(&self, url: url::Url, body: &Value) -> Result<Value> {
        read_json(
            self.http
                .post(url)
                .headers(api_headers(self.token))
                .json(body),
        )
        .await
    }
}

pub(crate) async fn call(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    request: GitHubRequest,
) -> Result<Value> {
    let api = Api { http, base, token };
    match request {
        GitHubRequest::ListRepositories { query, limit } => {
            let repos = api
                .get(
                    api.url(&["user", "repos"])?,
                    &[
                        ("sort", "pushed".into()),
                        ("per_page", "100".into()),
                        (
                            "affiliation",
                            "owner,collaborator,organization_member".into(),
                        ),
                    ],
                )
                .await?;
            Ok(repositories(&repos, query.as_deref(), limit as usize))
        }
        GitHubRequest::Search { query, kind, limit } => {
            let found = api
                .get(
                    api.url(&["search", "issues"])?,
                    &[
                        ("q", search_query(&query, kind)),
                        ("sort", "updated".into()),
                        ("per_page", limit.to_string()),
                    ],
                )
                .await?;
            Ok(search_results(&found, kind))
        }
        GitHubRequest::ReadIssue { repo, number } => {
            let number = number.to_string();
            let issue = api
                .get(api.repo_url(&repo, &["issues", &number])?, &[])
                .await?;
            let comments = if issue["comments"].as_u64().unwrap_or(0) > 0 {
                api.get(
                    api.repo_url(&repo, &["issues", &number, "comments"])?,
                    &[("per_page", "100".into())],
                )
                .await?
            } else {
                Value::Array(Vec::new())
            };
            Ok(issue_with_comments(&repo, &issue, &comments))
        }
        GitHubRequest::ReadPullRequest { repo, number } => {
            let number = number.to_string();
            let pull = api
                .get(api.repo_url(&repo, &["pulls", &number])?, &[])
                .await?;
            let files = api
                .get(
                    api.repo_url(&repo, &["pulls", &number, "files"])?,
                    &[("per_page", "100".into())],
                )
                .await?;
            let sha = pull["head"]["sha"].as_str().unwrap_or_default();
            // Only a commit id GitHub itself sent goes back into an address.
            let checks = if !sha.is_empty() && sha.chars().all(|c| c.is_ascii_hexdigit()) {
                api.get(
                    api.repo_url(&repo, &["commits", sha, "check-runs"])?,
                    &[("per_page", "100".into())],
                )
                .await
                .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            Ok(pull_request(&repo, &pull, &files, &checks))
        }
        GitHubRequest::ReadFile {
            repo,
            path,
            reference,
        } => {
            let mut rest = vec!["contents"];
            rest.extend(path.split('/').filter(|segment| !segment.is_empty()));
            let query: Vec<(&str, String)> = reference
                .iter()
                .map(|reference| ("ref", reference.clone()))
                .collect();
            let contents = api.get(api.repo_url(&repo, &rest)?, &query).await?;
            file_or_folder(&repo, &path, &contents)
        }
        GitHubRequest::CreateIssue { repo, title, body } => {
            let title = one_line(&title);
            let created = api
                .post(
                    api.repo_url(&repo, &["issues"])?,
                    &json!({
                        "title": clip_text(&title, TITLE_CHARS),
                        "body": clip_text(&body, WRITE_BODY_CHARS),
                    }),
                )
                .await?;
            Ok(json!({
                "ok": true,
                "repo": repo.full(),
                "number": created["number"],
                "url": created["html_url"],
            }))
        }
        GitHubRequest::Comment { repo, number, body } => {
            let number_text = number.to_string();
            let created = api
                .post(
                    api.repo_url(&repo, &["issues", &number_text, "comments"])?,
                    &json!({ "body": clip_text(&body, WRITE_BODY_CHARS) }),
                )
                .await?;
            Ok(json!({
                "ok": true,
                "repo": repo.full(),
                "number": number,
                "url": created["html_url"],
            }))
        }
    }
}

/// `segments` are added one by one, so each is percent-encoded and none can add a
/// query, a fragment or a `..` step.
pub(crate) fn address(base: &str, segments: &[&str]) -> Result<url::Url> {
    let wrong = || CallError::Failed("Sovra's GitHub address is wrong.".into());
    let mut url = url::Url::parse(base).map_err(|_| wrong())?;
    url.path_segments_mut()
        .map_err(|()| wrong())?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// Titles become a single line, so a model cannot smuggle extra lines in.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn repositories(repos: &Value, query: Option<&str>, limit: usize) -> Value {
    let needle = query.map(str::to_lowercase).unwrap_or_default();
    let items: Vec<Value> = repos
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|repo| {
            needle.is_empty()
                || text(&repo["full_name"]).to_lowercase().contains(&needle)
                || text(&repo["description"]).to_lowercase().contains(&needle)
        })
        .take(limit)
        .map(|repo| {
            json!({
                "repo": repo["full_name"],
                "description": clip_text(text(&repo["description"]), PREVIEW_CHARS),
                "private": repo["private"],
                "language": repo["language"],
                "stars": repo["stargazers_count"],
                "open_issues": repo["open_issues_count"],
                "default_branch": repo["default_branch"],
                "last_changed": repo["pushed_at"],
                "url": repo["html_url"],
            })
        })
        .collect();
    json!({ "ok": true, "untrusted": UNTRUSTED, "repositories": items })
}

/// `https://api.github.com/repos/owner/name` → `owner/name`.
fn repo_from_api_url(url: &str) -> String {
    url.split("/repos/").nth(1).unwrap_or_default().to_owned()
}

pub(crate) fn search_results(found: &Value, kind: SearchKind) -> Value {
    let items: Vec<Value> = found["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|item| {
            json!({
                "repo": repo_from_api_url(text(&item["repository_url"])),
                "number": item["number"],
                "title": item["title"],
                "state": item["state"],
                "author": item["user"]["login"],
                "pull_request": item.get("pull_request").is_some(),
                "comments": item["comments"],
                "updated": item["updated_at"],
                "preview": clip_text(text(&item["body"]), PREVIEW_CHARS),
                "url": item["html_url"],
            })
        })
        .collect();
    json!({
        "ok": true,
        "looked_for": match kind {
            SearchKind::Issues => "issues",
            SearchKind::PullRequests => "pull requests",
        },
        "total": found["total_count"],
        "untrusted": UNTRUSTED,
        "results": items,
    })
}

pub(crate) fn issue_with_comments(repo: &RepoName, issue: &Value, comments: &Value) -> Value {
    let all = comments.as_array().map(Vec::as_slice).unwrap_or_default();
    let shown: Vec<Value> = all
        .iter()
        .skip(all.len().saturating_sub(COMMENTS_SHOWN))
        .map(|comment| {
            json!({
                "author": comment["user"]["login"],
                "when": comment["created_at"],
                "text": clip_text(text(&comment["body"]), COMMENT_CHARS),
            })
        })
        .collect();
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|label| text(&label["name"]))
        .collect();
    json!({
        "ok": true,
        "repo": repo.full(),
        "number": issue["number"],
        "title": issue["title"],
        "state": issue["state"],
        "pull_request": issue.get("pull_request").is_some(),
        "author": issue["user"]["login"],
        "labels": labels,
        "opened": issue["created_at"],
        "url": issue["html_url"],
        "untrusted": UNTRUSTED,
        "text": clip_text(text(&issue["body"]), BODY_CHARS),
        "comment_count": issue["comments"],
        "earlier_comments_not_shown": all.len().saturating_sub(COMMENTS_SHOWN),
        "comments": shown,
    })
}

/// Counts check runs by outcome and names the ones that did not pass.
pub(crate) fn checks_summary(checks: &Value) -> Value {
    let Some(runs) = checks["check_runs"].as_array() else {
        return json!({ "known": false });
    };
    let mut passed = 0;
    let mut running = 0;
    let mut problems = Vec::new();
    for run in runs {
        match (text(&run["status"]), text(&run["conclusion"])) {
            (status, _) if status != "completed" => running += 1,
            (_, "success" | "neutral" | "skipped") => passed += 1,
            (_, conclusion) => problems.push(json!({
                "name": run["name"],
                "result": conclusion,
            })),
        }
    }
    json!({
        "known": true,
        "passed": passed,
        "still_running": running,
        "did_not_pass": problems,
    })
}

pub(crate) fn pull_request(repo: &RepoName, pull: &Value, files: &Value, checks: &Value) -> Value {
    let changed: Vec<Value> = files
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(FILES_SHOWN)
        .enumerate()
        .map(|(index, file)| {
            let mut entry = json!({
                "file": file["filename"],
                "change": file["status"],
                "added_lines": file["additions"],
                "removed_lines": file["deletions"],
            });
            if index < PATCHES_SHOWN {
                entry["patch"] = Value::String(clip_text(text(&file["patch"]), PATCH_CHARS));
            }
            entry
        })
        .collect();
    json!({
        "ok": true,
        "repo": repo.full(),
        "number": pull["number"],
        "title": pull["title"],
        "state": pull["state"],
        "merged": pull["merged"],
        "draft": pull["draft"],
        "author": pull["user"]["login"],
        "into_branch": pull["base"]["ref"],
        "from_branch": pull["head"]["ref"],
        "added_lines": pull["additions"],
        "removed_lines": pull["deletions"],
        "changed_file_count": pull["changed_files"],
        "url": pull["html_url"],
        "checks": checks_summary(checks),
        "untrusted": UNTRUSTED,
        "text": clip_text(text(&pull["body"]), BODY_CHARS),
        "files": changed,
    })
}

pub(crate) fn file_or_folder(repo: &RepoName, path: &str, contents: &Value) -> Result<Value> {
    if let Some(entries) = contents.as_array() {
        let listed: Vec<Value> = entries
            .iter()
            .take(FOLDER_ENTRIES)
            .map(|entry| {
                json!({
                    "name": entry["name"],
                    "path": entry["path"],
                    "kind": entry["type"],
                    "size": entry["size"],
                })
            })
            .collect();
        return Ok(json!({
            "ok": true,
            "repo": repo.full(),
            "folder": path,
            "entries_not_shown": entries.len().saturating_sub(FOLDER_ENTRIES),
            "entries": listed,
        }));
    }
    match text(&contents["type"]) {
        "file" => {}
        "symlink" | "submodule" => {
            return Ok(json!({
                "ok": true,
                "repo": repo.full(),
                "path": path,
                "kind": contents["type"],
                "points_to": contents["target"].as_str().or(contents["submodule_git_url"].as_str()),
            }));
        }
        _ => {
            return Err(CallError::Failed(
                "GitHub sent an answer Sovra couldn't read.".into(),
            ));
        }
    }
    let encoded: String = text(&contents["content"])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if encoded.is_empty() && contents["size"].as_u64().unwrap_or(0) > 0 {
        return Err(CallError::Failed(
            "That file is too large for Sovra to read through GitHub.".into(),
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| CallError::Failed("GitHub sent a file Sovra couldn't read.".into()))?;
    let Ok(body) = String::from_utf8(bytes) else {
        return Ok(json!({
            "ok": true,
            "repo": repo.full(),
            "path": path,
            "kind": "binary file",
            "size": contents["size"],
            "note": "This is not a text file, so Sovra can't show what's inside.",
        }));
    };
    Ok(json!({
        "ok": true,
        "repo": repo.full(),
        "path": path,
        "size": contents["size"],
        "url": contents["html_url"],
        "untrusted": UNTRUSTED,
        "text": clip_text(&body, FILE_CHARS),
    }))
}
