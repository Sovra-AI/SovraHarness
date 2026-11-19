//! Plugins taken straight from a public GitHub repository. Each install is pinned
//! to one commit; a newer commit is only taken after the person reviews what changed.
//! The repository's `SKILL.md` folders become skills and its MCP manifests become
//! suggested connections, assembled into an ordinary version 1 package.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    InstalledPlugin, PluginEntry, PluginPackage, PluginService, PluginSkill, check_download_url,
    check_update, error, short_text,
};
use crate::skills::{MAX_SKILL_BYTES, SkillError, valid_name};

/// Version 1 entries need a version; a GitHub plugin is identified by its commit.
pub const GITHUB_PLUGIN_VERSION: &str = "1.0.0";
/// Popular skill collections hold around twenty skills, more than the catalog limit
/// of 16. The 512 KB package and 256-skill library budgets still apply.
pub const MAX_GITHUB_SKILLS: usize = 32;
pub const MAX_GITHUB_FILE_BYTES: usize = 64 * 1024;
const MAX_SERVICES: usize = 8;
/// Skill files are read as text on demand, like an imported skill's resources.
pub const MAX_GITHUB_RESOURCE_BYTES: usize = 128 * 1024;
const MAX_RESOURCES_PER_SKILL: usize = 64;
/// Total skill-file bytes per repository, leaving room in the package for instructions.
const MAX_RESOURCE_TOTAL: u64 = 1536 * 1024;
/// Packages are stored and parsed whole, so they stay bounded even with skill files.
pub const MAX_GITHUB_PLUGIN_BYTES: usize = 4 * 1024 * 1024;
/// Files that are never text; downloading them would only be discarded.
const BINARY_EXTENSIONS: [&str; 22] = [
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "pdf", "zip", "gz", "tgz", "tar", "ttf",
    "otf", "woff", "woff2", "mp3", "mp4", "wav", "docx", "pptx", "xlsx",
];
/// Folders that hold dependencies or build output, never a repository's own skills.
const SKIPPED_FOLDERS: [&str; 4] = ["node_modules", "target", "vendor", "__pycache__"];

/// Where repository metadata and file contents are read from. Replaced in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubHosts {
    pub api: String,
    pub raw: String,
}

impl Default for GitHubHosts {
    fn default() -> Self {
        Self {
            api: "https://api.github.com".into(),
            raw: "https://raw.githubusercontent.com".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubPin {
    pub repository: GitHubRepository,
    pub commit: String,
}

impl GitHubRepository {
    /// Accepts `https://github.com/owner/name`, `github.com/owner/name(.git)` or `owner/name`.
    pub fn parse(input: &str) -> Result<Self, SkillError> {
        let mut text = input.trim();
        if let Some(cut) = text.find(['?', '#']) {
            text = &text[..cut];
        }
        for prefix in ["https://", "http://"] {
            if let Some(rest) = text.strip_prefix(prefix) {
                text = rest;
            }
        }
        text = text.strip_prefix("www.").unwrap_or(text);
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("github.com/") {
            text = &text["github.com/".len()..];
        } else if lower.contains('.') && lower.split('/').next().is_some_and(|h| h.contains('.')) {
            return Err(error(
                "Paste a GitHub repository address, like https://github.com/owner/repository.",
            ));
        }
        let text = text.trim_end_matches('/');
        let mut parts = text.split('/');
        let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(error(
                "Paste the repository's main address, like https://github.com/owner/repository.",
            ));
        };
        let name = name.strip_suffix(".git").unwrap_or(name);
        let repository = Self {
            owner: owner.into(),
            name: name.into(),
        };
        repository.validate()?;
        Ok(repository)
    }

    pub fn validate(&self) -> Result<(), SkillError> {
        let owner_ok = (1..=39).contains(&self.owner.len())
            && !self.owner.starts_with('-')
            && !self.owner.ends_with('-')
            && self
                .owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let name_ok = (1..=100).contains(&self.name.len())
            && self.name != "."
            && self.name != ".."
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if owner_ok && name_ok {
            Ok(())
        } else {
            Err(error(
                "That isn't a valid GitHub repository address. Check the owner and repository names.",
            ))
        }
    }

    pub fn web_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// A stable reverse-domain id. GitHub names are case-insensitive, so the id is too.
    pub fn plugin_id(&self) -> String {
        let owner = id_segment(&self.owner, 39);
        let budget = 128 - "io.github..".len() - owner.len();
        // `valid_name` caps each segment at 64 bytes.
        format!(
            "io.github.{owner}.{}",
            id_segment(&self.name, budget.min(64))
        )
    }
}

fn id_segment(value: &str, max: usize) -> String {
    let mut segment = String::new();
    for c in value.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            segment.push(c);
        } else if !segment.is_empty() && !segment.ends_with('-') {
            segment.push('-');
        }
    }
    segment.truncate(max);
    let segment = segment.trim_end_matches('-');
    if segment.is_empty() {
        "repo".into()
    } else {
        segment.into()
    }
}

