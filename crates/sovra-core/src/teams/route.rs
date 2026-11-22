//! Who speaks in a team chat, and when a moderated round should hand back.
//!
//! Precedence follows `SovraSRXProduction` `route_message` for an address, a
//! whole-team phrase, ask-everyone and a one-helper chat. A message that names
//! nobody is split into jobs the way the `SovraSRX` orchestrator splits a plan
//! (`decide_team_route`): each sentence asks the helper who matches it most
//! strongly, a tie asks each of those helpers, and the same helper can take
//! two sentences at once. A stray speciality word does not add a helper. The
//! team-chat classifier, which always keeps a single helper, is not used.
//! Scoring is role and expertise words, not a second model. The soft close follows the natural-ending block in
//! `team_chats.py`: from round 2, a continue hands back when confidence is
//! under 0.65, word-trigram overlap is at least 0.8, or less than 0.2 of the
//! new wording is new.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{CONDUCTOR_TAIL, MAX_REPLIES_PER_TURN, TeamMember, TranscriptLine, is_pass};

/// How sure the lead is that another round would help, in thousandths.
///
/// Stored as an integer so a decision stays `Eq`. 650 is 0.65 and does not
/// hand back; anything below it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(u16);

impl Confidence {
    pub const FULL: Self = Self(1000);

    pub fn full() -> Self {
        Self::FULL
    }

    pub fn from_unit(value: f64) -> Self {
        if !value.is_finite() {
            return Self::FULL;
        }
        let scaled = (value.clamp(0.0, 1.0) * 1000.0).round();
        // The clamp keeps this inside 0..=1000, which fits in u16.
        #[expect(clippy::cast_possible_truncation, reason = "clamped to 0..=1000")]
        #[expect(clippy::cast_sign_loss, reason = "clamped to 0..=1000")]
        let thousandths = scaled as u16;
        Self(thousandths)
    }

    pub fn below_soft_close(self) -> bool {
        self.0 < 650
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self::FULL
    }
}

/// Which helpers belong to this conversation.
///
/// An empty group is the whole switched-on team. Ask-everyone starts off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TeamRoom {
    #[default]
    Ordinary,
    Direct {
        helper_id: String,
    },
    Group {
        #[serde(default)]
        helper_ids: Vec<String>,
        #[serde(default)]
        ask_everyone: bool,
    },
}

impl TeamRoom {
    pub fn ordinary() -> Self {
        Self::Ordinary
    }

    pub fn direct(helper_id: impl Into<String>) -> Self {
        Self::Direct {
            helper_id: helper_id.into(),
        }
    }

    /// A chosen group. Ids are sorted so the same people are one chat.
    pub fn group(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut helper_ids: Vec<String> = ids
            .into_iter()
            .map(Into::into)
            .filter(|id| !id.is_empty())
            .collect();
        helper_ids.sort();
        helper_ids.dedup();
        Self::Group {
            helper_ids,
            ask_everyone: false,
        }
    }

    /// Every switched-on helper, with ask-everyone left off.
    pub fn all_helpers() -> Self {
        Self::Group {
            helper_ids: Vec::new(),
            ask_everyone: false,
        }
    }

    pub fn is_ordinary(&self) -> bool {
        matches!(self, Self::Ordinary)
    }

    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct { .. })
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }

    pub fn is_team_chat(&self) -> bool {
        !self.is_ordinary()
    }

    pub fn ask_everyone(&self) -> bool {
        matches!(
            self,
            Self::Group {
                ask_everyone: true,
                ..
            }
        )
    }

    #[must_use]
    pub fn with_ask_everyone(self, ask: bool) -> Self {
        match self {
            Self::Group { helper_ids, .. } => Self::Group {
                helper_ids,
                ask_everyone: ask,
            },
            other => other,
        }
    }
}

