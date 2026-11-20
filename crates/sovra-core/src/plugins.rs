//! Downloadable, declarative plugin packages. A catalog is a static JSON file,
//! suitable for GitHub Pages; installing instructions never grants tool access.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::skills::{EXAMPLE_METADATA_KEY, Skill, SkillCatalog, SkillError, SkillKind, valid_name};

mod code;
mod github;
pub use code::{
    CODE_API_VERSION, CODE_MANIFEST_PATH, CODE_MODULE_PATH, CodeChanges, CodeFetch, CodeFetchReply,
    CodeHost, CodeManifest, LoadedCode, MAX_FETCH_BODY_BYTES, MAX_FETCH_REPLY_BYTES,
    MAX_FETCHES_PER_CALL, MAX_MODULE_BYTES, MAX_NETWORK_HOSTS, MAX_TOOLS, PLUGIN_SERVICE_PREFIX,
    PluginCode, PluginCodeEngine, PluginTool, check_fetch, code_service_id, public_address,
    valid_host,
};
pub use github::{
    FetchedFile, GITHUB_PLUGIN_VERSION, GitHubHosts, GitHubPin, GitHubRepository,
    MAX_GITHUB_FILE_BYTES, MAX_GITHUB_PLUGIN_BYTES, MAX_GITHUB_RESOURCE_BYTES, MAX_GITHUB_SKILLS,
    PluginChanges, RepositoryPlan, RepositorySnapshot, SkillPlan, check_replacement,
    files_left_out, nothing_to_add, plan_repository, repository_skill, skill_label, valid_commit,
    valid_resource_path,
};

pub const MAX_CATALOG_BYTES: usize = 256 * 1024;
pub const MAX_PLUGIN_BYTES: usize = 512 * 1024;
pub const MAX_PLUGIN_LIBRARY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PLUGINS: usize = 4096;
pub const PLUGIN_METADATA_KEY: &str = "sovra-plugin";
/// The open-source starting point for making a plugin; also a safe one to try.
pub const PLUGIN_TEMPLATE: &str = "https://github.com/Sovra-AI/sovra-plugin-template";

