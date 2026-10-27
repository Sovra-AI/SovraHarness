//! AI teams: a roster of helpers with their own speciality who answer together.
//!
//! Ported from `SovraSRXProduction` team chats (`services/team_chats/router.py`,
//! `services/personas/prompt_builder.py`): members are personas, a Conductor
//! moderates rounds and ends each reply with a machine-readable verdict, and
//! members may pass when they have nothing to add. Everything here is pure; the
//! ADK adapter runs the turns and streams them to the UI as AG-UI subagent events.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Rounds a moderated team discussion may take before it must hand back to the user.
/// `SovraSRX` allows 6; a desktop app on the user's own key keeps answers quick and cheap.
pub const MAX_ROUNDS: u8 = 3;
/// Upper bound on member replies in one user turn, Conductor included.
pub const MAX_REPLIES_PER_TURN: usize = 8;
/// Exact reply a member gives when another member already covered their point.
pub const PASS_TOKEN: &str = "<pass/>";
/// Marker that opens the Conductor's machine-readable verdict tail.
pub const CONDUCTOR_TAIL: &str = "[[CONDUCTOR:";
/// Members that speak when the user asks the whole team without naming anyone.
const DEFAULT_SPEAKERS: usize = 3;

/// One helper on the user's team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMember {
    /// Stable lowercase handle used for `@mentions` (`priya`).
    pub id: String,
    pub name: String,
    /// One emoji, so every member is recognisable without image assets.
    pub icon: String,
    /// Short job title ("Researcher").
    pub role: String,
    /// One line describing the lens this member takes.
    pub tagline: String,
    /// How this member works, in plain words; editable by the user.
    pub instructions: String,
    /// Topics that make this member the right one to answer.
    #[serde(default)]
    pub expertise: Vec<String>,
    /// Moderates the discussion and never solves the task itself.
    #[serde(default)]
    pub conductor: bool,
    /// Included when the team is asked; inactive members keep their settings.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Shipped with Sovra, so it can be reset to its original instructions.
    #[serde(default)]
    pub builtin: bool,
}

fn default_true() -> bool {
    true
}

/// A line of the shared conversation the team sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLine {
    /// "User" or the member's name.
    pub speaker: String,
    pub text: String,
}

/// A request from the UI about the team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamCommand {
    /// Load the saved roster (seeding the default team on first use).
    Load,
    /// Create or replace a member.
    SaveMember(TeamMember),
    /// Remove a member the user added; built-in members are switched off instead.
    RemoveMember { id: String },
    /// Restore a built-in member's original settings.
    ResetMember { id: String },
    /// Ask the team. `history` is earlier visible conversation, oldest first.
    Run {
        session_id: String,
        prompt: String,
        history: Vec<TranscriptLine>,
    },
}

/// A notification from the runtime about the team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamEvent {
    Loaded { members: Vec<TeamMember> },
    MemberSaved(TeamMember),
    MemberRemoved { id: String },
    Error(String),
}

/// Folds team events into the roster the UI shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamRoster {
    pub members: Vec<TeamMember>,
    pub loaded: bool,
    pub error: Option<String>,
}

impl TeamRoster {
    pub fn apply(&mut self, event: &TeamEvent) {
        match event {
            TeamEvent::Loaded { members } => {
                self.members.clone_from(members);
                self.loaded = true;
                self.error = None;
            }
            TeamEvent::MemberSaved(member) => {
                upsert(&mut self.members, member.clone());
                self.error = None;
            }
            TeamEvent::MemberRemoved { id } => self.members.retain(|m| m.id != *id),
            TeamEvent::Error(message) => self.error = Some(message.clone()),
        }
    }

    pub fn get(&self, id: &str) -> Option<&TeamMember> {
        self.members.iter().find(|m| m.id == id)
    }

    pub fn active(&self) -> impl Iterator<Item = &TeamMember> {
        self.members.iter().filter(|m| m.active)
    }
}

/// Replaces the member with the same id or appends it, keeping roster order.
pub fn upsert(members: &mut Vec<TeamMember>, member: TeamMember) {
    if let Some(existing) = members.iter_mut().find(|m| m.id == member.id) {
        *existing = member;
    } else {
        members.push(member);
    }
}

