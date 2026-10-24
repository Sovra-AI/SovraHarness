//! Chat conversation history domain: threads, recency grouping, `Chats | Scheduled`
//! filter buckets, auto-titling, pinning, renaming, and soft-deletion.
//!
//! Modeled after `SovraSRXProduction`'s `RecentConversationsRail` (`group-conversations.ts`,
//! `chat-filter-pills.tsx`, `scheduled.ts`, and `conversation-row.tsx`). Pure and
//! deterministic: no clocks or IO (callers supply timestamps when needed).

use crate::agui::{ChatMessage, MessageRole};
use crate::memory::Timestamp;

/// The two history rail buckets in `SovraSRX`: ordinary user chats vs scheduler-created runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatHistoryFilter {
    Chats,
    Scheduled,
}

impl ChatHistoryFilter {
    pub const ALL: [Self; 2] = [Self::Chats, Self::Scheduled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Chats => "Chats",
            Self::Scheduled => "Scheduled",
        }
    }
}

/// Discriminates regular user-initiated conversations from headless or triggered schedule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Chat,
    Scheduled,
}

impl ConversationKind {
    pub fn matches_filter(self, filter: ChatHistoryFilter) -> bool {
        match filter {
            ChatHistoryFilter::Chats => self == Self::Chat,
            ChatHistoryFilter::Scheduled => self == Self::Scheduled,
        }
    }
}

/// Recency buckets used to group non-pinned conversations in the history rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecencyBucket {
    Today,
    Yesterday,
    Previous7Days,
    Previous30Days,
    Older,
}

impl RecencyBucket {
    pub const ALL: [Self; 5] = [
        Self::Today,
        Self::Yesterday,
        Self::Previous7Days,
        Self::Previous30Days,
        Self::Older,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::Previous7Days => "Previous 7 Days",
            Self::Previous30Days => "Previous 30 Days",
            Self::Older => "Older",
        }
    }
}

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// Classifies `updated_at` relative to `now` into a [`RecencyBucket`].
pub fn recency_bucket(updated_at: Timestamp, now: Timestamp) -> RecencyBucket {
    let age_ms = (now.millis() - updated_at.millis()).max(0);
    if age_ms < DAY_MS {
        RecencyBucket::Today
    } else if age_ms < 2 * DAY_MS {
        RecencyBucket::Yesterday
    } else if age_ms < 7 * DAY_MS {
        RecencyBucket::Previous7Days
    } else if age_ms < 30 * DAY_MS {
        RecencyBucket::Previous30Days
    } else {
        RecencyBucket::Older
    }
}

/// Formats a compact relative timestamp (`"now"`, `"5m"`, `"3h"`, `"2d"`, `"2w"`).
pub fn format_relative_time(updated_at: Timestamp, now: Timestamp) -> String {
    let diff_ms = (now.millis() - updated_at.millis()).max(0);
    if diff_ms < MINUTE_MS {
        "now".to_owned()
    } else if diff_ms < HOUR_MS {
        format!("{}m", diff_ms / MINUTE_MS)
    } else if diff_ms < DAY_MS {
        format!("{}h", diff_ms / HOUR_MS)
    } else if diff_ms < 7 * DAY_MS {
        format!("{}d", diff_ms / DAY_MS)
    } else {
        format!("{}w", (diff_ms / (7 * DAY_MS)).max(1))
    }
}

/// Default placeholder title for an untitled conversation.
pub const NEW_CHAT_TITLE: &str = "New chat";

/// Generates a concise conversation title from the first user prompt.
pub fn derive_chat_title(prompt: &str) -> String {
    let cleaned = prompt.trim().lines().next().unwrap_or_default().trim();
    if cleaned.is_empty() {
        return NEW_CHAT_TITLE.to_owned();
    }
    let max_chars = 48;
    let char_count = cleaned.chars().count();
    if char_count <= max_chars {
        cleaned.to_owned()
    } else {
        let prefix: String = cleaned.chars().take(max_chars - 1).collect();
        format!("{}…", prefix.trim_end())
    }
}

/// Generates a single-line subtitle preview from the most recent message.
pub fn derive_message_preview(text: &str) -> String {
    let single_line: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let max_chars = 64;
    if single_line.chars().count() <= max_chars {
        single_line
    } else {
        let prefix: String = single_line.chars().take(max_chars - 1).collect();
        format!("{}…", prefix.trim_end())
    }
}