/// Lowercase hex of the SHA-256 digest. `GenericArray` no longer implements `LowerHex`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = vec![0; digest.len() * 2];
    for (index, byte) in digest.iter().enumerate() {
        encoded[index * 2] = HEX[usize::from(byte >> 4)];
        encoded[index * 2 + 1] = HEX[usize::from(byte & 0x0f)];
    }
    String::from_utf8(encoded).expect("hex digits are ascii")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub api_version: u32,
    pub package_url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCatalog {
    pub schema_version: u32,
    pub plugins: Vec<PluginEntry>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub previous: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSkill {
    pub name: String,
    pub title: String,
    pub description: String,
    pub instructions: String,
    /// Text files from the skill's folder, by relative path. Only plugins added
    /// from GitHub carry them; catalog packages (format version 1) must not.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, String>,
    /// An everyday request the Skills page shows, from `sovra-example` in `SKILL.md`.
    /// Left out when empty so packages without one keep their checksums.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub example: String,
}

impl PluginSkill {
    /// Sets the Skills page example, on one line and at most 200 bytes.
    #[must_use]
    pub fn with_example(mut self, example: &str) -> Self {
        self.example = github::clean_text(example, MAX_EXAMPLE_BYTES);
        self
    }
}

/// Room for one plain sentence on a Skills page card.
const MAX_EXAMPLE_BYTES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginService {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPackage {
    pub schema_version: u32,
    pub title: String,
    pub description: String,
    pub api_version: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub skills: Vec<PluginSkill>,
    #[serde(default)]
    pub services: Vec<PluginService>,
    /// Code that runs in a sealed box. Only plugins added from GitHub carry it, at
    /// `api_version` 2; left out when absent so earlier packages keep their checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<PluginCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledPlugin {
    pub entry: PluginEntry,
    pub catalog_url: String,
    pub enabled: bool,
    /// Set for plugins added from a GitHub repository; `catalog_url` is then the repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHubPin>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLibrary {
    pub catalog_url: Option<String>,
    pub installed: Vec<InstalledPlugin>,
}

/// Blocking persistence port; the runtime owns scheduling and network downloads.
pub trait PluginStore: Send + Sync + std::fmt::Debug {
    fn load(&self) -> Result<PluginLibrary, SkillError>;
    fn save(&self, library: &PluginLibrary) -> Result<(), SkillError>;
    fn package(&self, entry: &PluginEntry) -> Result<PluginPackage, SkillError>;
    fn cache(&self, entry: &PluginEntry, bytes: &[u8]) -> Result<(), SkillError>;
    /// Deletes stored packages that no entry in `library` refers to (removed plugins and
    /// replaced versions) and returns how many were deleted.
    fn remove_unused(&self, library: &PluginLibrary) -> Result<usize, SkillError>;
    /// Decodes one Agent Skills `SKILL.md`. `folder` is `None` at a repository root,
    /// where the skill's own name is used instead of a folder name.
    fn read_skill_file(
        &self,
        folder: Option<&str>,
        content: &str,
    ) -> Result<PluginSkill, SkillError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCommand {
    Browse {
        url: String,
    },
    /// Only entries from the runtime's most recently loaded catalog are installable.
    Install {
        id: String,
    },
    NextPage,
    PreviousPage,
    Inspect {
        id: String,
    },
    /// Turning a plugin with code on needs [`PluginCommand::AllowCode`] instead.
    SetEnabled {
        id: String,
        enabled: bool,
    },
    /// Turns on a plugin with code after the person saw what it can reach. Refused
    /// unless `package_sha256` is still the installed package, so an update that
    /// lands between looking and allowing is never switched on unseen.
    AllowCode {
        id: String,
        package_sha256: String,
    },
    /// Installs a public repository's skills at its latest commit, switched off.
    AddFromGitHub {
        address: String,
    },
    /// Reads the newest commit of a GitHub plugin and reports what would change.
    CheckForUpdate {
        id: String,
    },
    /// Takes the update most recently reported by `CheckForUpdate` for this plugin.
    ApplyUpdate {
        id: String,
    },
    /// Takes a plugin off this computer. Connections set up from it are kept.
    Remove {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEvent {
    Loaded(PluginLibrary),
    Catalog {
        url: String,
        catalog: PluginCatalog,
    },
    Details {
        id: String,
        package: PluginPackage,
    },
    /// `left_out` names skills the repository has that could not be added, and why.
    Added {
        id: String,
        package: PluginPackage,
        left_out: Vec<String>,
    },
    UpdateReady {
        id: String,
        changes: PluginChanges,
    },
    UpToDate {
        id: String,
    },
    /// Sent after the `Loaded` library that no longer lists the plugin.
    Removed {
        id: String,
        title: String,
    },
    Busy,
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginState {
    pub library: PluginLibrary,
    pub catalog: Option<PluginCatalog>,
    pub catalog_url: String,
    pub details: Option<(String, PluginPackage)>,
    /// An update the person has been shown but not yet accepted.
    pub update: Option<(String, PluginChanges)>,
    pub busy: bool,
    pub notice: String,
    /// The notice reports something that went wrong, so it is shown as a warning.
    pub problem: bool,
}

impl PluginState {
    pub fn apply(&mut self, event: &PluginEvent) {
        self.problem = matches!(event, PluginEvent::Error(_));
        match event {
            PluginEvent::Loaded(library) => {
                self.library = library.clone();
                self.details = None;
                self.update = None;
                if self.catalog.is_none() {
                    self.catalog_url = library.catalog_url.clone().unwrap_or_default();
                }
                self.busy = false;
                // The cards show the new state; events with news set it after this.
                self.notice.clear();
            }
            PluginEvent::Catalog { url, catalog } => {
                self.catalog_url.clone_from(url);
                self.catalog = Some(catalog.clone());
                self.busy = false;
                self.notice =
                    "Choose a plugin to download. Only install from publishers you trust.".into();
            }
            PluginEvent::Details { id, package } => {
                self.details = Some((id.clone(), package.clone()));
                self.busy = false;
                self.notice.clear();
            }
            PluginEvent::Added {
                id,
                package,
                left_out,
            } => {
                self.details = Some((id.clone(), package.clone()));
                self.busy = false;
                let skipped = if left_out.is_empty() {
                    String::new()
                } else {
                    format!(" Left out: {}.", left_out.join("; "))
                };
                let tools = package.code.as_ref().map_or(0, |code| code.tools.len());
                let contents = match (package.skills.len(), tools) {
                    (skills, 0) => plural(skills, "skill"),
                    (0, tools) => plural(tools, "tool"),
                    (skills, tools) => {
                        format!("{} and {}", plural(skills, "skill"), plural(tools, "tool"))
                    }
                };
                self.notice = format!(
                    "Added {} with {contents}. It stays off until you choose Turn on.{skipped}",
                    package.title,
                );
            }
            PluginEvent::UpdateReady { id, changes } => {
                self.update = Some((id.clone(), changes.clone()));
                self.busy = false;
                self.notice =
                    "A newer version is ready. Check what changes, then choose Update.".into();
            }
            PluginEvent::UpToDate { id } => {
                self.update = None;
                self.busy = false;
                self.notice = format!("{} is already up to date.", self.title(id));
            }
            PluginEvent::Removed { id, title } => {
                if self.details.as_ref().is_some_and(|(shown, _)| shown == id) {
                    self.details = None;
                }
                if self.update.as_ref().is_some_and(|(shown, _)| shown == id) {
                    self.update = None;
                }
                self.busy = false;
                self.notice = format!(
                    "Removed {title}. Its skills are gone from Sovra. Connections you set up from it stay in Connections."
                );
            }
            PluginEvent::Busy => {
                self.busy = true;
                self.notice = "Updating plugins…".into();
            }
            PluginEvent::Error(message) => {
                self.busy = false;
                self.notice.clone_from(message);
            }
        }
    }
}

impl PluginState {
    pub fn title(&self, id: &str) -> String {
        self.library
            .installed
            .iter()
            .find(|plugin| plugin.entry.id == id)
            .map_or_else(|| "This plugin".into(), |plugin| plugin.entry.title.clone())
    }
}

/// GitHub plugins can carry skill files, so their packages may be larger.
pub fn package_limit(package_url: &str) -> usize {
    if github::is_github_package_url(package_url) {
        MAX_GITHUB_PLUGIN_BYTES
    } else {
        MAX_PLUGIN_BYTES
    }
}

pub fn check_download_url(value: &str) -> Result<(), SkillError> {
    let address = crate::mcp::check_service_address(value)
        .map_err(|_| error("Use an HTTPS address for the plugin catalog and packages."))?;
    if value.len() > 2048 || value != value.trim() || address.url.contains('?') {
        return Err(error(
            "Plugin addresses cannot contain query strings or surrounding spaces.",
        ));
    }
    Ok(())
}

impl PluginCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, SkillError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(error("The plugin catalog is too large."));
        }
        let catalog: Self = serde_json::from_slice(bytes)
            .map_err(|_| error("The address did not return a valid plugin catalog."))?;
        if catalog.schema_version != 1 || catalog.plugins.len() > 128 {
            return Err(error(
                "Unsupported plugin catalog version or too many entries.",
            ));
        }
        for url in catalog.next.iter().chain(catalog.previous.iter()) {
            check_download_url(url)?;
        }
        let mut ids = BTreeSet::new();
        for entry in &catalog.plugins {
            entry.validate()?;
            if !ids.insert(&entry.id) {
                return Err(error("The catalog repeats a plugin identifier."));
            }
        }
        Ok(catalog)
    }
}

impl PluginEntry {
    pub fn validate(&self) -> Result<(), SkillError> {
        if !valid_plugin_id(&self.id)
            || !short_text(&self.title, 120)
            || !short_text(&self.description, 1000)
            || !valid_version(&self.version)
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(error(
                "The plugin has invalid names, version, description or checksum.",
            ));
        }
        check_download_url(&self.package_url)
    }

    /// Builds a catalog entry from exact package bytes, for static catalog publishers.
    pub fn for_package(package_url: String, bytes: &[u8]) -> Result<Self, SkillError> {
        if bytes.len() > package_limit(&package_url) {
            return Err(error("The plugin package is too large."));
        }
        let package: PluginPackage =
            serde_json::from_slice(bytes).map_err(|_| error("Invalid plugin package."))?;
        let entry = Self {
            id: package.id.clone(),
            title: package.title.clone(),
            description: package.description.clone(),
            version: package.version.clone(),
            api_version: package.api_version,
            package_url,
            sha256: sha256_hex(bytes),
        };
        entry.validate()?;
        package.validate(&entry)?;
        Ok(entry)
    }

    pub fn verify_package(&self, bytes: &[u8]) -> Result<PluginPackage, SkillError> {
        self.validate()?;
        if bytes.len() > package_limit(&self.package_url) {
            return Err(error("The plugin download is too large."));
        }
        if sha256_hex(bytes) != self.sha256 {
            return Err(error(
                "The plugin download does not match its catalog checksum. Nothing was installed.",
            ));
        }
        let package: PluginPackage = serde_json::from_slice(bytes)
            .map_err(|_| error("The download is not a supported plugin package."))?;
        package.validate(self)?;
        Ok(package)
    }
}

/// The start of every skill command from plugin `id`. A skill file can then be found
/// by opening only that plugin's package instead of every switched-on one.
pub fn skill_command_prefix(id: &str) -> String {
    format!("plugin-{}-", &sha256_hex(id.as_bytes())[..16])
}

impl PluginPackage {
    fn validate(&self, entry: &PluginEntry) -> Result<(), SkillError> {
        let github = github::is_github_package_url(&entry.package_url);
        // Version 1 is skills and connections; version 2 adds code, which only a
        // GitHub repository can bring.
        let api_matches = match &self.code {
            None => self.api_version == 1,
            Some(_) => self.api_version == CODE_API_VERSION && github,
        };
        if self.schema_version != 1
            || !api_matches
            || self.api_version != entry.api_version
            || self.title != entry.title
            || self.description != entry.description
            || self.id != entry.id
            || self.version != entry.version
            || self.skills.len()
                > if github::is_github_package_url(&entry.package_url) {
                    MAX_GITHUB_SKILLS
                } else {
                    16
                }
            || self.services.len() > 8
            || (self.skills.is_empty() && self.services.is_empty() && self.code.is_none())
        {
            return Err(error(
                "The package does not match its catalog entry or has unsupported contents.",
            ));
        }
        let mut names = BTreeSet::new();
        for skill in &self.skills {
            if !valid_name(&skill.name) || skill.name.len() > 32 || !names.insert(&skill.name) {
                return Err(error(
                    "Plugin skill names must be unique, short lowercase names.",
                ));
            }
            if !skill.resources.is_empty() && !github {
                return Err(error(
                    "Catalog plugin packages cannot include skill files in format version 1.",
                ));
            }
            github::check_resources(&skill.resources)?;
            if !skill.example.is_empty() && !short_text(&skill.example, MAX_EXAMPLE_BYTES) {
                return Err(error(&format!(
                    "The example for \"{}\" must be one short line.",
                    skill.name
                )));
            }
        }
        SkillCatalog::new(self.skills(true))?;
        for service in &self.services {
            if !short_text(&service.name, 120) {
                return Err(error("A plugin service needs a short name."));
            }
            check_download_url(&service.address)?;
        }
        if let Some(code) = &self.code {
            code.validate()?;
        }
        Ok(())
    }

    /// The command name a package skill is listed under.
    pub fn skill_command(&self, skill: &PluginSkill) -> String {
        format!("{}{}", skill_command_prefix(&self.id), skill.name)
    }

    /// A skill file by the command name the model used and the file's path.
    pub fn resource(&self, command: &str, path: &str) -> Option<&str> {
        let path = path.strip_prefix("./").unwrap_or(path);
        let name = command.strip_prefix(&skill_command_prefix(&self.id))?;
        self.skills
            .iter()
            .find(|skill| skill.name == name)
            .and_then(|skill| skill.resources.get(path))
            .map(String::as_str)
    }

    pub fn skills(&self, enabled: bool) -> Vec<Skill> {
        self.skills
            .iter()
            .map(|skill| Skill {
                name: self.skill_command(skill),
                title: skill.title.clone(),
                description: skill.description.clone(),
                instructions: github::with_file_list(&skill.instructions, &skill.resources),
                category: "Plugins".into(),
                aliases: Vec::new(),
                kind: SkillKind::Procedure,
                tools: Vec::new(),
                argument_hint: String::new(),
                requires_input: false,
                enabled,
                auto_apply: true,
                builtin: false,
                license: None,
                compatibility: None,
                metadata: BTreeMap::from_iter(
                    [(PLUGIN_METADATA_KEY.into(), self.id.clone())]
                        .into_iter()
                        .chain(
                            (!skill.example.is_empty())
                                .then(|| (EXAMPLE_METADATA_KEY.into(), skill.example.clone())),
                        ),
                ),
            })
            .collect()
    }
}

impl PluginLibrary {
    /// Switches a plugin on or off. A plugin with code is only switched on through
    /// [`PluginLibrary::allow_code`], so every way in asks the person first.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), SkillError> {
        let plugin = self.installed_mut(id)?;
        if enabled && plugin.entry.api_version == CODE_API_VERSION {
            return Err(error(
                "This plugin runs its own code. Choose Turn on in Plugins to see what it can reach first.",
            ));
        }
        plugin.enabled = enabled;
        Ok(())
    }

    /// Switches a plugin with code on after the person saw the package with this
    /// checksum.
    pub fn allow_code(&mut self, id: &str, package_sha256: &str) -> Result<(), SkillError> {
        let plugin = self.installed_mut(id)?;
        if plugin.entry.sha256 != package_sha256 {
            return Err(error(
                "This plugin changed after you looked at it. Look at what it can reach again.",
            ));
        }
        plugin.enabled = true;
        Ok(())
    }

    fn installed_mut(&mut self, id: &str) -> Result<&mut InstalledPlugin, SkillError> {
        self.installed
            .iter_mut()
            .find(|plugin| plugin.entry.id == id)
            .ok_or_else(|| error("This plugin is no longer installed."))
    }

    /// File names (`<sha256>.json`) of the packages this library still uses. Anything
    /// else in the package folder is safe to delete.
    pub fn package_files(&self) -> BTreeSet<String> {
        self.installed
            .iter()
            .map(|plugin| format!("{}.json", plugin.entry.sha256))
            .collect()
    }

    /// Whether `name` has the shape of a stored package file, so collection never
    /// touches the index, staging files or anything a person put there.
    pub fn is_package_file(name: &str) -> bool {
        name.strip_suffix(".json").is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
    }

    pub fn validate(&self) -> Result<(), SkillError> {
        if self.installed.len() > MAX_PLUGINS {
            return Err(error("At most 4096 plugins can be installed."));
        }
        if let Some(url) = &self.catalog_url {
            check_download_url(url)?;
        }
        let mut ids = BTreeSet::new();
        for installed in &self.installed {
            installed.entry.validate()?;
            check_download_url(&installed.catalog_url)?;
            if let Some(pin) = &installed.github {
                pin.validate(installed)?;
            }
            if !ids.insert(&installed.entry.id) {
                return Err(error("The plugin library repeats a plugin identifier."));
            }
        }
        Ok(())
    }
}

/// "1 skill", "3 tools".
fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn short_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn valid_version(value: &str) -> bool {
    version_parts(value).is_some()
}

fn version_parts(value: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
        })
    {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

fn valid_plugin_id(value: &str) -> bool {
    value.len() <= 128 && value.split('.').count() >= 3 && value.split('.').all(valid_name)
}

/// An installed identity stays bound to its chosen catalog. Versions are immutable.
pub fn check_update(
    installed: &InstalledPlugin,
    candidate: &PluginEntry,
    catalog_url: &str,
) -> Result<(), SkillError> {
    candidate.validate()?;
    if installed.entry.id != candidate.id || installed.catalog_url != catalog_url {
        return Err(error(
            "This plugin belongs to a different catalog. Its installed copy was kept.",
        ));
    }
    if version_parts(&candidate.version) < version_parts(&installed.entry.version)
        || (candidate.version == installed.entry.version
            && candidate.sha256 != installed.entry.sha256)
    {
        return Err(error(
            "A plugin version cannot be replaced or downgraded. Ask its publisher for a new version.",
        ));
    }
    Ok(())
}

fn error(message: &str) -> SkillError {
    SkillError(message.into())
}

#[cfg(test)]
mod tests;
