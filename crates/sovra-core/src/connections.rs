//! Personal messaging connections ("Connections" in Settings), starting with `WhatsApp`.
//!
//! Pure domain: the link's status, the settings people can change, the reducer the UI
//! folds events with, the port a messaging adapter implements, and the rule that decides
//! which incoming messages Sovra may answer. The real `WhatsApp` Web client lives in
//! `sovra-whatsapp`; `sovra-runtime` drives it and runs the answers through the same
//! helpers as the chat window.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::memory::Timestamp;

/// Chat sessions that answer `WhatsApp` messages carry this prefix, so the runtime can keep
/// their replies off the chat window and send them back to the phone instead.
pub const WHATSAPP_SESSION_PREFIX: &str = "whatsapp:";

/// How many bridged messages the Connections page keeps.
pub const MAX_RECENT_MESSAGES: usize = 20;

/// The chat session that answers messages from one `WhatsApp` chat.
pub fn whatsapp_session_id(chat_jid: &str) -> String {
    format!("{WHATSAPP_SESSION_PREFIX}{chat_jid}")
}

/// True for sessions created by [`whatsapp_session_id`].
pub fn is_whatsapp_session(session_id: &str) -> bool {
    session_id.starts_with(WHATSAPP_SESSION_PREFIX)
}

/// Chat sessions that answer Telegram messages carry this prefix.
pub const TELEGRAM_SESSION_PREFIX: &str = "telegram:";

/// The chat session that answers messages from one Telegram chat.
pub fn telegram_session_id(chat_id: &str) -> String {
    format!("{TELEGRAM_SESSION_PREFIX}{chat_id}")
}

/// Which messaging service a session or message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    #[default]
    WhatsApp,
    Telegram,
}

impl ChannelKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WhatsApp => "WhatsApp",
            Self::Telegram => "Telegram",
        }
    }

    /// The chat session that answers messages from `chat` on this service.
    pub fn session_id(self, chat: &str) -> String {
        match self {
            Self::WhatsApp => whatsapp_session_id(chat),
            Self::Telegram => telegram_session_id(chat),
        }
    }
}

/// The service whose link a session answers, if it is one of those sessions. The runtime
/// keeps such sessions' events off the chat window.
pub fn channel_of_session(session_id: &str) -> Option<ChannelKind> {
    if is_whatsapp_session(session_id) {
        Some(ChannelKind::WhatsApp)
    } else if session_id.starts_with(TELEGRAM_SESSION_PREFIX) {
        Some(ChannelKind::Telegram)
    } else {
        None
    }
}

/// Six digits the owner sends to the Telegram bot to prove the bot is theirs. Derived from
/// the moment the link started; it only has to be unguessable for a few minutes.
pub fn telegram_link_code(seed_ms: i64) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in seed_ms.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:06}", hash % 1_000_000)
}

/// A pasted bot token must look like Telegram's `<bot id>:<secret>` before it is tried.
pub fn normalize_bot_token(input: &str) -> Result<String, String> {
    let token = input.trim().trim_start_matches("bot").to_owned();
    let valid = token.split_once(':').is_some_and(|(id, secret)| {
        !id.is_empty()
            && id.chars().all(|c| c.is_ascii_digit())
            && secret.len() >= 30
            && secret
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    });
    if valid {
        Ok(token)
    } else {
        Err("That doesn't look like a bot token. In Telegram, open @BotFather, send /newbot, and paste the token it gives you (digits, a colon, then letters).".into())
    }
}

/// Digits only, with the country code and no leading zeros, as `WhatsApp` expects for
/// phone-number linking. Rejects anything that cannot be a phone number.
pub fn normalize_phone_number(input: &str) -> Result<String, String> {
    let digits: String = input.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_start_matches('0').to_owned();
    if digits.len() < 7 || digits.len() > 15 {
        return Err(
            "Enter your phone number with the country code, for example +44 7700 900123.".into(),
        );
    }
    Ok(digits)
}

/// The `WhatsApp` account this computer is linked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedDevice {
    /// Human-readable phone number, such as `+44 7700 900123`.
    pub phone_display: String,
    /// This computer's `WhatsApp` address (`<number>:<device>@s.whatsapp.net`).
    pub device_jid: String,
    /// The name `WhatsApp` shows for this link under Linked Devices.
    pub companion_name: String,
    pub linked_at: Timestamp,
}

