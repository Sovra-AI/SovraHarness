//! Messaging tools and reviewable outbound messages. Recipient policy is pure;
//! transports and approval enforcement belong to the runtime.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Timestamp;
use crate::connections::{ChannelError, ChannelKind, ConnectionsState, GroupMatch, SyncedGroup};
use crate::people::NameMatch;

pub const MAX_MESSAGE_CHARS: usize = 4096;
pub const MAX_OUTBOUND_MESSAGES: usize = 40;
pub const REVIEW_LIFETIME_MS: i64 = 10 * 60 * 1000;
/// Recipients the person said Sovra may message without a review.
pub const MAX_TRUSTED_RECIPIENTS: usize = 50;
/// A trusted recipient still gets a review after this many sends in one turn,
/// so a runaway helper cannot flood a chat.
pub const MAX_SENDS_WITHOUT_REVIEW_PER_TURN: usize = 3;

/// A chat the person allowed Sovra to message without asking first. Only the
/// native "don't ask again" button adds one, and only after a real send to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedRecipient {
    pub channel: ChannelKind,
    /// Exact resolved chat id, never a name.
    pub recipient: String,
    pub label: String,
}

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
            ChannelKind::WhatsApp if crate::connections::is_whatsapp_group_id(&self.recipient) => {
                "WhatsApp group".into()
            }
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
            GroupMatch::One(group) => {
                let label = if group.subject.is_empty() {
                    group.id.clone()
                } else {
                    group.subject.clone()
                };
                return Ok((group.id.clone(), label));
            }
            GroupMatch::Many => {
                return Err(ChannelError::Failed(
                    "More than one WhatsApp group matches that name. Use the exact group id instead of guessing.".into(),
                ));
            }
            GroupMatch::None => {}
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

/// The one `WhatsApp` group a saved search may message. A name that matches more
/// than one stored group is refused, and a group that is not stored is refused.
pub fn resolve_scheduled_group(
    groups: &[SyncedGroup],
    channel: ChannelKind,
    input: &str,
) -> Result<String, ChannelError> {
    if channel != ChannelKind::WhatsApp {
        return Err(ChannelError::Failed(
            "A search that sends to a group works on WhatsApp.".into(),
        ));
    }
    let mut state = ConnectionsState::default();
    state.upsert_groups(groups);
    match state.resolve_group(input) {
        GroupMatch::One(group) => Ok(group.id.clone()),
        GroupMatch::Many => Err(ChannelError::Failed(
            "More than one WhatsApp group matches that name. Use the exact group id instead of guessing.".into(),
        )),
        GroupMatch::None => Err(ChannelError::Failed(
            "I can't find that WhatsApp group. Say the exact group name or its id.".into(),
        )),
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
        state.upsert_groups(&[SyncedGroup {
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
        state.upsert_groups(&[SyncedGroup {
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

    fn sent(recipient: &str, delivery: MessageDelivery) -> OutboundMessage {
        OutboundMessage {
            id: format!("{recipient}-{delivery:?}"),
            session_id: "desktop".into(),
            channel: ChannelKind::WhatsApp,
            sender: "+15550000000".into(),
            recipient: recipient.into(),
            recipient_label: "School Gate".into(),
            text: "Hi".into(),
            delivery,
            created_at: Timestamp(0),
            expires_at: Timestamp(1),
        }
    }

    #[test]
    fn trust_needs_an_accepted_send_to_that_exact_chat_and_can_be_taken_back() {
        let mut state = ConnectionsState::default();
        let group = "120363111@g.us";
        assert!(!state.set_trusted(ChannelKind::WhatsApp, group, true));
        state
            .outbound_messages
            .push(sent(group, MessageDelivery::Review));
        state
            .outbound_messages
            .push(sent(group, MessageDelivery::Uncertain));
        assert!(
            !state.set_trusted(ChannelKind::WhatsApp, group, true),
            "a review or an unconfirmed send is not a real send"
        );
        state.outbound_messages.push(sent(
            group,
            MessageDelivery::Accepted {
                message_id: "m1".into(),
            },
        ));
        assert!(!state.set_trusted(ChannelKind::Telegram, group, true));
        assert!(state.set_trusted(ChannelKind::WhatsApp, group, true));
        assert!(!state.set_trusted(ChannelKind::WhatsApp, group, true));
        assert!(state.is_trusted(ChannelKind::WhatsApp, group));
        assert!(!state.is_trusted(ChannelKind::WhatsApp, "120363222@g.us"));
        assert_eq!(state.trusted_recipients[0].label, "School Gate");
        let saved: ConnectionsState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(saved.is_trusted(ChannelKind::WhatsApp, group));
        assert!(state.set_trusted(ChannelKind::WhatsApp, group, false));
        assert!(!state.is_trusted(ChannelKind::WhatsApp, group));
        assert!(!state.set_trusted(ChannelKind::WhatsApp, group, false));
    }

    #[test]
    fn group_lookup_matches_words_of_the_name_and_never_lists_everything() {
        let mut state = ConnectionsState::default();
        state.upsert_groups(&[
            SyncedGroup {
                id: "1@g.us".into(),
                subject: "School Gate Parents".into(),
            },
            SyncedGroup {
                id: "2@g.us".into(),
                subject: "Family".into(),
            },
        ]);
        let ids = |query: &str| -> Vec<String> {
            state
                .find_groups(query)
                .iter()
                .map(|group| group.id.clone())
                .collect()
        };
        assert_eq!(ids("school"), ["1@g.us"]);
        assert_eq!(ids("  PARENTS   school "), ["1@g.us"]);
        assert_eq!(ids("2@g.us"), ["2@g.us"]);
        assert_eq!(ids(""), Vec::<String>::new());
        assert_eq!(ids("work"), Vec::<String>::new());
    }

    #[test]
    fn a_group_receipt_shows_a_plain_name_not_a_raw_id() {
        assert_eq!(
            sent("120363111@g.us", MessageDelivery::Review).recipient_display(),
            "WhatsApp group"
        );
        assert_eq!(
            sent("447700900123@s.whatsapp.net", MessageDelivery::Review).recipient_display(),
            "+447700900123"
        );
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
