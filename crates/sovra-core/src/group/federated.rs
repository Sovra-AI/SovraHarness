//! Federated recall: a helper's question travels to the computers that hold the
//! memories, is searched there, and only a few short, attributed answers come back.
//!
//! This is the federated sharing mode (`docs/groups.md`):
//! memory stays on its owner's computer and nothing is copied into the group. Both
//! sides opt in separately — answering ("let my group's helpers ask my memory") and
//! asking ("ask my group when I chat"), because the asker's question leaves their
//! computer too. Every rule here is pure; the runtime does the searching and sending.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::keys::DeviceId;
use super::session::shareable_text;
use crate::Timestamp;

/// Longest question a computer will search for another, in characters.
pub const MAX_ASK_CHARS: usize = 500;
/// Most answers one computer returns for one question.
pub const MAX_FEDERATED_ITEMS: usize = 4;
/// Longest single answer, in characters.
pub const MAX_FEDERATED_STATEMENT_CHARS: usize = 600;
/// How many questions one computer may ask another per minute.
pub const ASKS_PER_MINUTE: usize = 20;
/// How many recent questions the owner can review.
pub const MAX_ASK_LOG: usize = 30;

/// One answer from someone else's memory, worded for the group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedItem {
    pub statement: String,
    /// When the owner's computer learned it.
    pub recorded_at: Timestamp,
}

/// A question to search for, trimmed; `None` when empty or too long to be fair.
pub fn check_ask(question: &str) -> Option<String> {
    let question = question.trim();
    (!question.is_empty() && question.chars().count() <= MAX_ASK_CHARS).then(|| question.to_owned())
}

/// Turns the owner's own recall results into answers for the group: at most
/// [`MAX_FEDERATED_ITEMS`], reworded with the owner's name, each kept short.
pub fn federated_items(
    found: impl IntoIterator<Item = (String, Timestamp)>,
    owner_name: &str,
) -> Vec<FederatedItem> {
    let mut items: Vec<FederatedItem> = Vec::new();
    for (statement, recorded_at) in found {
        let worded = shareable_text(&statement, owner_name);
        if worded.is_empty() || items.iter().any(|item| item.statement == worded) {
            continue;
        }
        let mut statement: String = worded.chars().take(MAX_FEDERATED_STATEMENT_CHARS).collect();
        if worded.chars().count() > MAX_FEDERATED_STATEMENT_CHARS {
            statement.push('…');
        }
        items.push(FederatedItem {
            statement,
            recorded_at,
        });
        if items.len() == MAX_FEDERATED_ITEMS {
            break;
        }
    }
    items
}

/// One answer as the person who asked sees it: what, from whom, from which computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAnswer {
    pub statement: String,
    pub from: String,
    pub computer: String,
    pub recorded_at: Timestamp,
}

/// A question someone's helper asked this computer, for the owner to review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskRecord {
    pub asked_by: String,
    pub computer: String,
    pub question: String,
    /// How many answers went back (0 when nothing matched).
    pub answered: usize,
    pub at: Timestamp,
}

/// Remembers the latest questions, newest first, up to [`MAX_ASK_LOG`].
pub fn log_ask(log: &mut Vec<AskRecord>, record: AskRecord) {
    log.insert(0, record);
    log.truncate(MAX_ASK_LOG);
}

/// Per-computer limit on how often another computer may ask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskLimiter {
    recent: BTreeMap<DeviceId, VecDeque<Timestamp>>,
}

impl AskLimiter {
    /// Whether `from` may ask now; records the question when it may.
    pub fn allow(&mut self, from: DeviceId, now: Timestamp) -> bool {
        self.allow_up_to(from, now, ASKS_PER_MINUTE)
    }

    /// Like [`Self::allow`], with another per-minute limit (work jobs use fewer).
    pub fn allow_up_to(&mut self, from: DeviceId, now: Timestamp, per_minute: usize) -> bool {
        const MINUTE_MS: i64 = 60_000;
        let times = self.recent.entry(from).or_default();
        while times
            .front()
            .is_some_and(|at| now.0.saturating_sub(at.0) >= MINUTE_MS)
        {
            times.pop_front();
        }
        if times.len() >= per_minute {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// This computer's choices for federated recall. Both start off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedSettings {
    /// Answer other computers' helpers from this computer's memory.
    #[serde(default)]
    pub answer: bool,
    /// Ask the group's computers when this computer's helpers recall.
    #[serde(default)]
    pub ask: bool,
}