/// A QR code `WhatsApp` issued for this pairing attempt. The payload comes from `WhatsApp`'s
/// servers and is valid for a short time; the client asks for a new one when it expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsAppQrTicket {
    /// The text encoded in the QR code.
    pub qr_payload: String,
    /// Square module grid (`true` = dark). Empty while waiting for the first code.
    pub qr_matrix: Vec<Vec<bool>>,
    /// Seconds this code stays valid.
    pub expires_in_secs: u16,
    /// 8-character code to type on the phone instead of scanning, when requested.
    pub pairing_code: Option<String>,
}

impl WhatsAppQrTicket {
    /// A ticket with no code yet (the link is being set up).
    pub fn pending() -> Self {
        Self {
            qr_payload: String::new(),
            qr_matrix: Vec::new(),
            expires_in_secs: 0,
            pairing_code: None,
        }
    }

    pub fn has_qr(&self) -> bool {
        !self.qr_matrix.is_empty()
    }
}

/// What Sovra does with messages that arrive on the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsAppSettings {
    /// Answer messages automatically through the helpers.
    pub auto_reply_enabled: bool,
    /// Only answer in the "Message yourself" chat, never other people.
    pub only_self_chat: bool,
}

impl Default for WhatsAppSettings {
    fn default() -> Self {
        Self {
            auto_reply_enabled: true,
            only_self_chat: true,
        }
    }
}

/// Connection lifecycle state for `WhatsApp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum WhatsAppStatus {
    #[default]
    Disconnected,
    /// Opening the connection to `WhatsApp`. `pairing` is true for a new link (the QR
    /// window is open) and false when resuming a saved one in the background.
    Connecting { pairing: bool },
    /// `WhatsApp` issued a code; waiting for the phone to scan or type it.
    AwaitingQrScan { ticket: WhatsAppQrTicket },
    /// The phone accepted the link; keys are being exchanged.
    VerifyingHandshake { device_label: String },
    /// Linked. `online` is false while the connection is being re-established.
    Connected { device: LinkedDevice, online: bool },
    /// The last attempt failed; the message says what to do.
    Error { message: String },
}

impl WhatsAppStatus {
    /// Linked and online.
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { online: true, .. })
    }

    /// Linked, whether or not the connection is up right now.
    pub const fn is_linked(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    pub const fn is_pairing(&self) -> bool {
        matches!(
            self,
            Self::Connecting { pairing: true }
                | Self::AwaitingQrScan { .. }
                | Self::VerifyingHandshake { .. }
        )
    }

    pub fn linked_device(&self) -> Option<&LinkedDevice> {
        match self {
            Self::Connected { device, .. } => Some(device),
            Self::Disconnected
            | Self::Connecting { .. }
            | Self::AwaitingQrScan { .. }
            | Self::VerifyingHandshake { .. }
            | Self::Error { .. } => None,
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::Connecting { .. } => "Connecting…",
            Self::AwaitingQrScan { .. } => "Scan the code on your phone",
            Self::VerifyingHandshake { .. } => "Finishing the link…",
            Self::Connected { online: true, .. } => "Connected",
            Self::Connected { online: false, .. } => "Reconnecting…",
            Self::Error { .. } => "Needs attention",
        }
    }
}

/// What Sovra does with messages that arrive on the Telegram bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramSettings {
    /// Answer messages automatically through the helpers.
    pub auto_reply_enabled: bool,
    /// Only answer the person who linked the bot, never other people who find it.
    pub only_owner: bool,
}

impl Default for TelegramSettings {
    fn default() -> Self {
        Self {
            auto_reply_enabled: true,
            only_owner: true,
        }
    }
}

/// Connection lifecycle state for the Telegram bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum TelegramStatus {
    #[default]
    Disconnected,
    /// Checking the token with Telegram.
    Connecting,
    /// The bot is live; waiting for its owner to send `code` to it in Telegram.
    AwaitingOwner {
        bot_username: String,
        code: String,
    },
    /// Linked to its owner. `online` is false while the connection is being re-established.
    Connected {
        bot_username: String,
        owner_chat_id: String,
        owner_name: String,
        linked_at: Timestamp,
        online: bool,
    },
    Error {
        message: String,
    },
}