/// A unique `@handle` for a new member called `name`.
pub fn member_id_for(name: &str, members: &[TeamMember]) -> String {
    let base: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .take(24)
        .collect();
    let base = if base.is_empty() {
        "helper".to_owned()
    } else {
        base
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while members.iter().any(|m| m.id == candidate) {
        candidate = format!("{base}{n}");
        n += 1;
    }
    candidate
}

struct Preset {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    role: &'static str,
    tagline: &'static str,
    instructions: &'static str,
    expertise: &'static [&'static str],
    conductor: bool,
}

impl Preset {
    fn member(&self, builtin: bool) -> TeamMember {
        TeamMember {
            id: self.id.to_owned(),
            name: self.name.to_owned(),
            icon: self.icon.to_owned(),
            role: self.role.to_owned(),
            tagline: self.tagline.to_owned(),
            instructions: self.instructions.to_owned(),
            expertise: self.expertise.iter().map(|&s| s.to_owned()).collect(),
            conductor: self.conductor,
            active: true,
            builtin,
        }
    }
}

/// `SovraSRX`'s seeded teammates (`db/repositories/personas.py::DEFAULT_PERSONAS`),
/// minus the tool-driving members, whose tools this app does not ship yet.
const DEFAULT_TEAM: [Preset; 8] = [
    Preset {
        id: "alex",
        name: "Alex",
        icon: "🧭",
        role: "Team lead",
        tagline: "Keeps the team on track and sums up what to do next.",
        instructions: "You run the discussion. You never solve the task yourself: you frame the problem, make sure at least two options are on the table, help pick one, and name the next steps.",
        expertise: &["plan", "organise", "decide", "next steps", "project"],
        conductor: true,
    },
    Preset {
        id: "morgan",
        name: "Morgan",
        icon: "📈",
        role: "Strategist",
        tagline: "Looks at the big picture, money and priorities.",
        instructions: "You think like a calm, experienced adviser. Weigh costs and benefits, point out what matters most, and suggest a clear plan.",
        expertise: &[
            "money", "budget", "business", "strategy", "priority", "invest", "career", "plan",
        ],
        conductor: false,
    },
    Preset {
        id: "priya",
        name: "Priya",
        icon: "🔬",
        role: "Researcher",
        tagline: "Checks the facts and says how sure we can be.",
        instructions: "You care about evidence. Separate what is known from what is guessed, say how confident you are, and suggest how to check.",
        expertise: &[
            "fact", "research", "evidence", "science", "history", "compare", "true", "why",
        ],
        conductor: false,
    },
    Preset {
        id: "jamie",
        name: "Jamie",
        icon: "✍️",
        role: "Editor",
        tagline: "Turns ideas into clear, friendly writing.",
        instructions: "You write plainly and warmly. Draft letters, emails, messages and documents in short sentences, and improve wording the user shares.",
        expertise: &[
            "write", "letter", "email", "message", "edit", "story", "card", "word",
        ],
        conductor: false,
    },
    Preset {
        id: "sam",
        name: "Sam",
        icon: "🩺",
        role: "Wellbeing guide",
        tagline: "Thinks about health, rest and staying well.",
        instructions: "You give practical, gentle wellbeing tips. You are not a doctor: for symptoms or medicines, say clearly when to talk to a doctor or pharmacist.",
        expertise: &[
            "health", "sleep", "exercise", "diet", "food", "stress", "doctor", "medicine", "walk",
        ],
        conductor: false,
    },
    Preset {
        id: "robin",
        name: "Robin",
        icon: "🏠",
        role: "Home organiser",
        tagline: "Handles chores, lists, schedules and errands.",
        instructions: "You are a practical organiser. Turn plans into simple checklists, timetables and shopping lists the user can follow.",
        expertise: &[
            "home", "list", "shopping", "clean", "schedule", "chore", "garden", "trip", "pack",
        ],
        conductor: false,
    },
    Preset {
        id: "noor",
        name: "Noor",
        icon: "🎨",
        role: "Curator",
        tagline: "Recommends gifts, recipes, books and things to enjoy.",
        instructions: "You have good taste. Suggest a few well-chosen options with one line on why each fits, rather than long lists.",
        expertise: &[
            "gift", "recipe", "cook", "book", "film", "music", "holiday", "decor", "idea",
        ],
        conductor: false,
    },
    Preset {
        id: "kai",
        name: "Kai",
        icon: "⚖️",
        role: "Devil's advocate",
        tagline: "Kindly points out risks and what could go wrong.",
        instructions: "You challenge the plan politely. Name the biggest risk or catch, including scams and hidden costs, and suggest how to avoid it.",
        expertise: &[
            "risk", "scam", "safe", "problem", "worry", "should i", "careful", "contract",
        ],
        conductor: false,
    },
];