pub fn valid_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl GitHubPin {
    pub fn validate(&self, installed: &InstalledPlugin) -> Result<(), SkillError> {
        self.repository.validate()?;
        if !valid_commit(&self.commit)
            || installed.catalog_url != self.repository.web_url()
            || installed.entry.id != self.repository.plugin_id()
        {
            return Err(error(
                "A plugin added from GitHub has an inconsistent repository or commit.",
            ));
        }
        Ok(())
    }

    pub fn short_commit(&self) -> &str {
        &self.commit[..7]
    }
}

/// What to download from a repository's file list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryPlan {
    pub skills: Vec<SkillPlan>,
    pub manifests: Vec<String>,
    /// Skills that cannot be added, with the reason, so the rest still can.
    pub left_out: Vec<String>,
}

/// How a skill is named in messages: its folder, or "the main skill" at the root.
pub fn skill_label(folder: Option<&str>) -> String {
    folder.map_or_else(|| "the main skill".into(), str::to_owned)
}

/// `true` for the package address Sovra gives a GitHub plugin.
pub(super) fn is_github_package_url(url: &str) -> bool {
    url.strip_prefix("https://github.com/")
        .and_then(|rest| rest.rsplit_once("/tree/"))
        .is_some_and(|(_, commit)| valid_commit(commit))
}

#[derive(Deserialize)]
struct Tree {
    tree: Vec<TreeItem>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeItem {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPlan {
    /// The skill's folder name; `None` is a `SKILL.md` at the repository root.
    pub folder: Option<String>,
    pub path: String,
    /// `(path in the repository, path inside the skill)` of text files to download.
    pub resources: Vec<(String, String)>,
    /// Files in the folder that are not downloaded: not text, too large, or over a limit.
    pub skipped_files: usize,
}

/// Reads GitHub's recursive tree response and keeps only skill files and MCP manifests.
pub fn plan_repository(tree_json: &[u8]) -> Result<RepositoryPlan, SkillError> {
    let tree: Tree = serde_json::from_slice(tree_json)
        .map_err(|_| error("GitHub returned an unexpected file list for this repository."))?;
    if tree.truncated {
        return Err(error(
            "This repository is too large to read in one go. Ask its owner for a smaller plugin repository.",
        ));
    }
    let mut paths: Vec<_> = tree
        .tree
        .into_iter()
        .filter(|item| item.kind == "blob")
        .map(|item| (item.path, item.size.unwrap_or(0)))
        .collect();
    paths.sort();
    let mut plan = RepositoryPlan::default();
    let mut names = BTreeSet::new();
    for (path, size) in &paths {
        let (path, size) = (path.clone(), *size);
        if path == ".mcp.json" || path == ".claude-plugin/plugin.json" {
            plan.manifests.push(path);
            continue;
        }
        let mut parts: Vec<&str> = path.split('/').collect();
        if parts.pop() != Some("SKILL.md")
            || parts.iter().any(|part| {
                (part.starts_with('.') && *part != ".claude") || SKIPPED_FOLDERS.contains(part)
            })
        {
            continue;
        }
        let folder = parts.last().map(|name| (*name).to_owned());
        if names.contains(&folder) {
            continue;
        }
        if size > MAX_GITHUB_FILE_BYTES as u64 {
            plan.left_out.push(format!(
                "{} (its instructions are larger than 64 KB)",
                skill_label(folder.as_deref())
            ));
            continue;
        }
        // The same skill copied into two places (for example `skills/` and
        // `.claude/skills/`) is one skill; the first usable path in sorted order wins.
        names.insert(folder.clone());
        plan.skills.push(SkillPlan {
            folder,
            path,
            resources: Vec::new(),
            skipped_files: 0,
        });
    }
    plan_resources(&mut plan, &paths);
    if plan.skills.len() > MAX_GITHUB_SKILLS {
        return Err(SkillError(format!(
            "This repository has {} skills. Sovra can add up to {MAX_GITHUB_SKILLS} from one repository.",
            plan.skills.len()
        )));
    }
    if plan.skills.is_empty() && plan.manifests.is_empty() {
        return Err(nothing_to_add(&plan.left_out));
    }
    Ok(plan)
}

/// Everything read from one commit, ready to become a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySnapshot {
    pub repository: GitHubRepository,
    pub commit: String,
    pub description: Option<String>,
    pub skills: Vec<PluginSkill>,
    /// `(path, bytes)` of `.mcp.json` and `.claude-plugin/plugin.json`.
    pub manifests: Vec<(String, Vec<u8>)>,
    /// Skills found but not usable, with the reason.
    pub left_out: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<serde_json::Value>,
}

/// One downloaded file of a [`RepositoryPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchedFile {
    /// The `SKILL.md` bytes of `plan.skills[index]`.
    Instructions(usize, Vec<u8>),
    /// A file of `plan.skills[index]`: its path inside the skill, and its text, or
    /// `None` when it could not be read as text.
    Resource(usize, String, Option<String>),
    /// `.mcp.json` or `.claude-plugin/plugin.json`, by path.
    Manifest(String, Vec<u8>),
}