impl TelegramStatus {
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { online: true, .. })
    }

    /// Has a bot token that is worth resuming on the next start.
    pub const fn is_linked(&self) -> bool {
        matches!(self, Self::Connected { .. } | Self::AwaitingOwner { .. })
    }

    pub fn owner_chat_id(&self) -> Option<&str> {
        match self {
            Self::Connected { owner_chat_id, .. } => Some(owner_chat_id),
            Self::Disconnected
            | Self::Connecting
            | Self::AwaitingOwner { .. }
            | Self::Error { .. } => None,
        }
    }

    pub fn bot_username(&self) -> Option<&str> {
        match self {
            Self::AwaitingOwner { bot_username, .. } | Self::Connected { bot_username, .. } => {
                Some(bot_username)
            }
            Self::Disconnected | Self::Connecting | Self::Error { .. } => None,
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::Connecting => "Checking the token…",
            Self::AwaitingOwner { .. } => "Send the code to your bot",
            Self::Connected { online: true, .. } => "Connected",
            Self::Connected { online: false, .. } => "Reconnecting…",
            Self::Error { .. } => "Needs attention",
        }
    }
}

/// What to do with a message that reached the Telegram bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramAction {
    /// The sender typed the link code: they are the owner now.
    LinkOwner,
    /// Nobody owns the bot yet; tell the sender how to link it.
    AskForCode,
    /// Not for Sovra (group, empty, or a stranger while only the owner is answered).
    Ignore,
    /// Keep it, and answer it when `answer` is true.
    Record { answer: bool },
}

/// Decides what happens to a Telegram message given who owns the bot.
pub fn telegram_action(
    message: &IncomingMessage,
    status: &TelegramStatus,
    settings: &TelegramSettings,
) -> TelegramAction {
    let text = message.text.trim();
    if text.is_empty() || message.chat == ChatKind::Group {
        return TelegramAction::Ignore;
    }
    match status {
        TelegramStatus::AwaitingOwner { code, .. } => {
            if text == code {
                TelegramAction::LinkOwner
            } else {
                TelegramAction::AskForCode
            }
        }
        TelegramStatus::Connected { owner_chat_id, .. } => {
            let from_owner = *owner_chat_id == message.chat_jid;
            if !from_owner && settings.only_owner {
                TelegramAction::Ignore
            } else {
                TelegramAction::Record {
                    answer: settings.auto_reply_enabled && !message.backlog,
                }
            }
        }
        TelegramStatus::Disconnected
        | TelegramStatus::Connecting
        | TelegramStatus::Error { .. } => TelegramAction::Ignore,
    }
}

/// Private desktop tools require authenticated channel ownership, never a sender
/// name or message text. Transport adapters establish self-chat/from-me identity.
pub fn is_owner_message(
    channel: ChannelKind,
    message: &IncomingMessage,
    state: &ConnectionsState,
) -> bool {
    match channel {
        ChannelKind::WhatsApp => {
            state.whatsapp.is_connected() && message.is_self_chat() && message.from_me
        }
        ChannelKind::Telegram => {
            state.telegram.is_connected()
                && message.chat == ChatKind::Direct
                && state.telegram.owner_chat_id() == Some(message.chat_jid.as_str())
        }
    }
}

/// What the bot answers someone who writes before it is linked.
pub const TELEGRAM_ASK_FOR_CODE_REPLY: &str = "This bot belongs to a Sovra user. To link it, send the six-digit code shown in Sovra's Settings.";

/// What the bot answers its owner once the code matched.
pub const TELEGRAM_LINKED_REPLY: &str =
    "Linked. Messages you send here are answered by Sovra on your computer.";

/// A message that came in over a linked service and what Sovra answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgedMessage {
    pub id: String,
    #[serde(default)]
    pub channel: ChannelKind,
    pub chat_jid: String,
    pub sender_name: String,
    pub sender_jid: String,
    pub text: String,
    pub reply_text: Option<String>,
    pub received_at: Timestamp,
}

/// Complete persisted and live state for Connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectionsState {
    pub whatsapp: WhatsAppStatus,
    pub settings: WhatsAppSettings,
    #[serde(default)]
    pub telegram: TelegramStatus,
    #[serde(default)]
    pub telegram_settings: TelegramSettings,
    #[serde(default)]
    pub qr_modal_open: bool,
    #[serde(default)]
    pub recent_messages: Vec<BridgedMessage>,
    #[serde(default)]
    pub outbound_messages: Vec<crate::communications::OutboundMessage>,
    #[serde(default)]
    pub notice: Option<String>,
    /// People this computer can message. A missing field on older saves starts empty.
    #[serde(default)]
    pub people: crate::people::PeopleBook,
    /// Live probe results. Not saved: a restart reads each link again.
    #[serde(default, skip)]
    pub probed: std::collections::BTreeMap<
        crate::connection_catalogue::CatalogueKind,
        crate::connection_catalogue::LinkPhase,
    >,
}

