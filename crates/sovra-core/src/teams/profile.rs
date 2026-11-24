//! How a helper sounds and decides, stored as data and turned into their instruction.
//!
//! The shape follows `SovraSRXProduction` `services/personas/profiles.py`: voice,
//! how they decide, the lens they use, what they know, and where they step back.
//! Big Five numbers and type labels stay out of this app. The page and the
//! instruction use plain words.

use serde::{Deserialize, Serialize};

use super::TeamMember;

/// How a helper sounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HelperVoice {
    /// One sentence a person can read on the card.
    #[serde(default)]
    pub tone: String,
    /// The rhythm of their replies.
    #[serde(default)]
    pub cadence: String,
    /// Example ways they start, not a script.
    #[serde(default)]
    pub openings: Vec<String>,
    /// Phrases they reach for.
    #[serde(default)]
    pub phrases: Vec<String>,
    /// Words that are not their voice.
    #[serde(default)]
    pub avoids: Vec<String>,
}

/// Whether they argue, play along, or only keep the discussion moving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperPersonality {
    #[serde(default)]
    pub voice: HelperVoice,
    /// How they choose, in plain words.
    #[serde(default)]
    pub decides: String,
    /// The lens they look through.
    #[serde(default)]
    pub lens: String,
    /// What they know well.
    #[serde(default)]
    pub knows: Vec<String>,
    /// Where they should step back.
    #[serde(default)]
    pub defers: Vec<String>,
    /// They say so when they disagree.
    #[serde(default)]
    pub argues: bool,
    /// They join in when the chat is playful.
    #[serde(default = "default_true")]
    pub plays_along: bool,
    /// They keep the discussion moving and do not solve the task.
    #[serde(default)]
    pub moderates_only: bool,
}

fn default_true() -> bool {
    true
}

impl TeamMember {
    /// The one line on the card: how they sound, or their job line.
    pub fn personality_line(&self) -> &str {
        self.personality
            .as_ref()
            .map(|profile| profile.voice.tone.as_str())
            .filter(|line| !line.is_empty())
            .unwrap_or(self.tagline.as_str())
    }

    /// What they focus on, in plain words.
    pub fn focus_line(&self) -> String {
        self.personality
            .as_ref()
            .map(|profile| profile.lens.clone())
            .filter(|line| !line.is_empty())
            .unwrap_or_else(|| self.tagline.clone())
    }

    /// Where they step back, in plain words.
    pub fn defer_line(&self) -> String {
        let stepped = self
            .personality
            .as_ref()
            .map(|profile| profile.defers.join(", "))
            .unwrap_or_default();
        if stepped.is_empty() {
            "They stay with their own job.".to_owned()
        } else {
            stepped
        }
    }
}

/// The middle of a helper's instruction: their voice when they have one, otherwise
/// the free-text note.
pub(super) fn working_sections(member: &TeamMember) -> String {
    let Some(profile) = &member.personality else {
        return format!(
            "# ROLE\n{tagline}\n\n# HOW YOU WORK\n{instructions}",
            tagline = member.tagline,
            instructions = member.instructions,
        );
    };
    let mut out = format!("# ROLE\n{}\n\n{}", member.tagline, profile.voice_block());
    if !profile.decides.is_empty() {
        out.push_str("\n\n# HOW YOU DECIDE\n");
        out.push_str(&profile.decides);
    }
    if !profile.lens.is_empty() {
        out.push_str("\n\n# YOUR LENS\n");
        out.push_str(&profile.lens);
    }
    if !profile.knows.is_empty() {
        out.push_str("\n\n# WHAT YOU KNOW\n");
        out.push_str(&profile.knows.join(", "));
    }
    out.push_str("\n\n# WHERE YOU STEP BACK\n");
    out.push_str(&member.defer_line());
    out.push_str("\n\n# HOW YOU JOIN IN\n");
    out.push_str(&profile.joins_in());
    if !member.instructions.trim().is_empty() {
        out.push_str("\n\n# HOW YOU WORK\n");
        out.push_str(member.instructions.trim());
    }
    out
}

impl HelperPersonality {
    fn voice_block(&self) -> String {
        let voice = &self.voice;
        let mut lines = Vec::new();
        if !voice.tone.is_empty() {
            lines.push(format!("Tone: {}", voice.tone));
        }
        if !voice.cadence.is_empty() {
            lines.push(format!("Cadence: {}", voice.cadence));
        }
        if !voice.openings.is_empty() {
            lines.push(format!("Example openings: {}.", quoted(&voice.openings)));
        }
        if !voice.phrases.is_empty() {
            lines.push(format!(
                "Phrases you reach for: {}.",
                quoted(&voice.phrases)
            ));
        }
        if !voice.avoids.is_empty() {
            lines.push(format!("Words you avoid: {}.", quoted(&voice.avoids)));
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!("# HOW YOU SOUND\n{}", lines.join("\n"))
        }
    }

    fn joins_in(&self) -> String {
        let mut lines = Vec::new();
        if self.moderates_only {
            lines.push("You only moderate. You do not solve the task yourself.".to_owned());
        } else if self.argues {
            lines.push("If you disagree, say so and give the reason.".to_owned());
        } else {
            lines.push("Add your part. You do not pick a fight for its own sake.".to_owned());
        }
        if self.plays_along {
            lines.push("If the chat is playful, play along.".to_owned());
        }
        lines.join("\n")
    }
}

fn quoted(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}
