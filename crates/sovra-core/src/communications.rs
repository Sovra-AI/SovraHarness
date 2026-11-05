//! Messaging tools and reviewable outbound messages. Recipient policy is pure;
//! transports and approval enforcement belong to the runtime.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Timestamp;
use crate::connections::{ChannelError, ChannelKind, ConnectionsState};
use crate::people::NameMatch;

pub const MAX_MESSAGE_CHARS: usize = 4096;
pub const MAX_OUTBOUND_MESSAGES: usize = 40;
pub const REVIEW_LIFETIME_MS: i64 = 10 * 60 * 1000;

/// A tool cannot approve a message. Only the native review command can do that.
/// A confirmed timed send stored on a schedule transmits at fire time without
/// another review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagingRequest {
    Status,
    RecentChats {
        channel: ChannelKind,
    },
    ReadChat {
        channel: ChannelKind,
        chat: String,
    },
    Prepare {
        channel: ChannelKind,
        recipient: String,
        text: String,
    },
    Receipt {
        id: String,
    },
    /// A saved-person lookup. An empty query does not return the whole list.
    FindPeople {
        query: String,
    },
}

/// Bound to a trusted desktop conversation by the runtime, including in delegates.
#[async_trait]
pub trait MessagingAccess: Send + Sync {
    async fn call(&self, request: MessagingRequest) -> Result<Value, ChannelError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageDelivery {
    Review,
    Sending,
    Accepted {
        message_id: String,
    },
    Cancelled,
    Expired,
    NotSent,
    /// The transport may already have sent it. Never retry automatically.
    Uncertain,
}

impl MessageDelivery {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Review => "Review before sending",
            Self::Sending => "Sending…",
            Self::Accepted { .. } => {
                "Accepted by the messaging service. Delivery and reading are not confirmed."
            }
            Self::Cancelled => "Cancelled — not sent",
            Self::Expired => "Review expired — ask for a new message",
            Self::NotSent => {
                "Not sent — the review could not be saved. Try again when storage is available."
            }
            Self::Uncertain => {
                "Sending could not be confirmed. Check your phone before trying again."
            }
        }
    }
}

/// Exact immutable content displayed by the review UI; the model never gets an
/// approval token or a transport handle. Old records remain useful as receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub id: String,
    pub session_id: String,
    pub channel: ChannelKind,
    pub sender: String,
    pub recipient: String,
    pub recipient_label: String,
    pub text: String,
    pub delivery: MessageDelivery,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

impl OutboundMessage {
    pub fn recipient_display(&self) -> String {
        match self.channel {
            ChannelKind::WhatsApp => self
                .recipient
                .strip_suffix("@s.whatsapp.net")
                .map_or_else(|| self.recipient.clone(), |number| format!("+{number}")),
            ChannelKind::Telegram => format!("Chat {}", self.recipient),
        }
    }

    pub fn may_review(&self, session_id: &str, now: Timestamp) -> bool {
        self.session_id == session_id
            && self.delivery == MessageDelivery::Review
            && now.0 < self.expires_at.0
    }
}

pub fn validate_message(text: &str) -> Result<(), ChannelError> {
    if text.trim().is_empty() || text.chars().count() > MAX_MESSAGE_CHARS || text.contains('\0') {
        return Err(ChannelError::Failed(format!(
            "Write a message between 1 and {MAX_MESSAGE_CHARS} characters."
        )));
    }
    Ok(())
}