/// Semantic icon category derived from conversation title keywords (mirrors `conversation-icon.tsx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTopicIcon {
    Chat,
    Code,
    Market,
    Housing,
    Schedule,
    Search,
    Document,
}

/// Infers a semantic topic icon from the conversation title and kind.
pub fn infer_topic_icon(title: &str, kind: ConversationKind) -> ConversationTopicIcon {
    if kind == ConversationKind::Scheduled {
        return ConversationTopicIcon::Schedule;
    }
    let lower = title.to_lowercase();
    if [
        "schedul",
        "daily",
        "morning brief",
        "recurring",
        "cron",
        "weekly",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Schedule
    } else if [
        "rust", "code", "bug", "refactor", "trait", "test", "api", "build", "git",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Code
    } else if [
        "stock",
        "market",
        "fund",
        "invest",
        "portfolio",
        "ticker",
        "nasdaq",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Market
    } else if [
        "hous",
        "property",
        "villa",
        "apartment",
        "mortgage",
        "real estate",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Housing
    } else if ["research", "search", "find", "explore", "news", "paper"]
        .iter()
        .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Search
    } else if ["doc", "note", "knowledge", "index", "summary", "email"]
        .iter()
        .any(|k| lower.contains(k))
    {
        ConversationTopicIcon::Document
    } else {
        ConversationTopicIcon::Chat
    }
}

/// A full conversation thread with its metadata and message transcript.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatThread {
    pub id: String,
    pub title: String,
    pub kind: ConversationKind,
    pub schedule_id: Option<String>,
    pub pinned: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_message_preview: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub deleted_at: Option<Timestamp>,
}

impl ChatThread {
    /// Creates a new empty chat thread.
    pub fn new(id: impl Into<String>, created_at: Timestamp) -> Self {
        Self {
            id: id.into(),
            title: NEW_CHAT_TITLE.to_owned(),
            kind: ConversationKind::Chat,
            schedule_id: None,
            pinned: false,
            created_at,
            updated_at: created_at,
            last_message_preview: String::new(),
            messages: Vec::new(),
            deleted_at: None,
        }
    }

    /// Creates a new scheduled conversation thread linked to `schedule_id`.
    pub fn new_scheduled(
        id: impl Into<String>,
        schedule_id: impl Into<String>,
        schedule_name: &str,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id: id.into(),
            title: schedule_name.trim().to_owned(),
            kind: ConversationKind::Scheduled,
            schedule_id: Some(schedule_id.into()),
            pinned: false,
            created_at,
            updated_at: created_at,
            last_message_preview: String::new(),
            messages: Vec::new(),
            deleted_at: None,
        }
    }

    /// True if this thread has not been soft-deleted.
    pub fn is_active(&self) -> bool {
        self.deleted_at.is_none()
    }

    /// True if this thread has no messages and still has the default placeholder title.
    pub fn is_untouched_draft(&self) -> bool {
        self.messages.is_empty()
            && (self.title == NEW_CHAT_TITLE || self.title.trim().is_empty())
            && self.kind == ConversationKind::Chat
    }

    /// Returns true if the thread matches the active bucket (`Chats` vs `Scheduled`) and search query.
    pub fn matches(&self, filter: ChatHistoryFilter, query: &str) -> bool {
        if !self.is_active() || !self.kind.matches_filter(filter) {
            return false;
        }
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.title.to_lowercase().contains(&q)
            || self.last_message_preview.to_lowercase().contains(&q)
            || self
                .messages
                .iter()
                .any(|m| m.text.to_lowercase().contains(&q))
    }

    /// Updates `title` (if still a placeholder on first user message), `last_message_preview`,
    /// and `updated_at` after a message is appended or updated.
    pub fn refresh_summary(&mut self, updated_at: Timestamp) {
        self.updated_at = updated_at;
        if (self.title == NEW_CHAT_TITLE || self.title.trim().is_empty())
            && let Some(first_user) = self
                .messages
                .iter()
                .find(|m| m.role == MessageRole::User && !m.text.trim().is_empty())
        {
            self.title = derive_chat_title(&first_user.text);
        }
        if let Some(last) = self
            .messages
            .iter()
            .rev()
            .find(|m| !m.text.trim().is_empty())
        {
            self.last_message_preview = derive_message_preview(&last.text);
        }
    }
}

