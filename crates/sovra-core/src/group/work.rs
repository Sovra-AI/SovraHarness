//! Work handoff: a computer with no AI of its own (or a busy one) asks a free
//! computer in its group to answer a chat turn with that computer's AI.
//!
//! Roadmap T4, first slice. What moves is a **text-only chat job**: the
//! conversation's words, never tools, files, memory or keys. The helping computer
//! runs it on its own selected AI with no tools (other people's requests must never
//! reach this computer's tools), so model ports and credentials stay home. Both
//! directions are separate choices and start off: *Let my group use this computer's
//! AI* (helping) and *Use a free computer's AI when this one has none* (asking).
//! Every job a computer runs for someone is logged for its owner.

use serde::{Deserialize, Serialize};

use super::keys::DeviceId;
use crate::Timestamp;

/// Largest job a computer accepts, in bytes of text.
pub const MAX_JOB_BYTES: usize = 256 * 1024;
/// Most turns of conversation a job carries (the newest ones).
pub const MAX_JOB_TURNS: usize = 64;
/// How many jobs one computer may send another per minute.
pub const JOBS_PER_MINUTE: usize = 6;
/// How many jobs the helping computer's owner can review.
pub const MAX_WORK_LOG: usize = 30;

/// One turn of the conversation being answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobTurn {
    /// `true` for the person, `false` for the AI's earlier replies.
    pub from_person: bool,
    pub text: String,
}

/// A chat turn to answer: instructions and the conversation so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatJob {
    pub instructions: String,
    pub turns: Vec<JobTurn>,
}

impl ChatJob {
    /// Keeps the newest turns within [`MAX_JOB_TURNS`] and [`MAX_JOB_BYTES`] (the
    /// latest question always survives), so a long chat still fits.
    #[must_use]
    pub fn trimmed(mut self) -> Self {
        let mut budget = MAX_JOB_BYTES.saturating_sub(self.instructions.len());
        let mut kept = Vec::new();
        for turn in self.turns.into_iter().rev() {
            if kept.len() == MAX_JOB_TURNS || (turn.text.len() > budget && !kept.is_empty()) {
                break;
            }
            budget = budget.saturating_sub(turn.text.len());
            kept.push(turn);
        }
        kept.reverse();
        self.turns = kept;
        self
    }

    fn size(&self) -> usize {
        self.instructions.len() + self.turns.iter().map(|turn| turn.text.len()).sum::<usize>()
    }
}

/// Why a job could not be sent or run, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkError {
    #[error("there's nothing to answer")]
    Empty,
    #[error("that conversation is too long to hand to another computer")]
    TooBig,
    #[error("no computer in your group is free to help right now")]
    NobodyFree,
    #[error("the helping computer couldn't answer: {0}")]
    Failed(String),
}

/// The rules a job must meet, checked by the sender and by the helping computer.
pub fn check_job(job: &ChatJob) -> Result<(), WorkError> {
    let Some(last) = job.turns.last() else {
        return Err(WorkError::Empty);
    };
    if !last.from_person || last.text.trim().is_empty() {
        return Err(WorkError::Empty);
    }
    if job.size() > MAX_JOB_BYTES || job.turns.len() > MAX_JOB_TURNS {
        return Err(WorkError::TooBig);
    }
    Ok(())
}

/// An answer, and who helped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobAnswer {
    pub text: String,
    pub helper: String,
    pub computer: String,
}

/// A job this computer ran for someone, for its owner to review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub asked_by: String,
    pub computer: String,
    pub at: Timestamp,
    /// Whether an answer went back.
    pub answered: bool,
}

/// Remembers the latest jobs, newest first, up to [`MAX_WORK_LOG`].
pub fn log_work(log: &mut Vec<WorkRecord>, record: WorkRecord) {
    log.insert(0, record);
    log.truncate(MAX_WORK_LOG);
}

/// This computer's choices for work handoff. Both start off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSettings {
    /// Answer group members' chat jobs with this computer's AI.
    #[serde(default)]
    pub help_others: bool,
    /// When this computer has no AI connected, ask a free computer in the group.
    #[serde(default)]
    pub use_group: bool,
}

/// Runs a job on this computer's own AI, with no tools. Implemented by the harness.
#[async_trait::async_trait]
pub trait WorkModel: Send + Sync {
    async fn answer(&self, job: ChatJob) -> Result<String, WorkError>;
}

/// Hands a job to a free computer in the group. Implemented by the runtime.
#[async_trait::async_trait]
pub trait WorkPeer: Send + Sync {
    async fn run(&self, job: ChatJob) -> Result<JobAnswer, WorkError>;
}

/// The order to try helpers in: computers heard from most recently first (most
/// likely awake), then the rest; never this computer.
pub fn helper_order(
    candidates: impl IntoIterator<Item = DeviceId>,
    last_seen: &std::collections::BTreeMap<DeviceId, Timestamp>,
    me: &DeviceId,
) -> Vec<DeviceId> {
    let mut order: Vec<DeviceId> = candidates
        .into_iter()
        .filter(|device| device != me)
        .collect();
    order.sort_by(|a, b| {
        last_seen
            .get(b)
            .cmp(&last_seen.get(a))
            .then_with(|| a.cmp(b))
    });
    order.dedup();
    order
}