/// Restores a room saved with a chat. Missing or unreadable data is an ordinary chat.
pub fn room_from_json(raw: Option<&str>) -> TeamRoom {
    raw.and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

/// Serialises a team chat. An ordinary chat stores nothing.
pub fn room_to_json(room: &TeamRoom) -> Option<String> {
    if room.is_ordinary() {
        None
    } else {
        Some(serde_json::to_string(room).expect("a team room is plain data"))
    }
}

/// Title for an empty chat in this room. `None` means the ordinary "New chat".
pub fn room_title(room: &TeamRoom, members: &[TeamMember]) -> Option<String> {
    match room {
        TeamRoom::Ordinary => None,
        TeamRoom::Direct { helper_id } => {
            Some(format!("Chat with {}", member_name(members, helper_id)))
        }
        TeamRoom::Group { helper_ids, .. } if helper_ids.is_empty() => Some("My team".to_owned()),
        TeamRoom::Group { helper_ids, .. } => {
            let names = listed_names(members, helper_ids);
            Some(if names.is_empty() {
                "My team".to_owned()
            } else {
                english_list(&names)
            })
        }
    }
}

/// The sentence above the composer. `None` for an ordinary chat.
pub fn room_blurb(room: &TeamRoom, members: &[TeamMember]) -> Option<String> {
    match room {
        TeamRoom::Ordinary => None,
        TeamRoom::Direct { helper_id } => Some(format!(
            "{} will answer this chat.",
            member_name(members, helper_id)
        )),
        TeamRoom::Group {
            helper_ids,
            ask_everyone,
        } => {
            let names = listed_names(members, helper_ids);
            let who = if names.is_empty() {
                "Your helpers".to_owned()
            } else {
                english_list(&names)
            };
            Some(if *ask_everyone {
                format!("{who} will all answer.")
            } else {
                format!(
                    "{who} are in this chat. A question can ask one helper or several. The same helper can answer two parts at once."
                )
            })
        }
    }
}

fn member_name<'a>(members: &'a [TeamMember], id: &str) -> &'a str {
    members
        .iter()
        .find(|member| member.id == id)
        .map_or("helper", |member| member.name.as_str())
}

fn listed_names(members: &[TeamMember], ids: &[String]) -> Vec<String> {
    if ids.is_empty() {
        members
            .iter()
            .filter(|member| member.active && !member.conductor)
            .map(|member| member.name.clone())
            .collect()
    } else {
        members
            .iter()
            .filter(|member| ids.iter().any(|id| id == &member.id))
            .map(|member| member.name.clone())
            .collect()
    }
}

fn english_list(names: &[String]) -> String {
    match names {
        [] => "My team".to_owned(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Who answers this message, in speaking order.
///
/// A one-helper chat returns before mentions, so naming someone else or saying
/// "everyone" does not pull them in. The lead is added only for a real
/// multi-helper turn, or when the user addressed the lead with `@`.
pub fn route_speakers(
    prompt: &str,
    members: &[TeamMember],
    room: &TeamRoom,
    history: &[TranscriptLine],
) -> Vec<String> {
    if let TeamRoom::Direct { helper_id } = room {
        return members
            .iter()
            .find(|member| member.active && member.id == *helper_id)
            .map(|member| vec![member.id.clone()])
            .unwrap_or_default();
    }

    let pool = non_lead_pool(members, room);
    let mentionable = mentionable_members(members, &pool);

    if let Some(named) = addressed(prompt, members, &mentionable, &pool) {
        return cap_speakers(named, members);
    }

    if has_team_trigger(prompt) {
        return cap_speakers(with_lead(pool_ids(&pool), members), members);
    }

    if room.ask_everyone() {
        return cap_speakers(with_lead(pool_ids(&pool), members), members);
    }

    cap_speakers(jobs_for(&pool, prompt, history, members), members)
}

/// Switched-on helpers who belong to this chat.
///
/// A chosen group is those helpers plus the lead, who only moderates. A
/// one-helper chat is that helper. An open group or an ordinary chat is
/// everyone switched on.
pub fn chat_members<'a>(members: &'a [TeamMember], room: &TeamRoom) -> Vec<&'a TeamMember> {
    let chosen = match room {
        TeamRoom::Direct { helper_id } => Some(std::slice::from_ref(helper_id)),
        TeamRoom::Group { helper_ids, .. } if !helper_ids.is_empty() => Some(helper_ids.as_slice()),
        TeamRoom::Ordinary | TeamRoom::Group { .. } => None,
    };
    members
        .iter()
        .filter(|member| {
            member.active
                && chosen.is_none_or(|ids| {
                    ids.iter().any(|id| id == &member.id) || (member.conductor && room.is_group())
                })
        })
        .collect()
}

fn non_lead_pool<'a>(members: &'a [TeamMember], room: &TeamRoom) -> Vec<&'a TeamMember> {
    let explicit = match room {
        TeamRoom::Group { helper_ids, .. } if !helper_ids.is_empty() => Some(helper_ids),
        _ => None,
    };
    members
        .iter()
        .filter(|member| {
            member.active
                && !member.conductor
                && explicit.is_none_or(|ids| ids.iter().any(|id| id == &member.id))
        })
        .collect()
}