/// "It is not readable text." becomes "it is not readable text", to follow a skill name.
fn reason(error: &SkillError) -> String {
    let text = error.0.trim().trim_end_matches('.');
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

impl RepositorySnapshot {
    /// Turns a plan's downloaded files into a snapshot. Shared by the app's GitHub
    /// reader and the publisher's local check, so both decide the same way. `files`
    /// must be in plan order (each skill's instructions before its files); a skill
    /// that cannot be read is left out and named, so the rest are still usable.
    pub fn assemble(
        repository: GitHubRepository,
        commit: String,
        description: Option<String>,
        plan: RepositoryPlan,
        files: impl IntoIterator<Item = FetchedFile>,
        read_skill: impl Fn(Option<&str>, &str) -> Result<PluginSkill, SkillError>,
    ) -> Self {
        let mut skills: Vec<Result<PluginSkill, String>> = Vec::with_capacity(plan.skills.len());
        let mut skipped: Vec<usize> = plan.skills.iter().map(|s| s.skipped_files).collect();
        let mut manifests = Vec::with_capacity(plan.manifests.len());
        let mut left_out = plan.left_out;
        for item in files {
            match item {
                FetchedFile::Instructions(index, bytes) => {
                    let folder = plan
                        .skills
                        .get(index)
                        .and_then(|skill| skill.folder.as_deref());
                    let decoded = String::from_utf8(bytes)
                        .map_err(|_| SkillError("it is not readable text".into()))
                        .and_then(|text| read_skill(folder, &text));
                    skills.push(
                        decoded.map_err(|error| {
                            format!("{} ({})", skill_label(folder), reason(&error))
                        }),
                    );
                }
                FetchedFile::Resource(index, relative, text) => {
                    match (skills.get_mut(index), text) {
                        (Some(Ok(skill)), Some(text)) => {
                            skill.resources.insert(relative, text);
                        }
                        (Some(Ok(_)), None) => skipped[index] += 1,
                        // The skill itself was left out, so its files go with it.
                        (Some(Err(_)) | None, _) => {}
                    }
                }
                FetchedFile::Manifest(path, bytes) => manifests.push((path, bytes)),
            }
        }
        // Named in skill order, as the person sees them listed.
        let mut kept = Vec::with_capacity(skills.len());
        for (index, skill) in skills.into_iter().enumerate() {
            match skill {
                Ok(skill) => {
                    if skipped[index] > 0 {
                        left_out.push(files_left_out(
                            plan.skills[index].folder.as_deref(),
                            skipped[index],
                        ));
                    }
                    kept.push(skill);
                }
                Err(reason) => left_out.push(reason),
            }
        }
        Self {
            repository,
            commit,
            description,
            skills: kept,
            manifests,
            left_out,
        }
    }

    /// Builds the package bytes and their entry. Only HTTPS servers become suggested
    /// connections; servers that start a program on this computer are left out.
    pub fn package(&self) -> Result<(PluginEntry, Vec<u8>, GitHubPin), SkillError> {
        self.repository.validate()?;
        if !valid_commit(&self.commit) {
            return Err(error("GitHub returned an unexpected commit identifier."));
        }
        let mut description = None;
        let mut services: Vec<PluginService> = Vec::new();
        for (path, bytes) in &self.manifests {
            let Ok(manifest) = serde_json::from_slice::<Manifest>(bytes) else {
                continue;
            };
            if path.ends_with("plugin.json") {
                description = description.or(manifest.description);
            }
            // `.mcp.json` lists servers under `mcpServers`; older files list them at the top.
            let servers = match manifest.mcp_servers {
                Some(servers) => servers,
                None if path == ".mcp.json" => {
                    serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null)
                }
                None => serde_json::Value::Null,
            };
            let Some(servers) = servers.as_object() else {
                continue;
            };
            for (name, server) in servers {
                let Some(address) = server.get("url").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if services.len() < MAX_SERVICES
                    && short_text(name, 120)
                    && check_download_url(address).is_ok()
                    && !services.iter().any(|known| &known.name == name)
                {
                    services.push(PluginService {
                        name: name.clone(),
                        address: address.into(),
                    });
                }
            }
        }
        if self.skills.is_empty() && services.is_empty() {
            return Err(nothing_to_add(&self.left_out));
        }
        let description = description
            .or_else(|| self.description.clone())
            .map(|text| clean_text(&text, 1000))
            .filter(|text| short_text(text, 1000))
            .unwrap_or_else(|| {
                format!(
                    "Skills from github.com/{}/{}.",
                    self.repository.owner, self.repository.name
                )
            });
        let package = PluginPackage {
            schema_version: 1,
            api_version: 1,
            id: self.repository.plugin_id(),
            version: GITHUB_PLUGIN_VERSION.into(),
            title: clean_text(
                &format!("{}/{}", self.repository.owner, self.repository.name),
                120,
            ),
            description,
            skills: self.skills.clone(),
            services,
        };
        let bytes = serde_json::to_vec(&package)
            .map_err(|_| error("The repository could not be turned into a plugin."))?;
        if bytes.len() > MAX_GITHUB_PLUGIN_BYTES {
            return Err(error(
                "This repository's skills are larger than 4 MB together, which is more than Sovra can add.",
            ));
        }
        let entry = PluginEntry::for_package(
            format!("{}/tree/{}", self.repository.web_url(), self.commit),
            &bytes,
        )?;
        let pin = GitHubPin {
            repository: self.repository.clone(),
            commit: self.commit.clone(),
        };
        Ok((entry, bytes, pin))
    }
}