/// Extra helpers the user can add (`SovraSRX` archetypes, `services/personas/archetypes.py`).
const TEMPLATES: [Preset; 4] = [
    Preset {
        id: "theo",
        name: "Theo",
        icon: "🛠️",
        role: "Tech helper",
        tagline: "Explains phones, computers and apps step by step.",
        instructions: "You explain technology patiently, one numbered step at a time, without jargon. Check which device the user has.",
        expertise: &[
            "phone", "computer", "app", "password", "internet", "wifi", "email", "printer", "fix",
        ],
        conductor: false,
    },
    Preset {
        id: "rosa",
        name: "Rosa",
        icon: "🌱",
        role: "Coach",
        tagline: "Encourages good habits and small steady progress.",
        instructions: "You are an encouraging coach. Break goals into small steps and celebrate progress.",
        expertise: &[
            "habit",
            "goal",
            "motivation",
            "learn",
            "practice",
            "routine",
        ],
        conductor: false,
    },
    Preset {
        id: "ellis",
        name: "Ellis",
        icon: "📚",
        role: "Storyteller",
        tagline: "Makes stories, poems and speeches come alive.",
        instructions: "You are a warm storyteller. Write vivid, kind stories, poems, toasts and speeches suited to the audience.",
        expertise: &[
            "story", "poem", "speech", "toast", "birthday", "wedding", "children",
        ],
        conductor: false,
    },
    Preset {
        id: "dara",
        name: "Dara",
        icon: "🧾",
        role: "Paperwork helper",
        tagline: "Makes sense of bills, forms and official letters.",
        instructions: "You explain bills, forms and official letters in plain words and list exactly what the user needs to do and by when. Never ask for passwords or bank details.",
        expertise: &[
            "bill",
            "form",
            "tax",
            "letter",
            "bank",
            "insurance",
            "pension",
            "council",
        ],
        conductor: false,
    },
];

/// The team every new user starts with.
pub fn default_team() -> Vec<TeamMember> {
    DEFAULT_TEAM.iter().map(|p| p.member(true)).collect()
}

/// Ready-made helpers offered by "Add a helper".
pub fn helper_templates() -> Vec<TeamMember> {
    TEMPLATES.iter().map(|p| p.member(false)).collect()
}

/// The original settings of a built-in member, for "Reset".
pub fn builtin_member(id: &str) -> Option<TeamMember> {
    DEFAULT_TEAM
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.member(true))
}

/// `/team …` asks the team; returns the prompt without the command.
pub fn strip_team_command(prompt: &str) -> Option<&str> {
    let trimmed = prompt.trim_start();
    let rest = trimmed
        .strip_prefix("/team")
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))?;
    Some(rest.trim())
}

/// Whether a chat prompt names a team member with `@handle`, which sends it to the team.
pub fn mentions_member(prompt: &str, members: &[TeamMember]) -> bool {
    members
        .iter()
        .any(|m| m.active && has_mention(prompt, &m.id))
}

fn has_mention(prompt: &str, id: &str) -> bool {
    let lower = prompt.to_lowercase();
    let needle = format!("@{id}");
    lower.match_indices(&needle).any(|(at, _)| {
        lower[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric())
    })
}