fn mentionable_members<'a>(
    members: &'a [TeamMember],
    pool: &[&'a TeamMember],
) -> Vec<&'a TeamMember> {
    members
        .iter()
        .filter(|member| {
            member.active && (member.conductor || pool.iter().any(|picked| picked.id == member.id))
        })
        .collect()
}

fn pool_ids(pool: &[&TeamMember]) -> Vec<String> {
    pool.iter().map(|member| member.id.clone()).collect()
}

/// `@handle`s first, then plain names. `None` means the message named nobody.
fn addressed(
    prompt: &str,
    members: &[TeamMember],
    mentionable: &[&TeamMember],
    pool: &[&TeamMember],
) -> Option<Vec<String>> {
    let mentioned = parse_mentions(prompt, mentionable);
    if !mentioned.is_empty() {
        let non_lead = mentioned
            .iter()
            .filter(|id| {
                !members
                    .iter()
                    .any(|member| member.conductor && member.id == id.as_str())
            })
            .count();
        return Some(if non_lead >= 2 {
            with_lead(mentioned, members)
        } else {
            mentioned
        });
    }
    let named = parse_name_mentions(prompt, pool);
    if named.is_empty() {
        None
    } else if named.len() >= 2 {
        Some(with_lead(named, members))
    } else {
        Some(named)
    }
}

fn with_lead(mut speakers: Vec<String>, members: &[TeamMember]) -> Vec<String> {
    if let Some(lead) = members
        .iter()
        .find(|member| member.active && member.conductor)
        && !speakers.iter().any(|id| id == &lead.id)
    {
        speakers.push(lead.id.clone());
    }
    speakers
}

fn cap_speakers(mut speakers: Vec<String>, members: &[TeamMember]) -> Vec<String> {
    if speakers.len() <= MAX_REPLIES_PER_TURN {
        return speakers;
    }
    let lead = members
        .iter()
        .find(|member| member.conductor)
        .map(|member| member.id.clone());
    let keep_lead = lead
        .as_ref()
        .is_some_and(|id| speakers.iter().any(|speaker| speaker == id));
    speakers.truncate(MAX_REPLIES_PER_TURN);
    if let Some(lead) = lead.filter(|_| keep_lead)
        && !speakers.iter().any(|speaker| speaker == &lead)
    {
        speakers.pop();
        speakers.push(lead);
    }
    speakers
}