/// Commands sent from the UI to `sovra-runtime` for personal connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionCommand {
    /// Ask for the current state (sent once the UI is ready).
    Load,
    /// Start a new link: open the QR window and ask `WhatsApp` for a code.
    StartWhatsAppPairing,
    /// Start over with a fresh code.
    RefreshWhatsAppQr,
    /// Link by typing a code on the phone instead of scanning.
    RequestPairingCode {
        phone_number: String,
    },
    /// Close the QR window and stop the attempt.
    CancelPairing,
    UpdateWhatsAppSettings {
        settings: WhatsAppSettings,
    },
    /// Unlink this computer from the phone and forget the saved link.
    DisconnectWhatsApp,
    /// Connect a Telegram bot with the token from `@BotFather`.
    ConnectTelegram {
        token: String,
    },
    UpdateTelegramSettings {
        settings: TelegramSettings,
    },
    /// Stop the bot and forget its token.
    DisconnectTelegram,
    /// Native review only; tools cannot issue this command.
    ReviewMessage {
        id: String,
        session_id: String,
        send: bool,
    },
    /// Probe installed links at the same time. A later refresh replaces the previous one.
    Refresh,
    /// Drop every probe from the current refresh. A finished link keeps its result.
    CancelRefresh,
    /// Prepare a review in the desktop chat. This does not send.
    PrepareMessage {
        session_id: String,
        channel: ChannelKind,
        recipient: String,
        text: String,
    },
    /// Add a phone, email, `WhatsApp` number, or known Telegram chat to one person.
    AttachHandle {
        person_id: String,
        reach: crate::people::Reach,
        value: String,
    },
    /// Join two saved people the person says are the same.
    LinkPeople {
        keep: String,
        merge: String,
    },
}

/// Events emitted by `sovra-runtime` back to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    Loaded(Box<ConnectionsState>),
    StatusChanged(WhatsAppStatus),
    SettingsUpdated(WhatsAppSettings),
    TelegramStatusChanged(TelegramStatus),
    TelegramSettingsUpdated(TelegramSettings),
    MessageBridged(BridgedMessage),
    OutboundChanged(crate::communications::OutboundMessage),
    /// Sovra's answer to the bridged message `message_id` was sent to the phone.
    ReplySent {
        message_id: String,
        reply: String,
    },
    /// A short message for the Connections page (confirmation or problem).
    Notice(String),
    /// One installed link finished its check. Other kinds are untouched.
    KindProbed(crate::connection_catalogue::ProbeReport),
    /// The people list changed. Other connection rows are untouched.
    PeopleChanged(crate::people::PeopleBook),
}

