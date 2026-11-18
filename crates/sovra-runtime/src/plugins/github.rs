//! Reads one public GitHub repository at a single commit. Requests are unsigned, so
//! private repositories and GitHub's hourly request limit are explained plainly.
//! File contents come from the commit-pinned raw address, so a branch moving during
//! the download cannot mix two versions.

use std::time::Duration;

use futures::StreamExt as _;
use serde::Deserialize;
use sovra_core::plugins::{
    GitHubHosts, GitHubPin, GitHubRepository, MAX_GITHUB_FILE_BYTES, MAX_GITHUB_RESOURCE_BYTES,
    PluginEntry, PluginStore, RepositorySnapshot, plan_repository, valid_commit,
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

pub(super) struct Snapshot {
    pub(super) entry: PluginEntry,
    pub(super) bytes: Vec<u8>,
    pub(super) pin: GitHubPin,
    pub(super) left_out: Vec<String>,
}

/// An error sentence as a clause: "The skill name…." becomes "the skill name…".
fn reason(error: &SkillError) -> String {
    let text = error.0.trim().trim_end_matches('.');
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

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
        let mut skills = Vec::with_capacity(plan.skills.len());
        let mut left_out = plan.left_out;
        let commit = head.commit.as_str();
        for plan in &plan.skills {
            // Download failures stop the whole add; a skill that cannot be read is
            // left out and named so the rest of the repository is still usable.
            let folder = plan.folder.as_deref();
            let bytes = self
                .file(repository, &head.commit, &plan.path, MAX_GITHUB_FILE_BYTES)
                .await?;
            let label = sovra_core::plugins::skill_label(folder);
            let decoded = String::from_utf8(bytes)
                .map_err(|_| SkillError("it is not readable text".into()))
                .and_then(|text| store.read_skill_file(folder, &text));
            let mut skill = match decoded {
                Ok(skill) => skill,
                Err(error) => {
                    left_out.push(format!("{label} ({})", reason(&error)));
                    continue;
                }
            };
            // Skill files are optional extras: one that fails to download or is not
            // text is counted and left out rather than stopping the add.
            // Boxed so the spawned worker's future stays `Send` across the borrows.
            let pending: Vec<futures::future::BoxFuture<'_, (String, Option<String>)>> = plan
                .resources
                .iter()
                .map(|(path, relative)| {
                    let (path, relative) = (path.clone(), relative.clone());
                    Box::pin(async move {
                        let text = self
                            .file(repository, commit, &path, MAX_GITHUB_RESOURCE_BYTES)
                            .await
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok());
                        (relative, text)
                    }) as futures::future::BoxFuture<'_, _>
                })
                .collect();
            let downloads: Vec<_> = futures::stream::iter(pending)
                .buffered(PARALLEL_DOWNLOADS)
                .collect()
                .await;
            let mut skipped = plan.skipped_files;
            for (relative, text) in downloads {
                match text {
                    Some(text) => {
                        skill.resources.insert(relative, text);
                    }
                    None => skipped += 1,
                }
            }
            if skipped > 0 {
                left_out.push(sovra_core::plugins::files_left_out(folder, skipped));
            }
            skills.push(skill);
        }
        let mut manifests = Vec::with_capacity(plan.manifests.len());
        for path in plan.manifests {
            let bytes = self
                .file(repository, &head.commit, &path, MAX_GITHUB_FILE_BYTES)
                .await?;
            manifests.push((path, bytes));
        }
        RepositorySnapshot {
            repository: repository.clone(),
            commit: head.commit,
            description: head.description,
            skills,
            manifests,
            left_out: left_out.clone(),
        }
        .package()
        .map(|(entry, bytes, pin)| Snapshot {
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