/// Jobs in a message that named nobody.
///
/// Each sentence is one job. The helper who matches it most strongly answers
/// it. Helpers who only match a stray word stay out. An equal top score asks
/// each of those helpers, in roster order, and two sentences can book the same
/// helper twice. The lead joins only when two different helpers are answering.
/// A message that matches nobody asks the one helper who has waited longest.
fn jobs_for(
    pool: &[&TeamMember],
    prompt: &str,
    history: &[TranscriptLine],
    members: &[TeamMember],
) -> Vec<String> {
    let mut slots = Vec::new();
    for clause in task_clauses(prompt) {
        for member in strongest_matches(pool, &clause) {
            slots.push(member.id.clone());
        }
    }
    if slots.is_empty() {
        return best_helper(pool, prompt, history).into_iter().collect();
    }
    let mut seen = BTreeSet::new();
    let distinct = slots.iter().filter(|id| seen.insert((*id).clone())).count();
    if distinct >= 2 {
        with_lead(slots, members)
    } else {
        slots
    }
}

/// Sentences, and lines, are separate jobs. A comma or "and" stays in one job,
/// so "I can't sleep and feel stress" is still one helper.
fn task_clauses(prompt: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = String::new();
    for character in prompt.chars() {
        if matches!(character, '.' | '!' | '?' | ';' | '\n') {
            push_clause(&mut clauses, &current);
            current.clear();
        } else {
            current.push(character);
        }
    }
    push_clause(&mut clauses, &current);
    clauses
}

fn push_clause(clauses: &mut Vec<String>, current: &str) {
    let clause = current.trim();
    if !clause.is_empty() {
        clauses.push(clause.to_owned());
    }
}

/// Helpers tied for the strongest match. A weaker speciality word does not
/// add another helper. Nobody matches when every score is zero.
fn strongest_matches<'a>(pool: &[&'a TeamMember], clause: &str) -> Vec<&'a TeamMember> {
    let lower = clause.to_lowercase();
    let scored: Vec<(&TeamMember, usize)> = pool
        .iter()
        .copied()
        .map(|member| (member, expertise_score(member, &lower)))
        .collect();
    let best = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
    if best == 0 {
        return Vec::new();
    }
    scored
        .into_iter()
        .filter(|(_, score)| *score == best)
        .map(|(member, _)| member)
        .collect()
}

fn best_helper(pool: &[&TeamMember], prompt: &str, history: &[TranscriptLine]) -> Option<String> {
    let lower = prompt.to_lowercase();
    let scored: Vec<(&TeamMember, usize)> = pool
        .iter()
        .map(|member| (*member, expertise_score(member, &lower)))
        .collect();
    let best = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
    let tied: Vec<&TeamMember> = if best == 0 {
        pool.to_vec()
    } else {
        scored
            .into_iter()
            .filter(|(_, score)| *score == best)
            .map(|(member, _)| member)
            .collect()
    };
    fairest(&tied, history).map(|member| member.id.clone())
}

fn expertise_score(member: &TeamMember, lower: &str) -> usize {
    member
        .expertise
        .iter()
        .chain(std::iter::once(&member.role))
        .filter(|topic| {
            let topic = topic.to_lowercase();
            !topic.is_empty() && contains_phrase(lower, &topic)
        })
        .count()
}

/// Never-spoken helpers come first, then whoever spoke least recently.
/// Roster order breaks a tie, because `min_by` keeps the earlier equal item.
fn fairest<'a>(pool: &[&'a TeamMember], history: &[TranscriptLine]) -> Option<&'a TeamMember> {
    pool.iter().copied().min_by_key(|member| {
        history.iter().rposition(|line| {
            same_speaker(&line.speaker, &member.name) || same_speaker(&line.speaker, &member.id)
        })
    })
}

fn same_speaker(spoken: &str, name: &str) -> bool {
    spoken.eq_ignore_ascii_case(name)
}