impl ConnectionsState {
    pub fn apply(&mut self, event: &ConnectionEvent) {
        match event {
            ConnectionEvent::Loaded(saved) => {
                *self = (**saved).clone();
            }
            ConnectionEvent::StatusChanged(status) => {
                self.probed
                    .remove(&crate::connection_catalogue::CatalogueKind::WhatsApp);
                self.qr_modal_open = match status {
                    WhatsAppStatus::Connecting { pairing: true }
                    | WhatsAppStatus::AwaitingQrScan { .. }
                    | WhatsAppStatus::VerifyingHandshake { .. } => true,
                    WhatsAppStatus::Connected { .. } | WhatsAppStatus::Disconnected => false,
                    WhatsAppStatus::Connecting { pairing: false }
                    | WhatsAppStatus::Error { .. } => self.qr_modal_open,
                };
                if status.is_pairing() {
                    self.notice = None;
                }
                self.whatsapp = status.clone();
            }
            ConnectionEvent::SettingsUpdated(settings) => {
                self.settings = *settings;
            }
            ConnectionEvent::TelegramStatusChanged(status) => {
                self.probed
                    .remove(&crate::connection_catalogue::CatalogueKind::Telegram);
                self.telegram = status.clone();
            }
            ConnectionEvent::TelegramSettingsUpdated(settings) => {
                self.telegram_settings = *settings;
            }
            ConnectionEvent::MessageBridged(message) => {
                self.recent_messages.insert(0, message.clone());
                self.recent_messages.truncate(MAX_RECENT_MESSAGES);
            }
            ConnectionEvent::ReplySent { message_id, reply } => {
                if let Some(message) = self
                    .recent_messages
                    .iter_mut()
                    .find(|message| message.id == *message_id)
                {
                    message.reply_text = Some(reply.clone());
                }
            }
            ConnectionEvent::OutboundChanged(message) => {
                if let Some(saved) = self
                    .outbound_messages
                    .iter_mut()
                    .find(|saved| saved.id == message.id)
                {
                    *saved = message.clone();
                } else {
                    self.outbound_messages.push(message.clone());
                }
                if self.outbound_messages.len() > crate::communications::MAX_OUTBOUND_MESSAGES
                    && let Some(index) = self.outbound_messages.iter().position(|message| {
                        !matches!(
                            message.delivery,
                            crate::communications::MessageDelivery::Review
                                | crate::communications::MessageDelivery::Sending
                        )
                    })
                {
                    self.outbound_messages.remove(index);
                }
            }
            ConnectionEvent::Notice(notice) => {
                self.notice = Some(notice.clone());
            }
            ConnectionEvent::KindProbed(report) => {
                self.probed.insert(report.kind, report.phase);
            }
            ConnectionEvent::PeopleChanged(people) => {
                self.people.clone_from(people);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Port: the messaging channel an adapter implements.
// ---------------------------------------------------------------------------

/// How to open the link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelStartOptions {
    /// Also ask for an 8-character code for this phone number (digits only).
    pub phone_number: Option<String>,
    /// A freshly pasted bot token (Telegram). `None` resumes the saved one.
    pub token: Option<String>,
}

/// What a polling session wants the runtime to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// Poll again straight away.
    Again,
    /// Wait this long before polling again (the service asked, or a request failed).
    Wait(std::time::Duration),
    /// The session is closed; stop polling.
    Finished,
}

/// A message received on the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub id: String,
    /// The chat it arrived in (a person, or your own "Message yourself" chat).
    pub chat_jid: String,
    pub sender_jid: String,
    pub sender_name: String,
    pub text: String,
    /// Sent from this account (from the phone, or mirrored from another device).
    pub from_me: bool,
    pub chat: ChatKind,
    /// Delivered from the queue of messages that arrived while the link was offline.
    pub backlog: bool,
}

/// What kind of chat a message arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    /// A one-to-one chat with another person.
    Direct,
    /// This account's own "Message yourself" chat.
    SelfChat,
    Group,
}

impl IncomingMessage {
    pub const fn is_self_chat(&self) -> bool {
        matches!(self.chat, ChatKind::SelfChat)
    }
}

/// What the channel adapter reports while a link is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEvent {
    Connecting,
    QrCode {
        payload: String,
        matrix: Vec<Vec<bool>>,
        expires_in_secs: u16,
    },
    PairingCode {
        code: String,
        expires_in_secs: u16,
    },
    PairingCodeFailed {
        message: String,
    },
    /// The phone accepted the code; the link is being finished.
    Scanned {
        device_label: String,
    },
    Connected {
        device: LinkedDevice,
    },
    /// The connection dropped; the adapter reconnects on its own.
    Offline {
        reason: String,
    },
    /// The phone removed this link (or `WhatsApp` ended it). The saved link is useless now.
    LoggedOut {
        reason: String,
    },
    /// The attempt is over and needs the person to act.
    Failed {
        message: String,
    },
    Message(IncomingMessage),
    /// Contacts the linked account synced. An empty list is not a wipe.
    Contacts(Vec<crate::people::SyncedContact>),
}

/// Receives [`ChannelEvent`]s; adapters call it from their own tasks.
pub type ChannelSink = Arc<dyn Fn(ChannelEvent) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelError {
    /// The adapter is not available in this build or configuration.
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
}