pub(super) fn clean_text(value: &str, max: usize) -> String {
    let mut text: String = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// Turns a `SKILL.md` already decoded by the skills adapter into a plugin skill.
pub fn repository_skill(
    name: String,
    title: &str,
    description: &str,
    instructions: String,
) -> Result<PluginSkill, SkillError> {
    if !valid_name(&name) || name.len() > 32 {
        return Err(SkillError(format!(
            "The skill \"{name}\" needs a short lowercase name (32 characters at most) to be added."
        )));
    }
    Ok(PluginSkill {
        name,
        title: clean_text(title, 120),
        // YAML folded descriptions end with a newline.
        description: description.trim().into(),
        instructions,
        resources: BTreeMap::new(),
        example: String::new(),
    })
}

/// The folder a `SKILL.md` lives in; empty at the repository root.
fn skill_directory(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(directory, _)| directory)
}

/// Gives each skill the text files in its folder (not in a nested skill's folder),
/// within the per-file, per-skill and per-repository budgets. Smaller files are
/// chosen first across the whole repository, so a few large schema files in one
/// skill cannot use up the budget of every skill listed after it.
fn plan_resources(plan: &mut RepositoryPlan, paths: &[(String, u64)]) {
    let directories: Vec<String> = plan
        .skills
        .iter()
        .map(|skill| skill_directory(&skill.path).to_owned())
        .collect();
    let mut candidates = Vec::new();
    for (path, size) in paths {
        if path.ends_with("/SKILL.md") || path == "SKILL.md" {
            continue;
        }
        // The deepest skill folder containing the file owns it.
        let owner = directories
            .iter()
            .enumerate()
            .filter_map(|(index, directory)| {
                let relative = if directory.is_empty() {
                    Some(path.as_str())
                } else {
                    path.strip_prefix(directory.as_str())
                        .and_then(|rest| rest.strip_prefix('/'))
                };
                relative.map(|relative| (index, directory.len(), relative))
            })
            .max_by_key(|(_, depth, _)| *depth);
        let Some((index, _, relative)) = owner else {
            continue;
        };
        if !valid_resource_path(relative)
            || relative
                .split('/')
                .any(|part| SKIPPED_FOLDERS.contains(&part))
        {
            continue;
        }
        let binary = relative.rsplit_once('.').is_some_and(|(_, extension)| {
            BINARY_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        });
        if binary || *size > MAX_GITHUB_RESOURCE_BYTES as u64 {
            plan.skills[index].skipped_files += 1;
        } else {
            candidates.push((*size, path.clone(), index, relative.to_owned()));
        }
    }
    candidates.sort();
    let mut total = 0_u64;
    for (size, path, index, relative) in candidates {
        let skill = &mut plan.skills[index];
        if skill.resources.len() >= MAX_RESOURCES_PER_SKILL || total + size > MAX_RESOURCE_TOTAL {
            skill.skipped_files += 1;
            continue;
        }
        total += size;
        skill.resources.push((path, relative));
    }
    for skill in &mut plan.skills {
        skill.resources.sort();
    }
}

