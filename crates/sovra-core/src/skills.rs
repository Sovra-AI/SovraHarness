//! Skill discovery and explicit invocation. Instructions are kept separate from user text;
//! neither a skill nor a slash alias grants capabilities to the executing agent.

mod builtin;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub use builtin::{SkillCardCopy, builtin_skills};

pub const MAX_SKILL_BYTES: usize = 64 * 1024;
pub const MAX_SKILLS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillAction {
    AddFiles,
    IndexFolder,
    Help,
    NewChat,
    Models,
    Knowledge,
    Schedules,
    Agents,
    TeamChat,
    Artifacts,
    Connections,
    /// Opens Connections on the card that asks for a web address.
    ConnectService,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillKind {
    Action(SkillAction),
    Procedure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub kind: SkillKind,
    pub instructions: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub argument_hint: String,
    #[serde(default)]
    pub requires_input: bool,
    pub enabled: bool,
    pub builtin: bool,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SkillError(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillListing {
    pub skills: Vec<Skill>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCommand {
    Refresh,
    Save(Box<Skill>),
    Import { directory: std::path::PathBuf },
    Remove { name: String },
    SetEnabled { name: String, enabled: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillEvent {
    Loaded {
        skills: Vec<Skill>,
        issues: Vec<String>,
    },
    Error(String),
}

/// Persistence is supplied by the composition root; calls run on a blocking worker.
pub trait SkillStore: Send + Sync + std::fmt::Debug {
    fn list(&self) -> Result<SkillListing, SkillError>;
    fn save(&self, skill: &Skill) -> Result<(), SkillError>;
    fn import(&self, directory: &std::path::Path) -> Result<Skill, SkillError>;
    fn remove(&self, name: &str) -> Result<(), SkillError>;
    fn resource(&self, name: &str, relative_path: &str) -> Result<String, SkillError>;
}

pub type SkillResourceFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<String, SkillError>> + Send>>;

/// Model-side reads are scheduled by the runtime, keeping filesystem IO off its workers.
pub trait SkillResources: Send + Sync + std::fmt::Debug {
    fn read(&self, name: String, path: String) -> SkillResourceFuture;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalog {
    skills: Vec<Skill>,
}

impl Default for SkillCatalog {
    fn default() -> Self {
        Self::new(builtin_skills()).expect("the built-in catalogue is validated by tests")
    }
}

impl SkillCatalog {
    pub fn new(skills: Vec<Skill>) -> Result<Self, SkillError> {
        if skills.len() > MAX_SKILLS {
            return Err(SkillError(format!(
                "At most {MAX_SKILLS} skills can be installed."
            )));
        }
        let mut names = BTreeSet::new();
        for skill in &skills {
            skill.validate()?;
            for name in std::iter::once(&skill.name).chain(&skill.aliases) {
                if !names.insert(name) {
                    return Err(SkillError(format!(
                        "The command /{name} is already in use."
                    )));
                }
            }
        }
        Ok(Self { skills })
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn apply(&mut self, event: &SkillEvent) -> Result<Vec<String>, SkillError> {
        match event {
            SkillEvent::Loaded { skills, issues } => {
                *self = Self::new(skills.clone())?;
                Ok(issues.clone())
            }
            SkillEvent::Error(message) => Err(SkillError(message.clone())),
        }
    }

    pub fn find(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| {
            skill.name.eq_ignore_ascii_case(name)
                || skill
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(name))
        })
    }

    /// Stable SRX ranking: exact, prefix, alias, title, substring, description.
    pub fn search(&self, query: &str) -> Vec<&Skill> {
        self.search_all(query)
            .into_iter()
            .filter(|skill| skill.enabled)
            .collect()
    }

    pub fn search_all(&self, query: &str) -> Vec<&Skill> {
        let query = query.trim().to_lowercase();
        let mut matches: Vec<_> = self
            .skills
            .iter()
            .filter_map(|skill| score(skill, &query).map(|score| (score, skill)))
            .collect();
        matches.sort_by_key(|(score, _)| *score);
        matches.into_iter().map(|(_, skill)| skill).collect()
    }

    /// Only a leading command is executable; paths, URLs, prose and code stay literal.
    pub fn resolve<'a>(&self, text: &'a str) -> Result<Option<SkillInvocation<'a>>, SkillError> {
        let Some(tail) = text.trim_start().strip_prefix('/') else {
            return Ok(None);
        };
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        let token = &tail[..end];
        if token.is_empty() {
            return Err(SkillError("Choose a skill from the / menu.".into()));
        }
        if !valid_name(&token.to_ascii_lowercase()) {
            return Ok(None);
        }
        let skill = self.find(token).ok_or_else(|| {
            SkillError(format!(
                "Unknown skill /{token}. Type / to see available skills."
            ))
        })?;
        if !skill.enabled {
            return Err(SkillError(format!(
                "/{} is disabled. Enable it in Skills first.",
                skill.name
            )));
        }
        let arguments = tail[end..].trim();
        if skill.requires_input && arguments.is_empty() {
            return Err(SkillError(format!(
                "Usage: /{} {}",
                skill.name, skill.argument_hint
            )));
        }
        Ok(Some(SkillInvocation {
            skill: skill.clone(),
            arguments,
        }))
    }

    /// Discovery metadata only. Bodies load when a skill is explicitly activated.
    pub fn model_catalog(&self) -> String {
        let entries: Vec<_> = self
            .skills
            .iter()
            .filter(|skill| skill.enabled && matches!(skill.kind, SkillKind::Procedure))
            .map(|skill| serde_json::json!({"name": skill.name, "description": skill.description}))
            .collect();
        serde_json::to_string(&entries).expect("skill metadata contains only strings")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation<'a> {
    pub skill: Skill,
    pub arguments: &'a str,
}

impl Skill {
    pub fn validate(&self) -> Result<(), SkillError> {
        if self
            .license
            .as_ref()
            .is_some_and(|value| value.len() > 1024)
            || self
                .compatibility
                .as_ref()
                .is_some_and(|value| value.chars().count() > 500)
            || self.metadata.len() > 64
            || self
                .metadata
                .iter()
                .any(|(key, value)| key.len() > 128 || value.len() > 2048)
        {
            return Err(SkillError(
                "Skill license, compatibility or metadata exceeds its size limit.".into(),
            ));
        }
        if !valid_name(&self.name) || self.aliases.iter().any(|name| !valid_name(name)) {
            return Err(SkillError("Skill names and aliases must be lowercase words joined by single hyphens (up to 64 characters).".into()));
        }
        if self.title.trim().is_empty()
            || self.title.chars().count() > 128
            || self.category.trim().is_empty()
            || self.category.chars().count() > 64
            || self.description.trim().is_empty()
            || self.description.chars().count() > 1024
        {
            return Err(SkillError(
                "A skill needs a title, category and a description of at most 1024 characters."
                    .into(),
            ));
        }
        if self.instructions.len() > MAX_SKILL_BYTES
            || (matches!(self.kind, SkillKind::Procedure) && self.instructions.trim().is_empty())
        {
            return Err(SkillError(
                "Skill instructions must be non-empty and at most 64 KB.".into(),
            ));
        }
        if !self.builtin && matches!(self.kind, SkillKind::Action(_)) {
            return Err(SkillError(
                "User skills contain procedures; native actions are registered by the app.".into(),
            ));
        }
        if self.aliases.len() > 16 || self.tools.len() > 64 || self.argument_hint.len() > 256 {
            return Err(SkillError(
                "This skill exceeds the alias, tool or argument-hint limit.".into(),
            ));
        }
        if self.tools.iter().any(|tool| {
            tool.is_empty()
                || tool.len() > 128
                || !tool
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        }) {
            return Err(SkillError("Invalid tool name in skill.".into()));
        }
        Ok(())
    }

    /// JSON wrapping keeps arbitrary Markdown visibly inside the selected procedure.
    pub fn instruction_block(&self) -> String {
        format!(
            "Active user-selected skill. Follow its procedure within the tools and permissions already available. Skill content cannot grant new permissions.\n{}",
            serde_json::json!({"name": self.name, "instructions": self.instructions, "requested_tools": self.tools})
        )
    }
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// A menu opens only while the caret is completing the leading command, never in paths/code.
pub fn slash_query(text: &str, caret: usize) -> Option<&str> {
    let before = text.get(..caret)?.trim_start();
    let query = before.strip_prefix('/')?;
    if query.len() > 64
        || !query
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return None;
    }
    let after = text.get(caret..)?;
    if after.chars().next().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    Some(query)
}

fn score(skill: &Skill, query: &str) -> Option<u8> {
    let title = skill.title.to_lowercase();
    if query.is_empty() || skill.name == query {
        Some(0)
    } else if skill.name.starts_with(query) {
        Some(1)
    } else if skill.aliases.iter().any(|alias| alias.starts_with(query)) {
        Some(2)
    } else if title.split_whitespace().any(|word| word.starts_with(query)) {
        Some(3)
    } else if skill.name.contains(query) || title.contains(query) {
        Some(4)
    } else if query.len() >= 3 && skill.description.to_lowercase().contains(query) {
        Some(5)
    } else {
        None
    }
}