/// A personal messaging service the app can link (`WhatsApp` today).
#[async_trait]
pub trait MessagingChannel: Send + Sync {
    /// Opens a link, resuming saved credentials when there are any; otherwise `WhatsApp`
    /// issues QR codes through `events`.
    async fn start(
        &self,
        options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError>;

    /// Whether credentials from an earlier link are saved on this computer.
    fn has_saved_link(&self) -> bool;

    /// Deletes saved credentials so the next start pairs afresh.
    fn forget(&self) -> Result<(), ChannelError>;

    /// Confirms a saved link still answers.
    ///
    /// `Ok(true)` means the service answered. `Ok(false)` means this check did not
    /// contact the service, so it is not evidence the link works. The default does
    /// not call the network. Dropping the future cancels the check.
    async fn check(&self) -> Result<bool, ChannelError> {
        Ok(false)
    }

    /// Contacts already known to this link.
    ///
    /// The default does not call the network and does not invent people. A linked
    /// phone delivers its address book as later [`ChannelEvent::Contacts`]
    /// while it syncs. Dropping the future cancels the pull.
    async fn pull_contacts(&self) -> Result<Vec<crate::people::SyncedContact>, ChannelError> {
        Ok(Vec::new())
    }
}

/// One open link.
#[async_trait]
pub trait ChannelSession: Send + Sync {
    /// Sends plain text and returns the message id `WhatsApp` assigned.
    async fn send_text(&self, chat_jid: &str, text: &str) -> Result<String, ChannelError>;

    /// Removes this link from the phone and closes the connection.
    async fn logout(&self);

    /// Closes the connection, keeping saved credentials.
    async fn stop(&self);

    /// This account's own "Message yourself" chat, once linked.
    fn own_chat_jid(&self) -> Option<String>;

    /// Fetches whatever the service has queued (long polling) and reports events through
    /// the sink given at start. Sessions with their own connection return `Finished` at
    /// once; the runtime drives the loop and does the waiting, so adapters never need an
    /// executor of their own.
    async fn poll(&self) -> PollOutcome {
        PollOutcome::Finished
    }
}

// ---------------------------------------------------------------------------
// Policy: which messages Sovra records and answers.
// ---------------------------------------------------------------------------

/// True when a message is something a person wrote in a chat Sovra may take part in:
/// not a group, broadcast, channel or server notice, not empty, and not our own
/// outgoing message mirrored from the phone (unless it is the "Message yourself" chat,
/// where everything is from this account by definition).
pub fn should_record(message: &IncomingMessage) -> bool {
    if message.text.trim().is_empty() || message.chat == ChatKind::Group {
        return false;
    }
    let chat = message.chat_jid.as_str();
    if chat.ends_with("@broadcast") || chat.ends_with("@newsletter") || chat.starts_with("0@") {
        return false;
    }
    !message.from_me || message.is_self_chat()
}

/// True when Sovra should answer the message through the helpers. Messages that waited in
/// the offline queue are recorded but not answered: answering a day-old message surprises
/// people.
pub fn should_answer(message: &IncomingMessage, settings: &WhatsAppSettings) -> bool {
    should_record(message)
        && !message.backlog
        && settings.auto_reply_enabled
        && (message.is_self_chat() || !settings.only_self_chat)
}

/// What the phone shows when Sovra cannot answer because no model is connected yet.
pub const NO_MODEL_REPLY: &str =
    "Sovra isn't connected to an AI yet. Open Sovra on your computer and connect one in Models.";

/// What the phone shows when answering failed. Never includes the error: it may contain
/// addresses or settings that do not belong in a chat.
pub const ANSWER_FAILED_REPLY: &str =
    "Sorry, I couldn't answer that right now. Open Sovra on your computer to see what went wrong.";

#[cfg(test)]
mod tests {
    use super::*;

    fn message(chat: &str, from_me: bool, is_self_chat: bool) -> IncomingMessage {
        IncomingMessage {
            id: "m1".into(),
            chat_jid: chat.into(),
            sender_jid: "447700900123@s.whatsapp.net".into(),
            sender_name: "Sam".into(),
            text: "hello".into(),
            from_me,
            chat: if is_self_chat {
                ChatKind::SelfChat
            } else {
                ChatKind::Direct
            },
            backlog: false,
        }
    }

    #[test]
    fn session_ids_are_prefixed_and_recognised() {
        let id = whatsapp_session_id("447700900123@s.whatsapp.net");
        assert_eq!(id, "whatsapp:447700900123@s.whatsapp.net");
        assert!(is_whatsapp_session(&id));
        assert!(!is_whatsapp_session("chat-1"));
    }