/// Accept `self`, an exact observed direct-chat id, an explicit international
/// `WhatsApp` phone number, one saved person, or a unique stored `WhatsApp`
/// group (exact group id or unique subject). A name that matches more than
/// one person or group is never resolved to the first.
pub fn resolve_recipient(
    state: &ConnectionsState,
    channel: ChannelKind,
    input: &str,
    own_chat: &str,
) -> Result<(String, String), ChannelError> {
    let input = input.trim();
    if input == "me" || input == own_chat {
        return Ok((own_chat.to_owned(), "Your own chat".into()));
    }
    if let Some(message) = state.recent_messages.iter().find(|message| {
        message.channel == channel && message.chat_jid == input && direct_chat(channel, input)
    }) {
        return Ok((input.to_owned(), message.sender_name.clone()));
    }
    if channel == ChannelKind::WhatsApp
        && input.starts_with('+')
        && input[1..]
            .chars()
            .all(|c| c.is_ascii_digit() || " ()-".contains(c))
    {
        let digits: String = input.chars().filter(char::is_ascii_digit).collect();
        if (7..=15).contains(&digits.len()) && !digits.starts_with('0') {
            return Ok((format!("{digits}@s.whatsapp.net"), format!("+{digits}")));
        }
    }
    if let Some(person_id) = input.strip_prefix("person:") {
        return state.people.messaging_address(person_id, channel);
    }
    match state.people.resolve_name(input) {
        NameMatch::One(person_id) => state.people.messaging_address(&person_id, channel),
        NameMatch::Many => Err(ChannelError::Failed(
            "More than one saved person matches that name. Say the full name instead of guessing."
                .into(),
        )),
        NameMatch::None => resolve_group_or_unknown(state, channel, input),
    }
}

fn resolve_group_or_unknown(
    state: &ConnectionsState,
    channel: ChannelKind,
    input: &str,
) -> Result<(String, String), ChannelError> {
    if channel == ChannelKind::WhatsApp {
        match state.resolve_group(input) {
            crate::connections::GroupMatch::One(group) => {
                let label = if group.subject.is_empty() {
                    group.id.clone()
                } else {
                    group.subject.clone()
                };
                return Ok((group.id.clone(), label));
            }
            crate::connections::GroupMatch::Many => {
                return Err(ChannelError::Failed(
                    "More than one WhatsApp group matches that name. Use the exact group id instead of guessing.".into(),
                ));
            }
            crate::connections::GroupMatch::None => {}
        }
    }
    Err(ChannelError::Failed(
        "Choose 'me' or an exact chat id from recent chats. For WhatsApp you may also give a phone number starting with + and its country code, the full name of one saved person, a unique WhatsApp group name, or the group's exact id. Ask when more than one person or group matches. Telegram recipients must have contacted this bot first, unless you add that chat on the person.".into(),
    ))
}

/// Recipients a timed send may store without a live people or chat lookup.
/// Fire-time [`resolve_recipient`] still maps these to a chat id. A first name
/// is refused here so a job cannot sit until due and then fail instantly.
pub fn concrete_scheduled_recipient(channel: ChannelKind, input: &str) -> Result<(), ChannelError> {
    let input = input.trim();
    if input == "me" {
        return Ok(());
    }
    if let Some(id) = input.strip_prefix("person:") {
        if id.trim().is_empty() {
            return Err(ChannelError::Failed(
                "That saved person id is empty. Use person:<id> from a people lookup.".into(),
            ));
        }
        return Ok(());
    }
    match channel {
        ChannelKind::WhatsApp => {
            if input.starts_with('+')
                && input[1..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || " ()-".contains(c))
            {
                let digits: String = input.chars().filter(char::is_ascii_digit).collect();
                if (7..=15).contains(&digits.len()) && !digits.starts_with('0') {
                    return Ok(());
                }
            }
            if direct_chat(channel, input) || crate::connections::is_whatsapp_group_id(input) {
                return Ok(());
            }
            Err(ChannelError::Failed(
                "For a timed WhatsApp, use the international phone number starting with +, me, an exact chat id, a WhatsApp group id, or person:<id> from a people lookup. A first name alone is not enough.".into(),
            ))
        }
        ChannelKind::Telegram => {
            if direct_chat(channel, input) {
                return Ok(());
            }
            Err(ChannelError::Failed(
                "For a timed Telegram text, use me, the numeric chat id, or person:<id> from a people lookup. A first name alone is not enough.".into(),
            ))
        }
    }
}