fn parse_mentions(prompt: &str, mentionable: &[&TeamMember]) -> Vec<String> {
    let chars: Vec<char> = prompt.to_lowercase().chars().collect();
    let mut hits = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let at_boundary = index == 0 || chars[index - 1].is_whitespace();
        if at_boundary && chars[index] == '@' {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && is_slug_char(chars[end]) {
                end += 1;
            }
            let slug: String = chars[start..end].iter().collect();
            if let Some(member) = mentionable.iter().find(|member| member.id == slug) {
                hits.push(member.id.clone());
            }
            index = end.max(index + 1);
        } else {
            index += 1;
        }
    }
    dedupe(hits)
}

fn is_slug_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn parse_name_mentions(prompt: &str, pool: &[&TeamMember]) -> Vec<String> {
    let stripped: String = strip_mentions(&prompt.to_lowercase());
    let chars: Vec<char> = stripped.chars().collect();
    let mut candidates = Vec::new();
    for member in pool {
        for token in [member.id.to_lowercase(), member.name.to_lowercase()] {
            if token.chars().count() >= 2 {
                candidates.push((token, member.id.clone()));
            }
        }
    }
    candidates.sort_by_key(|(token, _)| std::cmp::Reverse(token.chars().count()));
    let mut hits: Vec<(usize, String)> = Vec::new();
    for (token, id) in &candidates {
        let needle: Vec<char> = token.chars().collect();
        let mut index = 0;
        while index + needle.len() <= chars.len() {
            if boundary_before(&chars, index)
                && chars[index..index + needle.len()] == needle[..]
                && boundary_after(&chars, index + needle.len())
            {
                hits.push((index, id.clone()));
                index += needle.len();
            } else {
                index += 1;
            }
        }
    }
    hits.sort_by_key(|(at, _)| *at);
    dedupe(hits.into_iter().map(|(_, id)| id).collect())
}

fn strip_mentions(lower: &str) -> String {
    let chars: Vec<char> = lower.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let at_boundary = index == 0 || chars[index - 1].is_whitespace();
        if at_boundary && chars[index] == '@' {
            let mut end = index + 1;
            while end < chars.len() && is_slug_char(chars[end]) {
                end += 1;
            }
            out.push(' ');
            index = end.max(index + 1);
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn dedupe(ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Whole-team phrases from `has_team_trigger`, on word boundaries.
///
/// Whitespace is collapsed first, so `all of you` matches a run of spaces.
/// `roundtable` matches because the separator in `round-table` may be empty.
/// A longer word such as `steam`, `teammate` or `debating` does not.
fn has_team_trigger(content: &str) -> bool {
    if content.trim().is_empty() {
        return false;
    }
    let collapsed = collapse_whitespace(&content.to_lowercase());
    let chars: Vec<char> = collapsed.chars().collect();
    TEAM_PHRASES
        .iter()
        .any(|phrase| phrase_at_boundary(&chars, phrase))
}

const TEAM_PHRASES: &[&str] = &[
    "everyone",
    "everybody",
    "all of you",
    "all three",
    "all four",
    "y'all",
    "yall",
    "the group",
    "the room",
    "who wants",
    "who thinks",
    "debates",
    "debated",
    "debate this",
    "debate",
    "roundtable",
    "round-table",
    "round -table",
    "round table",
    "all hands",
    "team",
];

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for c in input.chars() {
        if c.is_whitespace() {
            pending = true;
        } else {
            if pending && !out.is_empty() {
                out.push(' ');
            }
            pending = false;
            out.push(c);
        }
    }
    out
}

fn phrase_at_boundary(chars: &[char], phrase: &str) -> bool {
    let needle: Vec<char> = phrase.chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return false;
    }
    (0..=chars.len() - needle.len()).any(|index| {
        boundary_before(chars, index)
            && chars[index..index + needle.len()] == needle[..]
            && boundary_after(chars, index + needle.len())
    })
}

fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    phrase_at_boundary(&haystack.chars().collect::<Vec<_>>(), phrase)
}

fn boundary_before(chars: &[char], index: usize) -> bool {
    index == 0 || !is_word_char(chars[index - 1])
}