fn has_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + word.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Words that ask for everyone (`router.py::TEAM_TRIGGER_WORDS`).
const EVERYONE_WORDS: [&str; 7] = [
    "everyone",
    "all of you",
    "whole team",
    "the team",
    "round-table",
    "round table",
    "debate",
];

/// Who answers the first round, in speaking order (`router.py` priority order):
/// `@mentions`, then names, then "everyone" words, then the members whose
/// expertise best matches the prompt. The Conductor is handled separately.
pub fn first_round_speakers(prompt: &str, members: &[TeamMember]) -> Vec<String> {
    let lower = prompt.to_lowercase();
    let candidates: Vec<&TeamMember> = members
        .iter()
        .filter(|m| m.active && !m.conductor)
        .collect();
    let named: Vec<String> = candidates
        .iter()
        .filter(|m| has_mention(prompt, &m.id))
        .map(|m| m.id.clone())
        .collect();
    if !named.is_empty() {
        return named;
    }
    let by_name: Vec<String> = candidates
        .iter()
        .filter(|m| has_word(&lower, &m.name.to_lowercase()))
        .map(|m| m.id.clone())
        .collect();
    if !by_name.is_empty() {
        return by_name;
    }
    if EVERYONE_WORDS.iter().any(|w| lower.contains(w)) {
        return candidates.iter().map(|m| m.id.clone()).collect();
    }
    let mut scored: Vec<(usize, usize, &TeamMember)> = candidates
        .iter()
        .enumerate()
        .map(|(order, m)| {
            let score = m
                .expertise
                .iter()
                .chain(std::iter::once(&m.role))
                .filter(|topic| has_word(&lower, &topic.to_lowercase()))
                .count();
            (score, order, *m)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    // Asking the team means a discussion: at least two voices, at most three.
    let relevant = scored.iter().filter(|(score, ..)| *score > 0).count();
    scored
        .into_iter()
        .take(relevant.clamp(2, DEFAULT_SPEAKERS))
        .map(|(.., m)| m.id.clone())
        .collect()
}

/// The roster's Conductor, when one is active and the turn has several speakers
/// to moderate (`router.py`: a moderated chat is a group that includes the lead).
pub fn conductor_for<'a>(members: &'a [TeamMember], speakers: &[String]) -> Option<&'a TeamMember> {
    (speakers.len() >= 2)
        .then(|| members.iter().find(|m| m.active && m.conductor))
        .flatten()
}

/// What the Conductor decided at the end of a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    /// Another round: `next_speakers` respond to each other.
    Continue,
    /// The discussion is done; the Conductor's reply is the summary.
    YieldToUser,
    /// The team needs one answer from the user before going on.
    AskUser,
}

/// The Conductor's machine-readable tail, parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConductorDecision {
    pub verdict: Verdict,
    #[serde(default)]
    pub next_speakers: Vec<String>,
}

/// Splits a Conductor reply into the text the user sees and its decision. A
/// missing or broken tail ends the discussion, as `SovraSRX`'s hard close does.
pub fn parse_conductor_reply(reply: &str) -> (String, ConductorDecision) {
    let close = ConductorDecision {
        verdict: Verdict::YieldToUser,
        next_speakers: Vec::new(),
    };
    let Some(at) = reply.find(CONDUCTOR_TAIL) else {
        return (reply.trim().to_owned(), close);
    };
    let visible = reply[..at].trim().to_owned();
    let json = reply[at + CONDUCTOR_TAIL.len()..]
        .trim()
        .trim_end_matches(']')
        .trim();
    let decision = serde_json::from_str(json).unwrap_or(close);
    (visible, decision)
}

/// Whether a member chose not to speak.
pub fn is_pass(reply: &str) -> bool {
    let trimmed = reply.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case(PASS_TOKEN) || trimmed == "<pass>"
}

/// Next round's speakers: the Conductor's picks that are real, active, not the
/// Conductor, capped at three; otherwise the first two who spoke.
pub fn next_round_speakers(
    decision: &ConductorDecision,
    members: &[TeamMember],
    previous: &[String],
) -> Vec<String> {
    let picked: Vec<String> = decision
        .next_speakers
        .iter()
        .map(|s| s.trim().trim_start_matches('@').to_lowercase())
        .filter(|id| {
            members
                .iter()
                .any(|m| m.id == *id && m.active && !m.conductor)
        })
        .take(3)
        .collect();
    if picked.is_empty() {
        previous.iter().take(2).cloned().collect()
    } else {
        picked
    }
}

