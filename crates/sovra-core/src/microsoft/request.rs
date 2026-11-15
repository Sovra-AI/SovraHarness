//! Closed Microsoft action set. Models never supply arbitrary URLs or JSON writes.

use serde::{Deserialize, Serialize};

use super::{MicrosoftError, MicrosoftService};

/// One Graph path segment, validated before an adapter URL is constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceId(String);

impl ResourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ResourceId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 1024
            || matches!(value.as_str(), "." | "..")
            || value
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || "/\\?#%".contains(c))
        {
            return Err("Use an item ID returned by Microsoft, not a link or path.".into());
        }
        Ok(Self(value))
    }
}

impl From<ResourceId> for String {
    fn from(value: ResourceId) -> Self {
        value.0
    }
}

/// The adapter limits every collection; `more_available` in results marks truncation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MicrosoftRequest {
    SearchMail {
        query: String,
    },
    ReadMail {
        message_id: ResourceId,
    },
    DraftMail {
        to: String,
        subject: String,
        body: String,
    },
    ListEvents {
        from: String,
        to: String,
    },
    AddEvent {
        subject: String,
        start: String,
        end: String,
        time_zone: String,
    },
    SearchFiles {
        query: String,
    },
    ReadFile {
        item_id: ResourceId,
    },
    SearchContacts {
        query: String,
    },
    ListTaskLists,
    ListTasks {
        list_id: ResourceId,
    },
    AddTask {
        list_id: ResourceId,
        title: String,
    },
    ListNotes,
    ReadNote {
        page_id: ResourceId,
    },
    SearchSites {
        query: String,
    },
    ListSiteDrives {
        site_id: ResourceId,
    },
    SearchSiteFiles {
        drive_id: ResourceId,
        query: String,
    },
    ReadSiteFile {
        drive_id: ResourceId,
        item_id: ResourceId,
    },
    ListChats,
    ReadChat {
        chat_id: ResourceId,
    },
    ListTeams,
    ListChannels {
        team_id: ResourceId,
    },
    ReadChannel {
        team_id: ResourceId,
        channel_id: ResourceId,
    },
    ReadChannelReplies {
        team_id: ResourceId,
        channel_id: ResourceId,
        message_id: ResourceId,
    },
    ListPlannerTasks,
}

impl MicrosoftRequest {
    pub fn service(&self) -> MicrosoftService {
        match self {
            Self::SearchMail { .. } | Self::ReadMail { .. } | Self::DraftMail { .. } => {
                MicrosoftService::Mail
            }
            Self::ListEvents { .. } | Self::AddEvent { .. } => MicrosoftService::Calendar,
            Self::SearchFiles { .. } | Self::ReadFile { .. } => MicrosoftService::Files,
            Self::SearchContacts { .. } => MicrosoftService::Contacts,
            Self::ListTaskLists | Self::ListTasks { .. } | Self::AddTask { .. } => {
                MicrosoftService::Tasks
            }
            Self::ListNotes | Self::ReadNote { .. } => MicrosoftService::Notes,
            Self::SearchSites { .. }
            | Self::ListSiteDrives { .. }
            | Self::SearchSiteFiles { .. }
            | Self::ReadSiteFile { .. } => MicrosoftService::SharePoint,
            Self::ListChats | Self::ReadChat { .. } => MicrosoftService::Chats,
            Self::ListTeams
            | Self::ListChannels { .. }
            | Self::ReadChannel { .. }
            | Self::ReadChannelReplies { .. } => MicrosoftService::Channels,
            Self::ListPlannerTasks => MicrosoftService::Planner,
        }
    }

    pub fn changes_things(&self) -> bool {
        matches!(
            self,
            Self::DraftMail { .. } | Self::AddEvent { .. } | Self::AddTask { .. }
        )
    }

    /// Bound model-authored text before any IO. Date semantics are checked by the
    /// adapter's date parser; these checks prevent control/header injection.
    pub fn validate(&self) -> Result<(), MicrosoftError> {
        match self {
            Self::SearchMail { query }
            | Self::SearchFiles { query }
            | Self::SearchContacts { query }
            | Self::SearchSites { query }
            | Self::SearchSiteFiles { query, .. } => text(query, 512, false),
            Self::DraftMail { to, subject, body } => {
                text(to, 320, true)?;
                let valid_address = to.split_once('@').is_some_and(|(local, host)| {
                    !local.is_empty() && host.contains('.') && !host.contains('@')
                });
                if !valid_address || to.chars().any(|c| c.is_whitespace() || ",;<>".contains(c)) {
                    return Err(MicrosoftError::Failed(
                        "Use one complete email address for the draft.".into(),
                    ));
                }
                text(subject, 512, true)?;
                if body.len() > 20_000 || body.contains('\0') {
                    return Err(MicrosoftError::Failed(
                        "The email draft is too long or contains unsupported text.".into(),
                    ));
                }
                Ok(())
            }
            Self::ListEvents { from, to } => {
                text(from, 64, true)?;
                text(to, 64, true)
            }
            Self::AddEvent {
                subject,
                start,
                end,
                time_zone,
            } => {
                text(subject, 512, true)?;
                text(start, 64, true)?;
                text(end, 64, true)?;
                text(time_zone, 128, true)
            }
            Self::AddTask { title, .. } => text(title, 512, true),
            Self::ReadMail { .. }
            | Self::ReadFile { .. }
            | Self::ListTaskLists
            | Self::ListTasks { .. }
            | Self::ListNotes
            | Self::ReadNote { .. }
            | Self::ListSiteDrives { .. }
            | Self::ReadSiteFile { .. }
            | Self::ListChats
            | Self::ReadChat { .. }
            | Self::ListTeams
            | Self::ListChannels { .. }
            | Self::ReadChannel { .. }
            | Self::ReadChannelReplies { .. }
            | Self::ListPlannerTasks => Ok(()),
        }
    }
}

fn text(value: &str, max: usize, required: bool) -> Result<(), MicrosoftError> {
    if (required && value.trim().is_empty())
        || value.len() > max
        || value.chars().any(char::is_control)
    {
        Err(MicrosoftError::Failed(
            "The Microsoft request contains missing, oversized or unsupported text.".into(),
        ))
    } else {
        Ok(())
    }
}