fn boundary_after(chars: &[char], index: usize) -> bool {
    index >= chars.len() || !is_word_char(chars[index])
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Word-trigram signature of one reply. A pass or an empty reply is empty,
/// so it never counts as a repeat. The lead's marker is removed first.
pub fn round_signature(content: &str) -> BTreeSet<String> {
    let stripped = strip_conductor_marker(content).trim().to_lowercase();
    if stripped.is_empty() || is_pass(&stripped) {
        return BTreeSet::new();
    }
    let words = ascii_words(&stripped);
    if words.len() < 3 {
        return words.into_iter().map(str::to_owned).collect();
    }
    words
        .windows(3)
        .map(|window| format!("{} {} {}", window[0], window[1], window[2]))
        .collect()
}

fn strip_conductor_marker(content: &str) -> String {
    let Some(at) = content.find(CONDUCTOR_TAIL) else {
        return content.to_owned();
    };
    let after = at + CONDUCTOR_TAIL.len();
    let end = content[after..]
        .find("]]")
        .map_or(content.len(), |offset| after + offset + 2);
    let mut out = String::new();
    out.push_str(&content[..at]);
    out.push_str(&content[end..]);
    out
}

fn ascii_words(lower: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let bytes = lower.as_bytes();
    let mut start = None;
    for (index, byte) in bytes.iter().enumerate() {
        if byte.is_ascii_alphanumeric() {
            if start.is_none() {
                start = Some(index);
            }
        } else if let Some(from) = start.take() {
            words.push(&lower[from..index]);
        }
    }
    if let Some(from) = start {
        words.push(&lower[from..]);
    }
    words
}

/// Whether a CONTINUE at the end of `round` should hand back instead.
///
/// Round 1 always continues. Later rounds check confidence, then overlap with
/// the immediately previous round, then how much of the new wording is new.
/// A missing confidence is treated as 1.0, so it does not close by itself.
pub fn continue_hands_back(
    round: u8,
    decision: &super::ConductorDecision,
    previous: &[BTreeSet<String>],
    current: &[BTreeSet<String>],
) -> bool {
    if decision.verdict != super::Verdict::Continue || round < 2 {
        return false;
    }
    if decision.confidence.below_soft_close() {
        return true;
    }
    if rounds_overlap(previous, current) {
        return true;
    }
    new_wording_is_thin(previous, current)
}

fn union_sets(sets: &[BTreeSet<String>]) -> BTreeSet<String> {
    sets.iter().flatten().cloned().collect()
}

/// Jaccard of at least 0.8, compared with integers: `intersection * 5 >= union * 4`.
fn rounds_overlap(previous: &[BTreeSet<String>], current: &[BTreeSet<String>]) -> bool {
    if previous.is_empty() || current.is_empty() {
        return false;
    }
    let previous = union_sets(previous);
    let current = union_sets(current);
    if previous.is_empty() || current.is_empty() {
        return false;
    }
    let intersection = previous.intersection(&current).count();
    let union = previous.len() + current.len() - intersection;
    intersection * 5 >= union * 4
}

/// Novel wording under 0.2 of this round. Exactly 0.2 stays open.
/// An empty round has nothing new, so it hands back.
fn new_wording_is_thin(previous: &[BTreeSet<String>], current: &[BTreeSet<String>]) -> bool {
    let current = union_sets(current);
    if current.is_empty() {
        return true;
    }
    if previous.is_empty() {
        return false;
    }
    let previous = union_sets(previous);
    if previous.is_empty() {
        return false;
    }
    let novel = current.difference(&previous).count();
    novel * 5 < current.len()
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn confidence_rounds_the_soft_close_boundary() {
        assert!(!Confidence::from_unit(0.65).below_soft_close());
        assert!(Confidence::from_unit(0.649).below_soft_close());
        assert!(!Confidence::from_unit(1.5).below_soft_close());
        assert!(Confidence::from_unit(-1.0).below_soft_close());
    }
}
