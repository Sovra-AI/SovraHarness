//! Publisher check for a plugin repository. Runs the app's own **Add from GitHub**
//! decisions on a local checkout, so a repository that passes here is added the
//! same way once it is pushed.

use std::fs;
use std::path::Path;

use sovra_core::plugins::{
    FetchedFile, GitHubRepository, MAX_GITHUB_FILE_BYTES, MAX_GITHUB_RESOURCE_BYTES, PluginPackage,
    PluginStore as _, RepositorySnapshot, plan_repository,
};
use sovra_core::skills::SkillError;

use crate::{LocalPluginStore, io_error, read_bytes};

/// GitHub truncates larger trees, which the app then refuses.
const MAX_FILES: usize = 20_000;
/// Any valid commit: a local check has no pushed commit yet.
const LOCAL_COMMIT: &str = "0000000000000000000000000000000000000000";

/// What Sovra would add from the repository, and anything it would leave out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryCheck {
    pub package: PluginPackage,
    pub left_out: Vec<String>,
}

/// Checks `folder` as the repository `owner/name` would be added from GitHub.
/// `.git` is ignored; every other file counts, as it would once pushed.
pub fn check_repository(folder: &Path, repository: &str) -> Result<RepositoryCheck, SkillError> {
    let repository = GitHubRepository::parse(repository)?;
    let mut files = Vec::new();
    list(folder, "", &mut files)?;
    let tree: Vec<_> = files
        .iter()
        .map(|(path, size)| serde_json::json!({"path": path, "type": "blob", "size": size}))
        .collect();
    let tree = serde_json::to_vec(&serde_json::json!({"tree": tree, "truncated": false}))
        .map_err(|_| SkillError("The file list could not be prepared.".into()))?;
    let plan = plan_repository(&tree)?;
    let mut fetched = Vec::new();
    for (index, skill) in plan.skills.iter().enumerate() {
        let bytes = read_bytes(&folder.join(&skill.path), MAX_GITHUB_FILE_BYTES)
            .map_err(|error| SkillError(format!("{}: {error}", skill.path)))?;
        fetched.push(FetchedFile::Instructions(index, bytes));
        for (path, relative) in &skill.resources {
            let text = read_bytes(&folder.join(path), MAX_GITHUB_RESOURCE_BYTES)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            fetched.push(FetchedFile::Resource(index, relative.clone(), text));
        }
    }
    for path in &plan.manifests {
        let bytes = read_bytes(&folder.join(path), MAX_GITHUB_FILE_BYTES)
            .map_err(|error| SkillError(format!("{path}: {error}")))?;
        fetched.push(FetchedFile::Manifest(path.clone(), bytes));
    }
    // The parser never touches this folder; it only decodes `SKILL.md` text.
    let parser = LocalPluginStore::new(folder.join(".sovra-check-unused"));
    let snapshot = RepositorySnapshot::assemble(
        repository,
        LOCAL_COMMIT.into(),
        None,
        plan,
        fetched,
        |skill_folder, text| parser.read_skill_file(skill_folder, text),
    );
    let left_out = snapshot.left_out.clone();
    let (entry, bytes, _) = snapshot.package()?;
    Ok(RepositoryCheck {
        package: entry.verify_package(&bytes)?,
        left_out,
    })
}

/// Regular files under `directory` as `(slash/separated/path, size)`. Links are not
/// followed: GitHub serves a link's target path, not the file it points to.
fn list(directory: &Path, prefix: &str, files: &mut Vec<(String, u64)>) -> Result<(), SkillError> {
    for item in fs::read_dir(directory).map_err(|error| io_error(&error))? {
        let item = item.map_err(|error| io_error(&error))?;
        let Some(name) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let kind = item.file_type().map_err(|error| io_error(&error))?;
        let path = format!("{prefix}{name}");
        if kind.is_dir() {
            if prefix.is_empty() && name == ".git" {
                continue;
            }
            list(&item.path(), &format!("{path}/"), files)?;
        } else if kind.is_file() {
            if files.len() == MAX_FILES {
                return Err(SkillError(
                    "This repository has too many files for Sovra to read in one go.".into(),
                ));
            }
            let size = item.metadata().map_err(|error| io_error(&error))?.len();
            files.push((path, size));
        }
    }
    Ok(())
}
