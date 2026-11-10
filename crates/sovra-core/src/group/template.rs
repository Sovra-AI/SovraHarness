//! Helpers and schedules people share with their group, as templates.
//!
//! A shared helper is its words only: name, look, job and instructions. Sovra's
//! helpers carry no tool grants, so a shared helper can never bring tools with it.
//! A shared schedule is its name, timing and request, never a message-sending
//! target. A shared skill is its title, description and instructions; any tools it
//! asks for stay behind, so each person grants tools on their own computer. Neither runs anywhere until someone reads it and chooses **Use this
//! helper** or **Add to my schedules**, which makes their own copy under their own
//! control: a shared schedule never runs on another person's computer
//! (roadmap T3, "author-only execution").
//!
//! Templates are signed operations folded like shared artifacts: sharing an update
//! replaces earlier versions, two made at once are both kept, and withdraw / erase
//! work the same way.

use serde::{Deserialize, Serialize};

use super::keys::{DeviceId, OpHash, PersonId};
use super::knowledge::DetailStatus;
use super::space::Space;
use crate::Timestamp;

/// Longest helper instructions, in characters.
pub const MAX_TEMPLATE_INSTRUCTIONS_CHARS: usize = 8_000;
/// Longest name, role, tagline or timing, in characters.
pub const MAX_TEMPLATE_LINE_CHARS: usize = 200;
/// Most expertise topics on a shared helper.
pub const MAX_TEMPLATE_TOPICS: usize = 12;

/// What is shared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "template", rename_all = "snake_case")]
pub enum Template {
    Helper {
        name: String,
        icon: String,
        role: String,
        tagline: String,
        instructions: String,
        #[serde(default)]
        expertise: Vec<String>,
    },
    Schedule {
        name: String,
        /// Plain-words timing ("every weekday at 8am"), read again when added.
        cadence: String,
        prompt: String,
    },
    Skill {
        /// The title people see ("Plan a weekly menu").
        name: String,
        description: String,
        instructions: String,
    },
}

impl Template {
    pub fn name(&self) -> &str {
        match self {
            Self::Helper { name, .. } | Self::Schedule { name, .. } | Self::Skill { name, .. } => {
                name
            }
        }
    }

    /// The words someone should read before using it.
    pub fn body(&self) -> &str {
        match self {
            Self::Helper { instructions, .. } | Self::Skill { instructions, .. } => instructions,
            Self::Schedule { prompt, .. } => prompt,
        }
    }

    /// Clears every word, for "erase the words".
    fn erase(&mut self) {
        match self {
            Self::Helper {
                name,
                icon,
                role,
                tagline,
                instructions,
                expertise,
            } => {
                for text in [name, icon, role, tagline, instructions] {
                    text.clear();
                }
                expertise.clear();
            }
            Self::Schedule {
                name,
                cadence,
                prompt,
            } => {
                for text in [name, cadence, prompt] {
                    text.clear();
                }
            }
            Self::Skill {
                name,
                description,
                instructions,
            } => {
                for text in [name, description, instructions] {
                    text.clear();
                }
            }
        }
    }
}

/// Why a template could not be shared (checked by the sharer and every receiver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("it needs a name")]
    NoName,
    #[error("its words are empty")]
    Empty,
    #[error("it is too long to share")]
    TooLong,
    #[error("it has too many topics")]
    TooManyTopics,
}

/// The rules a shared helper or schedule must meet.
pub fn check_template(template: &Template) -> Result<(), TemplateError> {
    let too_long = |text: &str, limit: usize| text.chars().count() > limit;
    if template.name().trim().is_empty() {
        return Err(TemplateError::NoName);
    }
    if template.body().trim().is_empty() {
        return Err(TemplateError::Empty);
    }
    match template {
        Template::Helper {
            name,
            icon,
            role,
            tagline,
            instructions,
            expertise,
        } => {
            if [name, icon, role, tagline]
                .iter()
                .any(|line| too_long(line, MAX_TEMPLATE_LINE_CHARS))
                || too_long(instructions, MAX_TEMPLATE_INSTRUCTIONS_CHARS)
                || expertise
                    .iter()
                    .any(|topic| too_long(topic, MAX_TEMPLATE_LINE_CHARS))
            {
                return Err(TemplateError::TooLong);
            }
            if expertise.len() > MAX_TEMPLATE_TOPICS {
                return Err(TemplateError::TooManyTopics);
            }
        }
        Template::Schedule {
            name,
            cadence,
            prompt,
        } => {
            if too_long(name, MAX_TEMPLATE_LINE_CHARS)
                || too_long(cadence, MAX_TEMPLATE_LINE_CHARS)
                || too_long(prompt, MAX_TEMPLATE_INSTRUCTIONS_CHARS)
            {
                return Err(TemplateError::TooLong);
            }
        }
        Template::Skill {
            name,
            description,
            instructions,
        } => {
            if too_long(name, MAX_TEMPLATE_LINE_CHARS)
                || too_long(description, MAX_TEMPLATE_INSTRUCTIONS_CHARS)
                || too_long(instructions, MAX_TEMPLATE_INSTRUCTIONS_CHARS)
            {
                return Err(TemplateError::TooLong);
            }
        }
    }
    Ok(())
}

/// One shared helper or schedule as people see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedTemplate {
    pub id: OpHash,
    pub template: Template,
    pub author: DeviceId,
    pub person: Option<PersonId>,
    pub shared_by: String,
    pub computer: String,
    pub shared_at: Timestamp,
    pub replaces: Vec<OpHash>,
    pub status: DetailStatus,
    pub from_former_member: bool,
    #[serde(default)]
    pub space: Space,
}

impl SharedTemplate {
    /// Whether it is one of the versions people should see.
    pub fn is_latest(&self) -> bool {
        !self.from_former_member
            && matches!(
                self.status,
                DetailStatus::Current | DetailStatus::Disputed { .. }
            )
    }

    /// The team handle a copy of this helper gets: stable, so using it twice
    /// updates the same helper instead of adding another.
    pub fn helper_id(&self) -> String {
        format!("shared-{}", &self.id.to_hex()[..10])
    }

    pub(super) fn erase(&mut self) {
        self.template.erase();
    }
}