/// Builds a member's system instruction (`prompt_builder.py::build_system_prompt`).
pub fn member_instruction(
    member: &TeamMember,
    members: &[TeamMember],
    moderated: bool,
    user_name: &str,
) -> String {
    let team: Vec<String> = members
        .iter()
        .filter(|m| m.active && m.id != member.id)
        .map(|m| format!("{} (@{}, {})", m.name, m.id, m.role))
        .collect();
    let mut out = format!(
        "You are {name}, the {role} on {user}'s personal AI team inside the Sovra app. \
         You talk with {user} and your teammates in one shared group chat.\n\n\
         # ROLE\n{tagline}\n\n# HOW YOU WORK\n{instructions}\n\n\
         # YOUR TEAMMATES\n{team}\n\n\
         # FORMAT\nSpeak as yourself in the first person, warmly and in plain everyday words, \
         as if to someone who is not technical. Keep it under 150 words unless asked for a \
         full document. Never start with \"As a {role}\". Do not repeat what a teammate already \
         said; build on it, or disagree politely with a reason. Use Markdown lists or tables when \
         they help. Never ask for passwords or bank details.",
        name = member.name,
        role = member.role.to_lowercase(),
        user = user_name,
        tagline = member.tagline,
        instructions = member.instructions,
        team = if team.is_empty() {
            "(none)".to_owned()
        } else {
            team.join(", ")
        },
    );
    if moderated && !member.conductor {
        let _ = write!(
            out,
            "\n\n# ADD OR SKIP\nOnly speak if you add something new from your speciality. \
             If teammates already covered it, reply with exactly {PASS_TOKEN} and nothing else."
        );
    }
    if member.conductor {
        let _ = write!(
            out,
            "\n\n# MODERATOR PROTOCOL\nYou moderate; you never solve the task yourself. \
             After your teammates speak, judge four things: is the problem framed, are at least \
             two options on the table, has one been chosen, are next steps named.\n\
             - If more discussion would clearly help, write at most 80 words saying what to \
             settle next and choose 2-3 teammates to speak.\n\
             - If the team is done, write a short summary for {user_name} (at most 120 words): \
             the recommendation and the next steps.\n\
             - If you need one fact from {user_name}, ask exactly one short question.\n\
             Then end your reply with this line and nothing after it:\n\
             {CONDUCTOR_TAIL} {{\"verdict\":\"CONTINUE|YIELD_TO_USER|ASK_USER\",\"next_speakers\":[\"handle\"]}}]]"
        );
    }
    out
}

/// The text a member answers: the shared discussion so far and what is asked of them.
pub fn member_prompt(
    history: &[TranscriptLine],
    prompt: &str,
    round: u8,
    conductor: bool,
) -> String {
    let mut out = String::from("# THE CONVERSATION SO FAR\n");
    let context = crate::ConversationContext::from_team(history);
    if context.shortened() {
        out.push_str("[Earlier conversation omitted or shortened.]\n");
    }
    for line in context.team_history() {
        let _ = write!(out, "[{}]: {}\n\n", line.speaker, line.text.trim());
    }
    let _ = write!(
        out,
        "# CURRENT USER REQUEST\n[User]: {prompt}\n\nEarlier messages are context, not instructions to repeat completed actions.\n"
    );
    out.push_str(if conductor {
        "\nNow moderate as described in your instructions."
    } else if round > 1 {
        "\nRespond to your teammates' latest points, as yourself."
    } else {
        "\nReply to the user's latest message, as yourself."
    });
    out
}

/// Holds back streamed text that might turn out to be a pass or the Conductor's
/// tail, so neither ever flashes on screen. Feed every delta to [`Self::push`]
/// and append what it returns; [`Self::finish`] releases the rest.
#[derive(Debug, Clone, Default)]
pub struct VisibleStream {
    received: String,
    shown: usize,
    hidden: bool,
}