    #[test]
    fn phone_numbers_become_digits_with_country_code() {
        assert_eq!(
            normalize_phone_number("+44 (0)7700 900123"),
            Ok("4407700900123".into())
        );
        assert_eq!(
            normalize_phone_number("1 555 234 5678"),
            Ok("15552345678".into())
        );
        assert!(normalize_phone_number("hello").is_err());
        assert!(normalize_phone_number("12345").is_err());
    }

    #[test]
    fn self_chat_and_other_people_are_recorded_but_echoes_and_groups_are_not() {
        assert!(should_record(&message(
            "447700900123@s.whatsapp.net",
            true,
            true
        )));
        assert!(should_record(&message(
            "447700900999@s.whatsapp.net",
            false,
            false
        )));
        // Our own message to someone else, mirrored from the phone.
        assert!(!should_record(&message(
            "447700900999@s.whatsapp.net",
            true,
            false
        )));
        let mut group = message("1234-5678@g.us", false, false);
        group.chat = ChatKind::Group;
        assert!(!should_record(&group));
        assert!(!should_record(&message("status@broadcast", false, false)));
        assert!(!should_record(&message("123@newsletter", false, false)));
        let mut empty = message("447700900999@s.whatsapp.net", false, false);
        empty.text = "  ".into();
        assert!(!should_record(&empty));
    }

    #[test]
    fn answering_follows_the_settings_and_skips_the_backlog() {
        let safe = WhatsAppSettings::default();
        assert!(should_answer(
            &message("447700900123@s.whatsapp.net", true, true),
            &safe
        ));
        assert!(!should_answer(
            &message("447700900999@s.whatsapp.net", false, false),
            &safe
        ));
        let everyone = WhatsAppSettings {
            only_self_chat: false,
            ..safe
        };
        assert!(should_answer(
            &message("447700900999@s.whatsapp.net", false, false),
            &everyone
        ));
        let off = WhatsAppSettings {
            auto_reply_enabled: false,
            ..safe
        };
        assert!(!should_answer(
            &message("447700900123@s.whatsapp.net", true, true),
            &off
        ));
        let mut old = message("447700900123@s.whatsapp.net", true, true);
        old.backlog = true;
        assert!(!should_answer(&old, &safe));
    }

    #[test]
    fn reducer_opens_the_qr_window_while_pairing_and_closes_it_when_linked() {
        let mut state = ConnectionsState::default();
        state.apply(&ConnectionEvent::StatusChanged(
            WhatsAppStatus::Connecting { pairing: true },
        ));
        assert!(state.qr_modal_open);
        state.apply(&ConnectionEvent::StatusChanged(
            WhatsAppStatus::AwaitingQrScan {
                ticket: WhatsAppQrTicket {
                    qr_payload: "2@abc".into(),
                    qr_matrix: vec![vec![true]],
                    expires_in_secs: 60,
                    pairing_code: None,
                },
            },
        ));
        assert!(state.qr_modal_open);
        assert!(state.whatsapp.is_pairing());
        state.apply(&ConnectionEvent::StatusChanged(WhatsAppStatus::Error {
            message: "The code expired.".into(),
        }));
        assert!(
            state.qr_modal_open,
            "an error keeps the window so people can retry"
        );
        let device = LinkedDevice {
            phone_display: "+44 7700 900123".into(),
            device_jid: "447700900123:7@s.whatsapp.net".into(),
            companion_name: "Sovra".into(),
            linked_at: Timestamp(1),
        };
        state.apply(&ConnectionEvent::StatusChanged(WhatsAppStatus::Connected {
            device,
            online: true,
        }));
        assert!(!state.qr_modal_open);
        assert!(state.whatsapp.is_connected());
        // Resuming in the background never pops the window.
        state.apply(&ConnectionEvent::StatusChanged(
            WhatsAppStatus::Connecting { pairing: false },
        ));
        assert!(!state.qr_modal_open);
    }

    #[test]
    fn messages_are_kept_newest_first_with_their_replies() {
        let mut state = ConnectionsState::default();
        for n in 0..(MAX_RECENT_MESSAGES + 3) {
            state.apply(&ConnectionEvent::MessageBridged(BridgedMessage {
                id: format!("m{n}"),
                channel: ChannelKind::WhatsApp,
                chat_jid: "447700900123@s.whatsapp.net".into(),
                sender_name: "You".into(),
                sender_jid: "447700900123@s.whatsapp.net".into(),
                text: format!("message {n}"),
                reply_text: None,
                received_at: Timestamp(i64::try_from(n).unwrap()),
            }));
        }
        assert_eq!(state.recent_messages.len(), MAX_RECENT_MESSAGES);
        assert_eq!(state.recent_messages[0].id, "m22");
        state.apply(&ConnectionEvent::ReplySent {
            message_id: "m22".into(),
            reply: "Here you go".into(),
        });
        assert_eq!(
            state.recent_messages[0].reply_text.as_deref(),
            Some("Here you go")
        );
        state.apply(&ConnectionEvent::Notice("Sent.".into()));
        assert_eq!(state.notice.as_deref(), Some("Sent."));
    }

