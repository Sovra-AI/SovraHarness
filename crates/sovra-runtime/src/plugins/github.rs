//! Reads one public GitHub repository at a single commit. Requests are unsigned, so
//! private repositories and GitHub's hourly request limit are explained plainly.
//! File contents come from the commit-pinned raw address, so a branch moving during
//! the download cannot mix two versions.

use std::time::Duration;

use futures::{StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use sovra_core::plugins::{
    FetchedFile, GitHubHosts, GitHubPin, GitHubRepository, MAX_GITHUB_FILE_BYTES,
    MAX_GITHUB_RESOURCE_BYTES, PluginEntry, PluginStore, RepositorySnapshot, plan_repository,
    valid_commit,
};
use sovra_core::skills::SkillError;

const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_TREE_BYTES: usize = 8 * 1024 * 1024;
/// Skill folders can hold dozens of small files; a few at a time keeps adding quick
/// without opening many connections at once.
const PARALLEL_DOWNLOADS: usize = 6;

pub(super) struct Head {
    pub(super) commit: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct RepositoryInfo {
    default_branch: String,
    #[serde(default)]
    description: Option<String>,
}

/// One finished download, tagged with the skill (by plan index) it belongs to.
pub(super) struct Snapshot {
    pub(super) entry: PluginEntry,
    pub(super) bytes: Vec<u8>,
    pub(super) pin: GitHubPin,
    pub(super) left_out: Vec<String>,
}

/// An error sentence as a clause: "The skill name…." becomes "the skill name…".
pub(super) struct Reader {
    client: reqwest::Client,
    hosts: GitHubHosts,
}

impl Reader {
    pub(super) fn new(hosts: &GitHubHosts) -> Result<Self, SkillError> {
        // Redirects are refused so a moved repository is reported, not silently followed.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Sovra")
            .build()
            .map_err(|_| SkillError("The GitHub reader could not start.".into()))?;
        Ok(Self {
            client,
            hosts: hosts.clone(),
        })
    }

    /// The newest commit on the repository's default branch.
    pub(super) async fn head(&self, repository: &GitHubRepository) -> Result<Head, SkillError> {
        let base = ["repos", repository.owner.as_str(), repository.name.as_str()];
        let info: RepositoryInfo = serde_json::from_slice(
            &self
                .get(
                    address(&self.hosts.api, &base)?,
                    "application/vnd.github+json",
                    MAX_METADATA_BYTES,
                )
                .await?,
        )
        .map_err(|_| SkillError("GitHub returned unexpected repository details.".into()))?;
        let mut commit_path = base.to_vec();
        commit_path.push("commits");
        commit_path.extend(info.default_branch.split('/'));
        let commit = self
            .get(
                address(&self.hosts.api, &commit_path)?,
                "application/vnd.github.sha",
                MAX_METADATA_BYTES,
            )
            .await?;
        let commit = String::from_utf8(commit)
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| valid_commit(text))
            .ok_or_else(|| SkillError("GitHub returned an unexpected commit identifier.".into()))?;
        Ok(Head {
            commit,
            description: info.description,
        })
    }

    /// Reads the skill files and MCP manifests at `head` and builds the package.
    pub(super) async fn snapshot(
        &self,
        repository: &GitHubRepository,
        head: Head,
        store: &dyn PluginStore,
    ) -> Result<Snapshot, SkillError> {
        let mut tree = address(
            &self.hosts.api,
            &[
                "repos",
                &repository.owner,
                &repository.name,
                "git",
                "trees",
                &head.commit,
            ],
        )?;
        tree.query_pairs_mut().append_pair("recursive", "1");
        let plan = plan_repository(
            &self
                .get(tree, "application/vnd.github+json", MAX_TREE_BYTES)
                .await?,
        )?;
        let commit = head.commit.as_str();
        // Every file goes through one queue so a repository of many small skills is
        // not read one skill at a time. Instructions and manifests are required:
        // the first that fails stops the add. Skill files are optional extras, so a
        // failed or non-text one becomes `None` and is counted as left out.
        let mut jobs: Vec<
            futures::future::BoxFuture<'_, Result<(usize, FetchedFile), SkillError>>,
        > = Vec::new();
        for (index, skill) in plan.skills.iter().enumerate() {
            let path = skill.path.clone();
            let order = jobs.len();
            jobs.push(Box::pin(async move {
                let bytes = self
                    .file(repository, commit, &path, MAX_GITHUB_FILE_BYTES)
                    .await?;
                Ok((order, FetchedFile::Instructions(index, bytes)))
            }));
            for (path, relative) in &skill.resources {
                let (path, relative) = (path.clone(), relative.clone());
                let order = jobs.len();
                jobs.push(Box::pin(async move {
                    let text = self
                        .file(repository, commit, &path, MAX_GITHUB_RESOURCE_BYTES)
                        .await
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok());
                    Ok((order, FetchedFile::Resource(index, relative, text)))
                }));
            }
        }
        for path in &plan.manifests {
            let path = path.clone();
            let order = jobs.len();
            jobs.push(Box::pin(async move {
                let bytes = self
                    .file(repository, commit, &path, MAX_GITHUB_FILE_BYTES)
                    .await?;
                Ok((order, FetchedFile::Manifest(path, bytes)))
            }));
        }
        // Unordered, so one slow file does not hold the other slots while downloads
        // queued after it have already finished.
        let mut fetched: Vec<(usize, FetchedFile)> = futures::stream::iter(jobs)
            .buffer_unordered(PARALLEL_DOWNLOADS)
            .try_collect()
            .await?;

        // Back in queue order: each skill's instructions come before its files, and the
        // result does not depend on which download finished first.
        fetched.sort_unstable_by_key(|(order, _)| *order);
        let snapshot = RepositorySnapshot::assemble(
            repository.clone(),
            head.commit,
            head.description,
            plan,
            fetched.into_iter().map(|(_, fetched)| fetched),
            |folder, text| store.read_skill_file(folder, text),
        );
        let left_out = snapshot.left_out.clone();
        snapshot.package().map(|(entry, bytes, pin)| Snapshot {
            entry,
            bytes,
            pin,
            left_out,
        })
    }

    async fn file(
        &self,
        repository: &GitHubRepository,
        commit: &str,
        path: &str,
        limit: usize,
    ) -> Result<Vec<u8>, SkillError> {
        let mut segments = vec![repository.owner.as_str(), repository.name.as_str(), commit];
        segments.extend(path.split('/'));
        self.get(address(&self.hosts.raw, &segments)?, "text/plain", limit)
            .await
            .map_err(|error| SkillError(format!("{path}: {error}")))
    }

    async fn get(
        &self,
        url: reqwest::Url,
        accept: &str,
        limit: usize,
    ) -> Result<Vec<u8>, SkillError> {
        let response = self
            .client
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| {
                SkillError("GitHub could not be reached. Check your internet connection.".into())
            })?;
        let status = response.status().as_u16();
        match status {
            200..=299 => super::read_limited(response, limit).await,
            301 | 302 | 307 | 308 => Err(SkillError(
                "This repository has moved or been renamed. Paste its new address.".into(),
            )),
            404 => Err(SkillError(
                "Sovra couldn't find that repository on GitHub. Check the address. Private repositories can't be added yet."
                    .into(),
            )),
            403 | 429 => Err(SkillError(
                "GitHub is limiting how often this computer can ask. Wait an hour and try again."
                    .into(),
            )),
            _ => Err(SkillError(format!(
                "GitHub returned HTTP {status}. Try again later."
            ))),
        }
    }
}

/// Joins path segments onto a host, percent-encoding each one.
fn address(base: &str, segments: &[&str]) -> Result<reqwest::Url, SkillError> {
    let mut url = reqwest::Url::parse(base)
        .map_err(|_| SkillError("The GitHub address is not configured correctly.".into()))?;
    url.path_segments_mut()
        .map_err(|()| SkillError("The GitHub address is not configured correctly.".into()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::address;

    #[test]
    fn addresses_encode_each_segment_and_keep_the_base_path() {
        let url = address(
            "https://raw.example/base/",
            &["owner", "repo", "abc", "my skills", "SKILL.md"],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://raw.example/base/owner/repo/abc/my%20skills/SKILL.md"
        );
    }
}
