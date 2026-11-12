//! Everything an agent may ask of GitHub, one variant per tool, and the checks that
//! keep model-written names, paths and searches safe to put into a GitHub address.
//!
//! Writes are small and visible: a new issue or a comment, signed as the person, on
//! a repository they name. Nothing merges, pushes, closes or deletes, so a prompt
//! hidden in an issue cannot change code or remove anything.

/// A repository written `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoName {
    pub owner: String,
    pub name: String,
}

impl RepoName {
    /// Reads `owner/name`, or a `github.com/owner/name/…` link.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let text = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .unwrap_or(text);
        let text = text.strip_prefix("www.").unwrap_or(text);
        let text = text.strip_prefix("github.com/").unwrap_or(text);
        let mut parts = text.trim_matches('/').split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts
            .next()
            .unwrap_or_default()
            .trim_end_matches(".git")
            .to_owned();
        let owner_ok = (1..=39).contains(&owner.len())
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let name_ok = (1..=100).contains(&name.len())
            && name != "."
            && name != ".."
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if owner_ok && name_ok {
            Ok(Self {
                owner: owner.to_owned(),
                name,
            })
        } else {
            Err("`repo` must look like owner/name, for example octocat/hello-world.".into())
        }
    }

    pub fn full(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// A file or folder path inside a repository, with every segment checked.
/// Empty means the top folder.
pub fn clean_repo_path(path: &str) -> Result<String, String> {
    let path = path.trim().trim_matches('/');
    if path.is_empty() {
        return Ok(String::new());
    }
    if path.len() > 1_000 || path.contains('\\') || path.chars().any(char::is_control) {
        return Err("That path can't be read.".into());
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err("A path can't contain empty parts, `.` or `..`.".into());
    }
    Ok(segments.join("/"))
}

/// A branch, tag or commit name. `None` when blank (the default branch).
pub fn clean_reference(reference: Option<&str>) -> Result<Option<String>, String> {
    let Some(reference) = reference.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let ok = reference.len() <= 200
        && !reference.contains("..")
        && !reference.starts_with('/')
        && reference
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if ok {
        Ok(Some(reference.to_owned()))
    } else {
        Err("`ref` must be a branch, tag or commit name.".into())
    }
}

/// Which kind of item a search looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Issues,
    PullRequests,
}

/// GitHub's issue search needs to be told issues or pull requests; adds that unless
/// the query already says.
pub fn search_query(query: &str, kind: SearchKind) -> String {
    let query = query.trim();
    let says = query.split_whitespace().any(|word| {
        let word = word.to_ascii_lowercase();
        word == "is:issue"
            || word == "is:pr"
            || word == "is:pull-request"
            || word.starts_with("type:")
    });
    if says {
        return query.to_owned();
    }
    let qualifier = match kind {
        SearchKind::Issues => "is:issue",
        SearchKind::PullRequests => "is:pr",
    };
    if query.is_empty() {
        qualifier.to_owned()
    } else {
        format!("{query} {qualifier}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRequest {
    /// The person's repositories, most recently changed first; `query` filters by name.
    ListRepositories { query: Option<String>, limit: u32 },
    /// GitHub search words, e.g. `repo:octocat/hello-world is:open author:@me`.
    Search {
        query: String,
        kind: SearchKind,
        limit: u32,
    },
    /// An issue or pull request with its latest comments.
    ReadIssue { repo: RepoName, number: u64 },
    /// A pull request's changed files and whether its checks passed.
    ReadPullRequest { repo: RepoName, number: u64 },
    /// A file's text, or a folder's list, at `reference` (default branch when `None`).
    ReadFile {
        repo: RepoName,
        path: String,
        reference: Option<String>,
    },
    CreateIssue {
        repo: RepoName,
        title: String,
        body: String,
    },
    Comment {
        repo: RepoName,
        number: u64,
        body: String,
    },
}

impl GitHubRequest {
    /// Whether it adds something on GitHub (an issue or a comment).
    pub fn changes_things(&self) -> bool {
        match self {
            Self::CreateIssue { .. } | Self::Comment { .. } => true,
            Self::ListRepositories { .. }
            | Self::Search { .. }
            | Self::ReadIssue { .. }
            | Self::ReadPullRequest { .. }
            | Self::ReadFile { .. } => false,
        }
    }
}