fn direct_chat(channel: ChannelKind, chat: &str) -> bool {
    match channel {
        ChannelKind::WhatsApp => chat.split_once('@').is_some_and(|(user, server)| {
            !user.is_empty()
                && user.chars().all(|c| c.is_ascii_digit())
                && matches!(server, "s.whatsapp.net" | "lid")
        }),
        ChannelKind::Telegram => !chat.is_empty() && chat.chars().all(|c| c.is_ascii_digit()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_require_exact_observation_or_an_international_phone() {
        let state = ConnectionsState::default();
        let own = "15550000000@s.whatsapp.net";
        assert_eq!(
            resolve_recipient(&state, ChannelKind::WhatsApp, "me", own)
                .unwrap()
                .0,
            own
        );
        assert_eq!(
            resolve_recipient(&state, ChannelKind::WhatsApp, "+44 (7700) 900123", own)
                .unwrap()
                .0,
            "447700900123@s.whatsapp.net"
        );
        for bad in [
            "Anna",
            "447700900123",
            "+00 447700900123",
            "+44name7700900123",
            "1@g.us",
            "1@broadcast",
            "12@s.whatsapp.net",
        ] {
            assert!(
                resolve_recipient(&state, ChannelKind::WhatsApp, bad, own).is_err(),
                "{bad}"
            );
        }
        assert!(resolve_recipient(&state, ChannelKind::Telegram, "123", "456").is_err());
    }

    #[test]
    fn a_unique_stored_group_is_a_review_recipient_and_an_ambiguous_subject_is_not() {
        let mut state = ConnectionsState::default();
        let own = "15550000000@s.whatsapp.net";
        state.upsert_groups(&[crate::connections::SyncedGroup {
            id: "120363111@g.us".into(),
            subject: "School Gate".into(),
        }]);
        let (jid, label) =
            resolve_recipient(&state, ChannelKind::WhatsApp, "School Gate", own).unwrap();
        assert_eq!(jid, "120363111@g.us");
        assert_eq!(label, "School Gate");
        assert_eq!(
            resolve_recipient(&state, ChannelKind::WhatsApp, "120363111@g.us", own)
                .unwrap()
                .0,
            "120363111@g.us"
        );
        state.upsert_groups(&[crate::connections::SyncedGroup {
            id: "120363222@g.us".into(),
            subject: "School Gate".into(),
        }]);
        let ambiguous =
            resolve_recipient(&state, ChannelKind::WhatsApp, "School Gate", own).unwrap_err();
        assert!(
            ambiguous.to_string().contains("More than one"),
            "{ambiguous}"
        );
        assert!(resolve_recipient(&state, ChannelKind::WhatsApp, "120363222@g.us", own).is_ok());
    }

    #[test]
    fn timed_send_stores_a_number_or_saved_id_not_a_first_name() {
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "Ryan").is_err());
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "Anna").is_err());
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "447347529758").is_err());
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "+44 7347 529758").is_ok());
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "me").is_ok());
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "person:abc").is_ok());
        assert!(
            concrete_scheduled_recipient(ChannelKind::WhatsApp, "447347529758@s.whatsapp.net")
                .is_ok()
        );
        assert!(concrete_scheduled_recipient(ChannelKind::WhatsApp, "120363@g.us").is_ok());
        assert!(concrete_scheduled_recipient(ChannelKind::Telegram, "Ryan").is_err());
        assert!(concrete_scheduled_recipient(ChannelKind::Telegram, "123456").is_ok());
        assert!(concrete_scheduled_recipient(ChannelKind::Telegram, "me").is_ok());
    }

    #[test]
    fn message_limits_count_unicode_characters_without_changing_the_message() {
        assert!(validate_message(&"😀".repeat(MAX_MESSAGE_CHARS)).is_ok());
        assert!(validate_message(&"a".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
        assert!(validate_message(" \n ").is_err());
        assert!(validate_message("hello\0").is_err());
    }

    #[test]
    fn review_is_bound_to_conversation_state_and_deadline() {
        let mut message = OutboundMessage {
            id: "draft".into(),
            session_id: "desktop".into(),
            channel: ChannelKind::WhatsApp,
            sender: "me".into(),
            recipient: "me".into(),
            recipient_label: "Me".into(),
            text: "Hi".into(),
            delivery: MessageDelivery::Review,
            created_at: Timestamp(10),
            expires_at: Timestamp(20),
        };
        assert!(message.may_review("desktop", Timestamp(19)));
        assert!(!message.may_review("other", Timestamp(19)));
        assert!(!message.may_review("desktop", Timestamp(20)));
        message.delivery = MessageDelivery::Sending;
        assert!(!message.may_review("desktop", Timestamp(19)));
    }
}