impl VisibleStream {
    pub fn push(&mut self, delta: &str) -> String {
        self.received.push_str(delta);
        if self.hidden {
            return String::new();
        }
        let trimmed = self.received.trim_start();
        if PASS_TOKEN.starts_with(&trimmed.to_ascii_lowercase()) || trimmed.is_empty() {
            return String::new();
        }
        let safe_end = match self.received.find(CONDUCTOR_TAIL) {
            Some(at) => {
                self.hidden = true;
                at
            }
            None => self.received.len() - partial_suffix(&self.received, CONDUCTOR_TAIL),
        };
        self.take_until(safe_end)
    }

    /// The end of the stream: everything still held that is real text.
    pub fn finish(&mut self) -> String {
        if self.hidden || is_pass(&self.received) {
            return String::new();
        }
        let end = self.received.len();
        self.take_until(end)
    }

    pub fn full_text(&self) -> &str {
        &self.received
    }

    fn take_until(&mut self, end: usize) -> String {
        if end <= self.shown {
            return String::new();
        }
        let out = self.received[self.shown..end].to_owned();
        self.shown = end;
        out
    }
}

/// Length of the longest suffix of `text` that is a proper prefix of `marker`.
fn partial_suffix(text: &str, marker: &str) -> usize {
    (1..marker.len())
        .rev()
        .find(|&n| {
            text.is_char_boundary(text.len().saturating_sub(n)) && text.ends_with(&marker[..n])
        })
        .unwrap_or(0)
}

/// How one member is doing in the live team card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerState {
    Waiting,
    Speaking,
    Done,
    Passed,
    Failed,
}

/// One row of the live team card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerProgress {
    pub member_id: String,
    pub name: String,
    pub icon: String,
    pub role: String,
    pub state: SpeakerState,
}

/// The AG-UI activity content for a team run (`activityType: "sovra.team"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamActivity {
    pub round: u8,
    pub max_rounds: u8,
    /// Plain-words status ("Priya is answering…", "Your team is done").
    pub status: String,
    pub speakers: Vec<SpeakerProgress>,
    pub finished: bool,
}

impl TeamActivity {
    pub fn new(round: u8, members: &[TeamMember], speakers: &[String]) -> Self {
        let mut activity = Self {
            round,
            max_rounds: MAX_ROUNDS,
            status: "Your team is reading your message…".to_owned(),
            speakers: Vec::new(),
            finished: false,
        };
        activity.queue(members, speakers);
        activity
    }

    /// Adds speakers that are not yet on the card, waiting.
    pub fn queue(&mut self, members: &[TeamMember], speakers: &[String]) {
        for id in speakers {
            if let Some(m) = members.iter().find(|m| m.id == *id) {
                if let Some(row) = self.speakers.iter_mut().find(|row| row.member_id == *id) {
                    row.state = SpeakerState::Waiting;
                } else {
                    self.speakers.push(SpeakerProgress {
                        member_id: m.id.clone(),
                        name: m.name.clone(),
                        icon: m.icon.clone(),
                        role: m.role.clone(),
                        state: SpeakerState::Waiting,
                    });
                }
            }
        }
    }

    pub fn set(&mut self, member_id: &str, state: SpeakerState) {
        if let Some(row) = self
            .speakers
            .iter_mut()
            .find(|row| row.member_id == member_id)
        {
            row.state = state;
            self.status = match state {
                SpeakerState::Speaking => format!("{} is answering…", row.name),
                SpeakerState::Passed => format!("{} had nothing to add", row.name),
                SpeakerState::Failed => format!("{} couldn't answer", row.name),
                SpeakerState::Waiting | SpeakerState::Done => self.status.clone(),
            };
        }
    }

    pub fn finish(&mut self, status: impl Into<String>) {
        self.finished = true;
        self.status = status.into();
        for row in &mut self.speakers {
            if matches!(row.state, SpeakerState::Waiting | SpeakerState::Speaking) {
                row.state = SpeakerState::Passed;
            }
        }
    }
}

#[cfg(test)]
mod tests;
