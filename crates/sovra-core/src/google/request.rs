//! Everything an agent may ask of Google, one variant per tool.
//!
//! Writes stay inside the person's own account and are easy to undo: email is
//! only ever drafted (they send it from Gmail), and events invite nobody. That
//! keeps a prompt hidden in an email from sending anything out on its own.

use super::GoogleService;

/// Largest result list a tool may ask for, so replies stay readable.
pub const MAX_RESULTS: u32 = 25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleRequest {
    /// Gmail search syntax, e.g. `from:anna newer_than:7d`.
    SearchMail {
        query: String,
        limit: u32,
    },
    ReadMail {
        id: String,
    },
    /// Saves a draft; never sends. `reply_to` is the message being answered.
    DraftMail {
        to: Vec<String>,
        subject: String,
        body: String,
        reply_to: Option<String>,
    },
    /// RFC 3339 bounds; the adapter defaults to the next seven days.
    ListEvents {
        from: Option<String>,
        to: Option<String>,
        query: Option<String>,
        limit: u32,
    },
    /// `start`/`end` are RFC 3339 date-times, or `YYYY-MM-DD` for all-day events.
    CreateEvent {
        title: String,
        start: String,
        end: String,
        location: Option<String>,
        description: Option<String>,
    },
    SearchFiles {
        query: String,
        limit: u32,
    },
    ReadFile {
        id: String,
    },
    SearchContacts {
        query: String,
        limit: u32,
    },
    ListTasks {
        limit: u32,
    },
    /// `due` is a `YYYY-MM-DD` date (Google Tasks keeps only the day).
    AddTask {
        title: String,
        notes: Option<String>,
        due: Option<String>,
    },
}

impl GoogleRequest {
    pub fn service(&self) -> GoogleService {
        match self {
            Self::SearchMail { .. } | Self::ReadMail { .. } | Self::DraftMail { .. } => {
                GoogleService::Mail
            }
            Self::ListEvents { .. } | Self::CreateEvent { .. } => GoogleService::Calendar,
            Self::SearchFiles { .. } | Self::ReadFile { .. } => GoogleService::Files,
            Self::SearchContacts { .. } => GoogleService::Contacts,
            Self::ListTasks { .. } | Self::AddTask { .. } => GoogleService::Tasks,
        }
    }

    /// Whether it adds something to the account (a draft, an event, a to-do).
    pub fn changes_things(&self) -> bool {
        match self {
            Self::DraftMail { .. } | Self::CreateEvent { .. } | Self::AddTask { .. } => true,
            Self::SearchMail { .. }
            | Self::ReadMail { .. }
            | Self::ListEvents { .. }
            | Self::SearchFiles { .. }
            | Self::ReadFile { .. }
            | Self::SearchContacts { .. }
            | Self::ListTasks { .. } => false,
        }
    }
}

/// A tool's requested count, kept between 1 and [`MAX_RESULTS`] (default 10).
pub fn result_limit(requested: Option<u64>) -> u32 {
    requested
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(10)
        .clamp(1, MAX_RESULTS)
}