/// Partitioned and recency-grouped view of visible conversation threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedHistory<'a> {
    pub pinned: Vec<&'a ChatThread>,
    pub groups: Vec<(RecencyBucket, Vec<&'a ChatThread>)>,
}

impl GroupedHistory<'_> {
    pub fn is_empty(&self) -> bool {
        self.pinned.is_empty() && self.groups.is_empty()
    }
}

/// Filters, sorts (newest `updated_at` first), partitions pinned threads, and buckets the rest by recency.
pub fn group_threads<'a>(
    threads: &'a [ChatThread],
    filter: ChatHistoryFilter,
    query: &str,
    now: Timestamp,
) -> GroupedHistory<'a> {
    let mut matching: Vec<&'a ChatThread> = threads
        .iter()
        .filter(|t| t.matches(filter, query))
        .collect();
    matching.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut pinned = Vec::new();
    let mut unpinned = Vec::new();
    for thread in matching {
        if thread.pinned {
            pinned.push(thread);
        } else {
            unpinned.push(thread);
        }
    }

    let mut groups = Vec::new();
    for bucket in RecencyBucket::ALL {
        let items: Vec<&'a ChatThread> = unpinned
            .iter()
            .copied()
            .filter(|t| recency_bucket(t.updated_at, now) == bucket)
            .collect();
        if !items.is_empty() {
            groups.push((bucket, items));
        }
    }

    GroupedHistory { pinned, groups }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_clean_title_and_preview() {
        assert_eq!(derive_chat_title("   "), NEW_CHAT_TITLE);
        assert_eq!(
            derive_chat_title("Summarize Q3 revenue and margin trends\nExtra line"),
            "Summarize Q3 revenue and margin trends"
        );
        let long = "A".repeat(80);
        let title = derive_chat_title(&long);
        assert!(title.ends_with('…'));
        assert_eq!(title.chars().count(), 48);
    }

    #[test]
    fn partitions_pinned_and_groups_by_recency_and_filter() {
        let now = Timestamp(100 * DAY_MS);
        let mut t_pinned = ChatThread::new("c-pinned", Timestamp(90 * DAY_MS));
        t_pinned.title = "Pinned strategy".into();
        t_pinned.pinned = true;

        let mut t_today = ChatThread::new("c-today", now);
        t_today.title = "Today coding task".into();

        let mut t_yesterday =
            ChatThread::new("c-yesterday", Timestamp(now.millis() - DAY_MS - 1_000));
        t_yesterday.title = "Yesterday review".into();

        let mut t_sched = ChatThread::new_scheduled(
            "c-sched",
            "sched-morning-brief",
            "Executive Morning Brief",
            now,
        );
        t_sched.last_message_preview = "Priority emails summarized".into();

        let threads = vec![t_yesterday, t_pinned, t_today, t_sched];

        let chats = group_threads(&threads, ChatHistoryFilter::Chats, "", now);
        assert_eq!(chats.pinned.len(), 1);
        assert_eq!(chats.pinned[0].id, "c-pinned");
        assert_eq!(chats.groups.len(), 2);
        assert_eq!(chats.groups[0].0, RecencyBucket::Today);
        assert_eq!(chats.groups[0].1[0].id, "c-today");
        assert_eq!(chats.groups[1].0, RecencyBucket::Yesterday);
        assert_eq!(chats.groups[1].1[0].id, "c-yesterday");

        let scheduled = group_threads(&threads, ChatHistoryFilter::Scheduled, "", now);
        assert_eq!(scheduled.pinned, Vec::<&ChatThread>::new());
        assert_eq!(scheduled.groups.len(), 1);
        assert_eq!(scheduled.groups[0].1[0].id, "c-sched");

        let searched = group_threads(&threads, ChatHistoryFilter::Chats, "coding", now);
        assert_eq!(searched.pinned, Vec::<&ChatThread>::new());
        assert_eq!(searched.groups.len(), 1);
        assert_eq!(searched.groups[0].1[0].id, "c-today");
    }
}