/// Names files a skill could not keep: pictures and other non-text files, large
/// files, or more than the limits allow.
pub fn files_left_out(folder: Option<&str>, count: usize) -> String {
    format!(
        "{}: {count} file{} that {} not text, very large, or over the limit",
        skill_label(folder),
        if count == 1 { "" } else { "s" },
        if count == 1 { "is" } else { "are" }
    )
}

/// A relative path inside a skill: no hidden parts, no `..`, no absolute or Windows paths.
pub fn valid_resource_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.'))
}

pub(super) fn check_resources(resources: &BTreeMap<String, String>) -> Result<(), SkillError> {
    if resources.len() > MAX_RESOURCES_PER_SKILL
        || resources.iter().any(|(path, text)| {
            !valid_resource_path(path) || text.len() > MAX_GITHUB_RESOURCE_BYTES
        })
    {
        return Err(error(
            "A plugin skill has files with unsafe paths, too many files, or a file that is too large.",
        ));
    }
    Ok(())
}

/// Tells the model which files it can read, when that still fits the skill's size limit.
pub(super) fn with_file_list(instructions: &str, resources: &BTreeMap<String, String>) -> String {
    if resources.is_empty() {
        return instructions.into();
    }
    let list = resources.keys().cloned().collect::<Vec<_>>().join(", ");
    let combined = format!(
        "{instructions}\n\nFiles in this skill (read one with load_skill, giving this skill's name and the file's path as resource): {list}"
    );
    if combined.len() <= MAX_SKILL_BYTES {
        combined
    } else {
        instructions.into()
    }
}

/// What an update would change, in the words shown before the person accepts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub connections_added: Vec<String>,
    pub connections_removed: Vec<String>,
    /// Skills in the newer commit that cannot be added, with the reason.
    pub left_out: Vec<String>,
}

/// Explains an empty result, naming the skills that were found but could not be used.
pub fn nothing_to_add(left_out: &[String]) -> SkillError {
    if left_out.is_empty() {
        error("This repository has no SKILL.md files or MCP connection settings for Sovra to add.")
    } else {
        SkillError(format!(
            "None of this repository's skills could be added: {}.",
            left_out.join("; ")
        ))
    }
}

impl PluginChanges {
    pub fn between(old: &PluginPackage, new: &PluginPackage) -> Self {
        let mut changes = Self::default();
        for skill in &new.skills {
            match old.skills.iter().find(|known| known.name == skill.name) {
                None => changes.added.push(skill.title.clone()),
                Some(known) if known != skill => changes.changed.push(skill.title.clone()),
                Some(_) => {}
            }
        }
        for skill in &old.skills {
            if !new.skills.iter().any(|next| next.name == skill.name) {
                changes.removed.push(skill.title.clone());
            }
        }
        for service in &new.services {
            if !old.services.contains(service) {
                changes
                    .connections_added
                    .push(format!("{}: {}", service.name, service.address));
            }
        }
        for service in &old.services {
            if !new.services.contains(service) {
                changes
                    .connections_removed
                    .push(format!("{}: {}", service.name, service.address));
            }
        }
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.connections_added.is_empty()
            && self.connections_removed.is_empty()
    }
}

/// A replacement keeps the plugin's identity: catalog plugins follow version rules,
/// GitHub plugins must come from the same repository at a different commit.
pub fn check_replacement(
    installed: &InstalledPlugin,
    candidate: &InstalledPlugin,
) -> Result<(), SkillError> {
    match (&installed.github, &candidate.github) {
        (None, None) => check_update(installed, &candidate.entry, &candidate.catalog_url),
        (Some(old), Some(new)) => {
            if old.repository.plugin_id() != new.repository.plugin_id()
                || installed.catalog_url != candidate.catalog_url
            {
                return Err(error(
                    "This plugin came from a different repository. Its installed copy was kept.",
                ));
            }
            if old.commit == new.commit {
                return Err(error("This plugin is already up to date."));
            }
            Ok(())
        }
        (Some(_), None) | (None, Some(_)) => Err(error(
            "A plugin with this name is already installed from somewhere else. Its installed copy was kept.",
        )),
    }
}

#[cfg(test)]
mod tests;