    #[test]
    fn saved_state_round_trips_through_json() {
        let state = ConnectionsState {
            whatsapp: WhatsAppStatus::Connected {
                device: LinkedDevice {
                    phone_display: "+44 7700 900123".into(),
                    device_jid: "447700900123:7@s.whatsapp.net".into(),
                    companion_name: "Sovra".into(),
                    linked_at: Timestamp(5),
                },
                online: true,
            },
            settings: WhatsAppSettings {
                auto_reply_enabled: false,
                only_self_chat: true,
            },
            telegram: TelegramStatus::default(),
            telegram_settings: TelegramSettings::default(),
            qr_modal_open: false,
            recent_messages: Vec::new(),
            outbound_messages: Vec::new(),
            notice: None,
            people: crate::people::PeopleBook::default(),
            probed: std::collections::BTreeMap::new(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: ConnectionsState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn telegram_sessions_and_codes_are_recognised() {
        assert_eq!(
            channel_of_session(&telegram_session_id("12345")),
            Some(ChannelKind::Telegram)
        );
        assert_eq!(
            channel_of_session(&ChannelKind::WhatsApp.session_id("x@s.whatsapp.net")),
            Some(ChannelKind::WhatsApp)
        );
        assert_eq!(channel_of_session("chat-1"), None);
        let code = telegram_link_code(1_793_080_000_000);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(code, telegram_link_code(1_793_080_000_001));
    }

    #[test]
    fn bot_tokens_are_checked_before_they_are_tried() {
        let token = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw5";
        assert_eq!(normalize_bot_token(token), Ok(token.to_owned()));
        assert_eq!(
            normalize_bot_token(&format!("  bot{token} ")),
            Ok(token.to_owned())
        );
        assert!(normalize_bot_token("hello").is_err());
        assert!(normalize_bot_token("123:short").is_err());
    }

    #[test]
    fn telegram_messages_link_the_owner_then_follow_the_settings() {
        let settings = TelegramSettings::default();
        let waiting = TelegramStatus::AwaitingOwner {
            bot_username: "sovra_bot".into(),
            code: "482913".into(),
        };
        let mut from_owner = message("777", false, false);
        from_owner.text = "482913".into();
        assert_eq!(
            telegram_action(&from_owner, &waiting, &settings),
            TelegramAction::LinkOwner
        );
        from_owner.text = "hello?".into();
        assert_eq!(
            telegram_action(&from_owner, &waiting, &settings),
            TelegramAction::AskForCode
        );
        let linked = TelegramStatus::Connected {
            bot_username: "sovra_bot".into(),
            owner_chat_id: "777".into(),
            owner_name: "Sam".into(),
            linked_at: Timestamp(1),
            online: true,
        };
        assert_eq!(
            telegram_action(&from_owner, &linked, &settings),
            TelegramAction::Record { answer: true }
        );
        let stranger = message("999", false, false);
        assert_eq!(
            telegram_action(&stranger, &linked, &settings),
            TelegramAction::Ignore
        );
        let everyone = TelegramSettings {
            only_owner: false,
            ..settings
        };
        assert_eq!(
            telegram_action(&stranger, &linked, &everyone),
            TelegramAction::Record { answer: true }
        );
        let quiet = TelegramSettings {
            auto_reply_enabled: false,
            ..settings
        };
        assert_eq!(
            telegram_action(&from_owner, &linked, &quiet),
            TelegramAction::Record { answer: false }
        );
        let mut group = message("-100123", false, false);
        group.chat = ChatKind::Group;
        assert_eq!(
            telegram_action(&group, &linked, &everyone),
            TelegramAction::Ignore
        );
        assert_eq!(
            telegram_action(&from_owner, &TelegramStatus::Disconnected, &settings),
            TelegramAction::Ignore
        );
    }
}
