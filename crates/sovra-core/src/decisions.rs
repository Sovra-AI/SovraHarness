//! Company decision records: explicit evidence, human acceptance and retained history.
//!
//! Identity and time are supplied by the runtime. Only the company's written stops
//! are checked; prose is never interpreted as an automatic policy or legal rule.

use serde::{Deserialize, Serialize};

use crate::{AuthSession, Timestamp};

#[cfg(test)]
mod tests;

pub const DECISION_PAGE_SIZE: usize = 100;
const MAX_VERSIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionPerson {
    pub account_id: String,
    pub name: String,
}

impl From<&AuthSession> for DecisionPerson {
    fn from(session: &AuthSession) -> Self {
        Self {
            account_id: session.account_id.clone(),
            name: session.full_name.clone(),
        }
    }
}

/// A person's explicit check of a company-written condition. No built-in rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionStop {
    pub condition: String,
    pub cleared: bool,
}

/// The saved quotation remains readable when the original document changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionEvidence {
    pub reference: String,
    pub passage: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionDraft {
    pub question: String,
    pub decision: String,
    pub criteria: String,
    pub evidence: Vec<DecisionEvidence>,
    /// UTC calendar day, YYYY-MM-DD. Acceptance expires at the start of this day.
    pub review_on: String,
    pub ends_when: String,
    pub hard_stops: Vec<DecisionStop>,
    /// Optional original chat reference and wording, retained when the decision is edited.
    pub chat: Option<DecisionEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionAcceptance {
    pub by: DecisionPerson,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionEnding {
    pub by: DecisionPerson,
    pub at: Timestamp,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionVersion {
    pub draft: DecisionDraft,
    pub drafted_by: DecisionPerson,
    pub saved_at: Timestamp,
    pub accepted: Option<DecisionAcceptance>,
    pub ended: Option<DecisionEnding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    /// Changes on every mutation, including acceptance and ending, to reject stale actions.
    pub revision: u64,
    pub versions: Vec<DecisionVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionStatus {
    Draft,
    Blocked,
    Accepted,
    ReviewDue,
    Ended,
}

impl DecisionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "Needs a second person's acceptance",
            Self::Blocked => "Blocked by your hard stops",
            Self::Accepted => "Accepted company position",
            Self::ReviewDue => "Review due — needs a new decision",
            Self::Ended => "No longer valid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub id: String,
    pub question: String,
    pub review_on: String,
    pub blocked: bool,
    pub accepted: bool,
    pub ended: bool,
}

impl DecisionSummary {
    pub fn status(&self, now: Timestamp) -> DecisionStatus {
        if self.ended {
            DecisionStatus::Ended
        } else if self.blocked {
            DecisionStatus::Blocked
        } else if self.accepted {
            if review_date(&self.review_on).is_ok_and(|at| now < at) {
                DecisionStatus::Accepted
            } else {
                DecisionStatus::ReviewDue
            }
        } else {
            DecisionStatus::Draft
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DecisionError(pub String);

fn required(value: &str, label: &str, limit: usize) -> Result<(), DecisionError> {
    if value.trim().is_empty() || value.len() > limit {
        return Err(DecisionError(format!(
            "{label} is required and must fit in {limit} bytes."
        )));
    }
    Ok(())
}

impl DecisionDraft {
    pub fn validate(&self) -> Result<(), DecisionError> {
        required(&self.question, "The question", 512)?;
        required(&self.decision, "The decision", 16_384)?;
        required(&self.criteria, "Your criteria", 16_384)?;
        required(&self.ends_when, "What would end this decision", 8_192)?;
        review_date(&self.review_on)?;
        if self.evidence.is_empty() || self.evidence.len() > 16 || self.hard_stops.len() > 32 {
            return Err(DecisionError(
                "Include 1–16 cited passages and at most 32 hard stops.".into(),
            ));
        }
        for evidence in self.evidence.iter().chain(self.chat.iter()) {
            required(&evidence.reference, "The source reference", 2_048)?;
            required(&evidence.passage, "The cited passage", 16_384)?;
        }
        for stop in &self.hard_stops {
            required(&stop.condition, "The hard stop", 1_024)?;
        }
        Ok(())
    }
}

impl DecisionRecord {
    /// A readable, dated copy. Earlier acceptances are explicitly historical.
    pub fn text(&self, now: Timestamp) -> String {
        use std::fmt::Write as _;
        let mut text = format!(
            "Decision record {}\nCopy made {}\n",
            self.id,
            decision_time(now)
        );
        if let Ok(summary) = self.summary() {
            let _ = writeln!(text, "Current status: {}", summary.status(now).label());
        }
        for (index, version) in self.versions.iter().enumerate() {
            let draft = &version.draft;
            let _ = write!(
                text,
                "\nVersion {} ({})\nDrafted by {} [{}] on {}\n\nQuestion: {}\n\nDecision: {}\n\nOur criteria: {}\n\nReview on (UTC): {}\n\nEnds when: {}\n",
                index + 1,
                if index + 1 == self.versions.len() {
                    "current"
                } else {
                    "historical"
                },
                version.drafted_by.name,
                version.drafted_by.account_id,
                decision_time(version.saved_at),
                draft.question,
                draft.decision,
                draft.criteria,
                draft.review_on,
                draft.ends_when
            );
            for evidence in &draft.evidence {
                let _ = write!(
                    text,
                    "\nSource: {}\nCited passage: {}\n",
                    evidence.reference, evidence.passage
                );
            }
            for stop in &draft.hard_stops {
                let _ = writeln!(
                    text,
                    "Hard stop ({}): {}",
                    if stop.cleared { "met" } else { "unresolved" },
                    stop.condition
                );
            }
            if let Some(chat) = &draft.chat {
                let _ = write!(
                    text,
                    "\nOriginal chat: {}\n{}\n",
                    chat.reference, chat.passage
                );
            }
            if let Some(accepted) = &version.accepted {
                let _ = writeln!(
                    text,
                    "Accepted by {} [{}] on {}",
                    accepted.by.name,
                    accepted.by.account_id,
                    decision_time(accepted.at)
                );
            }
            if let Some(ended) = &version.ended {
                let _ = writeln!(
                    text,
                    "Ended by {} [{}] on {}: {}",
                    ended.by.name,
                    ended.by.account_id,
                    decision_time(ended.at),
                    ended.reason
                );
            }
        }
        text
    }

    pub fn create(
        id: String,
        draft: DecisionDraft,
        by: DecisionPerson,
        now: Timestamp,
    ) -> Result<Self, DecisionError> {
        required(&id, "The record ID", 128)?;
        draft.validate()?;
        required(&by.account_id, "A signed-in account", 128)?;
        required(&by.name, "The person's name", 512)?;
        Ok(Self {
            id,
            revision: 1,
            versions: vec![DecisionVersion {
                draft,
                drafted_by: by,
                saved_at: now,
                accepted: None,
                ended: None,
            }],
        })
    }

    pub fn current(&self) -> Result<&DecisionVersion, DecisionError> {
        self.versions
            .last()
            .ok_or_else(|| DecisionError("This decision has no saved version.".into()))
    }

    pub fn summary(&self) -> Result<DecisionSummary, DecisionError> {
        let version = self.current()?;
        Ok(DecisionSummary {
            id: self.id.clone(),
            question: version.draft.question.clone(),
            review_on: version.draft.review_on.clone(),
            blocked: version.draft.hard_stops.iter().any(|stop| !stop.cleared),
            accepted: version.accepted.is_some(),
            ended: version.ended.is_some(),
        })
    }

    fn check_revision(&self, expected: u64) -> Result<(), DecisionError> {
        if self.revision != expected {
            return Err(DecisionError(
                "This decision changed. Reopen it before trying again.".into(),
            ));
        }
        Ok(())
    }

    pub fn edit(
        &self,
        expected: u64,
        mut draft: DecisionDraft,
        by: DecisionPerson,
        now: Timestamp,
    ) -> Result<Self, DecisionError> {
        self.check_revision(expected)?;
        if self.versions.len() >= MAX_VERSIONS {
            return Err(DecisionError(
                "This record has 128 versions. Start a new decision and cite this record.".into(),
            ));
        }
        // A later editor cannot replace the original chat attribution.
        draft.chat.clone_from(&self.current()?.draft.chat);
        let mut next = Self::create(self.id.clone(), draft, by, now)?;
        let version = next.versions.remove(0);
        next.versions.clone_from(&self.versions);
        next.versions.push(version);
        next.revision = self.revision + 1;
        Ok(next)
    }

    pub fn can_accept(&self, by: &DecisionPerson, now: Timestamp) -> Result<(), DecisionError> {
        let version = self.current()?;
        require_real_account(by)?;
        if self
            .versions
            .iter()
            .any(|version| version.drafted_by.account_id == by.account_id)
        {
            return Err(DecisionError("A second person must sign in with their own account to accept this decision. Its writers cannot accept it.".into()));
        }
        if version.accepted.is_some() || version.ended.is_some() {
            return Err(DecisionError(
                "Save a new version before accepting this decision again.".into(),
            ));
        }
        version.draft.validate()?;
        if version.draft.hard_stops.iter().any(|stop| !stop.cleared) {
            return Err(DecisionError(
                "Clear each company-written hard stop before acceptance.".into(),
            ));
        }
        if now >= review_date(&version.draft.review_on)? {
            return Err(DecisionError(
                "The review date has arrived. Save a new version with a future review date.".into(),
            ));
        }
        Ok(())
    }

    pub fn accept(
        &self,
        expected: u64,
        by: DecisionPerson,
        now: Timestamp,
    ) -> Result<Self, DecisionError> {
        self.check_revision(expected)?;
        self.can_accept(&by, now)?;
        let mut next = self.clone();
        next.versions
            .last_mut()
            .expect("current version validated")
            .accepted = Some(DecisionAcceptance { by, at: now });
        next.revision += 1;
        Ok(next)
    }

    pub fn end(
        &self,
        expected: u64,
        reason: String,
        by: DecisionPerson,
        now: Timestamp,
    ) -> Result<Self, DecisionError> {
        self.check_revision(expected)?;
        required(&reason, "Why this decision ended", 8_192)?;
        require_real_account(&by)?;
        if self.current()?.ended.is_some() {
            return Err(DecisionError("This decision has already ended.".into()));
        }
        let mut next = self.clone();
        next.versions
            .last_mut()
            .expect("current version validated")
            .ended = Some(DecisionEnding {
            by,
            at: now,
            reason,
        });
        next.revision += 1;
        Ok(next)
    }
}

/// A guest visit is not a second person. Drafting stays allowed.
fn require_real_account(by: &DecisionPerson) -> Result<(), DecisionError> {
    required(&by.account_id, "A signed-in account", 128)?;
    required(&by.name, "The person's name", 512)?;
    if by.account_id == crate::auth::GUEST_ACCOUNT_ID {
        return Err(DecisionError(
            "Sign in with your own account before accepting or ending a company decision.".into(),
        ));
    }
    Ok(())
}

/// Strict Gregorian date parsing without a clock, timezone, or IO dependency.
pub fn review_date(value: &str) -> Result<Timestamp, DecisionError> {
    let invalid =
        || DecisionError("Use a real review date as YYYY-MM-DD, from 1970 to 9999.".into());
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let year: i64 = value[..4].parse().map_err(|_| invalid())?;
    let month: usize = value[5..7].parse().map_err(|_| invalid())?;
    let day: i64 = value[8..].parse().map_err(|_| invalid())?;
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day < 1
        || day > months[month - 1]
    {
        return Err(invalid());
    }
    let years: i64 = (1970..year).map(|y| if leap(y) { 366 } else { 365 }).sum();
    Ok(Timestamp(
        (years + months[..month - 1].iter().sum::<i64>() + day - 1) * 86_400_000,
    ))
}

/// Exact UTC date and minute for a saved action; independent of the viewing machine.
pub fn decision_time(at: Timestamp) -> String {
    let (year, month, day, _) = crate::civil_date_from_unix_days(at.0.div_euclid(86_400_000));
    let minute = at.0.div_euclid(60_000).rem_euclid(1_440);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minute / 60,
        minute % 60
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionCommand {
    Load {
        offset: usize,
    },
    Open {
        id: String,
    },
    Save {
        id: Option<String>,
        expected_revision: u64,
        draft: Box<DecisionDraft>,
    },
    Accept {
        id: String,
        expected_revision: u64,
    },
    End {
        id: String,
        expected_revision: u64,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionEvent {
    Listed {
        records: Vec<DecisionSummary>,
        offset: usize,
        more: bool,
    },
    Opened(Box<DecisionRecord>),
    Saved(Box<DecisionRecord>),
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionBook {
    pub records: Vec<DecisionSummary>,
    pub open: Option<Box<DecisionRecord>>,
    pub offset: usize,
    pub more: bool,
    pub error: Option<String>,
}

impl DecisionBook {
    pub fn apply(&mut self, event: &DecisionEvent) {
        self.error = None;
        match event {
            DecisionEvent::Listed {
                records,
                offset,
                more,
            } => {
                self.records.clone_from(records);
                self.offset = *offset;
                self.more = *more;
            }
            DecisionEvent::Opened(record) | DecisionEvent::Saved(record) => {
                self.open = Some(record.clone());
                if let Ok(summary) = record.summary()
                    && let Some(existing) = self
                        .records
                        .iter_mut()
                        .find(|existing| existing.id == record.id)
                {
                    *existing = summary;
                }
            }
            DecisionEvent::Error(message) => self.error = Some(message.clone()),
        }
    }
}
