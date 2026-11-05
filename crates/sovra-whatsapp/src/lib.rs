//! `WhatsApp` adapter: implements `sovra_core::connections::MessagingChannel` with the
//! open-source `whatsapp-rust` client (`WhatsApp` Web multi-device protocol: Noise
//! handshake, Signal end-to-end encryption, QR and phone-number linking), so a personal
//! `WhatsApp` account links to this app with no sidecar, server or business account.
//!
//! This is the only crate that names `whatsapp-rust` and `qrcode`. Credentials and
//! Signal state live in one SQLite file under the app data directory. Policy (which
//! messages to answer) stays in `sovra-core`; this crate only reports what happens and
//! sends what it is told to.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
mod map;

use sovra_core::Timestamp;
use sovra_core::connections::{
    AddressBookSnapshot, ChannelError, ChannelEvent, ChannelSession, ChannelSink,
    ChannelStartOptions, ChatKind, IncomingMessage, LinkedDevice, MessagingChannel,
};
use whatsapp_rust::Jid;
use whatsapp_rust::bot::{Bot, BotHandle, EventDelivery};
use whatsapp_rust::client::Client;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::proto_helpers::MessageExt as _;
use whatsapp_rust::store::SqliteStore;
use whatsapp_rust::types::events::Event;
use whatsapp_rust::types::events::LazyHistorySync;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::waproto::whatsapp::device_props::PlatformType;

/// File that holds the link's credentials and encryption state.
const DB_FILE: &str = "session.sqlite";
/// SQLite side files that must go when the database goes.
const DB_SIDE_FILES: [&str; 3] = [
    "session.sqlite-wal",
    "session.sqlite-shm",
    "session.sqlite-journal",
];
/// How this link appears under Linked Devices on the phone.
///
/// `WhatsApp` shows the `DeviceProps` `os` field when `platform_type` is
/// `DESKTOP`. The library default is `UNKNOWN` / `"rust"`, which the phone
/// labels "Unknown device". Pair-code hello still sends a real OS name on the
/// wire because the server rejects branding there; the Linked Devices label is
/// this string.
const COMPANION_NAME: &str = "Sovra connection";
/// Ordered delivery keeps a conversation's messages in the order the phone sent them.
const EVENT_MAILBOX: usize = 1024;

/// A personal `WhatsApp` account linked through this app.
#[derive(Clone)]
pub struct WhatsAppChannel {
    dir: PathBuf,
    live: Arc<Mutex<Option<Arc<Client>>>>,
}

impl std::fmt::Debug for WhatsAppChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsAppChannel")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl WhatsAppChannel {
    /// `dir` is a private, app-owned directory (for example `<data>/whatsapp`).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            live: Arc::new(Mutex::new(None)),
        }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }

    fn remember_client(&self, client: Arc<Client>) {
        *self
            .live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(client);
    }

    fn forget_client(&self) {
        *self
            .live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    fn live_client(&self) -> Option<Arc<Client>> {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl MessagingChannel for WhatsAppChannel {
    async fn start(
        &self,
        options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        std::fs::create_dir_all(&self.dir).map_err(|error| {
            ChannelError::Failed(format!(
                "Could not create the WhatsApp folder {}: {error}",
                self.dir.display()
            ))
        })?;
        let db_path = self.db_path();
        let db_url = db_path.to_string_lossy().into_owned();
        let store = SqliteStore::new(&db_url).await.map_err(|error| {
            ChannelError::Failed(format!("Could not open the WhatsApp store: {error}"))
        })?;

        let sink = events.clone();
        let mut builder = Bot::builder()
            .with_backend(store)
            .with_device_props(companion_device_props())
            .with_event_delivery(EventDelivery::Ordered {
                capacity: EVENT_MAILBOX,
            })
            .on_event(move |event, client| {
                let sink = sink.clone();
                async move {
                    for mapped in map_event(&event, &client) {
                        sink(mapped);
                    }
                }
            });
        if let Some(phone_number) = options.phone_number {
            builder = builder.with_pair_code(PairCodeOptions {
                phone_number,
                ..PairCodeOptions::default()
            });
        }
        let bot = builder
            .build()
            .await
            .map_err(|error| ChannelError::Failed(format!("Could not start WhatsApp: {error}")))?;
        let client = bot.client();
        self.remember_client(client.clone());
        let handle = bot.spawn();
        tracing::info!(path = %db_path.display(), "whatsapp link started");
        events(ChannelEvent::Connecting);
        Ok(Arc::new(Session {
            client,
            handle: Mutex::new(Some(handle)),
        }))
    }

    fn has_saved_link(&self) -> bool {
        self.db_path().is_file()
    }

    async fn pull_contacts(&self) -> Result<AddressBookSnapshot, ChannelError> {
        // Contacts arrive as app-state and history metadata events. Groups are
        // queried from the live session so they are known without waiting for a
        // group message. An empty snapshot must not invent people or groups.
        let Some(client) = self.live_client() else {
            return Ok(AddressBookSnapshot::default());
        };
        let groups = match client.groups().get_participating().await {
            Ok(found) => found
                .into_iter()
                .filter_map(|(jid, meta)| map::map_group(&jid.to_non_ad_string(), &meta.subject))
                .collect(),
            Err(error) => {
                return Err(ChannelError::Failed(format!(
                    "WhatsApp could not list groups: {error}"
                )));
            }
        };
        Ok(AddressBookSnapshot {
            contacts: Vec::new(),
            groups,
        })
    }

    fn forget(&self) -> Result<(), ChannelError> {
        self.forget_client();
        remove_if_present(&self.db_path())?;
        for name in DB_SIDE_FILES {
            remove_if_present(&self.dir.join(name))?;
        }
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> Result<(), ChannelError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ChannelError::Failed(format!(
            "Could not remove {}: {error}",
            path.display()
        ))),
    }
}

/// One open link. Dropping it without `stop` leaves the client running until the
/// executor shuts down, so the runtime always stops sessions it replaces.
struct Session {
    client: Arc<Client>,
    handle: Mutex<Option<BotHandle>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("connected", &self.client.is_connected())
            .finish_non_exhaustive()
    }
}

impl Session {
    fn take_handle(&self) -> Option<BotHandle> {
        self.handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

#[async_trait]
impl ChannelSession for Session {
    async fn send_text(&self, chat_jid: &str, text: &str) -> Result<String, ChannelError> {
        let to: Jid = chat_jid
            .parse()
            .map_err(|error| ChannelError::Failed(format!("Bad WhatsApp address: {error}")))?;
        let sent = self.client.send_text(to, text).await.map_err(|error| {
            ChannelError::Failed(format!("WhatsApp did not accept the message: {error}"))
        })?;
        Ok(sent.message_id)
    }

    async fn logout(&self) {
        self.client.logout().await;
        if let Some(handle) = self.take_handle() {
            handle.shutdown().await;
        }
    }

    async fn stop(&self) {
        if let Some(handle) = self.take_handle() {
            handle.shutdown().await;
        } else {
            self.client.disconnect().await;
        }
    }

    fn own_chat_jid(&self) -> Option<String> {
        self.client.pn().map(|jid| jid.to_non_ad_string())
    }
}

/// Turns the client's events into channel events. Many client events are irrelevant
/// here (receipts, presence, calls); they map to nothing.
fn map_event(event: &Event, client: &Client) -> Vec<ChannelEvent> {
    match event {
        Event::PairingQrCode(qr) => vec![ChannelEvent::QrCode {
            payload: qr.code.clone(),
            matrix: qr_matrix(&qr.code),
            expires_in_secs: secs(qr.timeout),
        }],
        Event::PairingCode(code) => vec![ChannelEvent::PairingCode {
            code: code.code.clone(),
            expires_in_secs: secs(code.timeout),
        }],
        Event::PairingCodeError(error) => vec![ChannelEvent::PairingCodeFailed {
            message: match &error.rejection {
                Some(rejection) => format!(
                    "WhatsApp did not accept the phone-number code ({rejection:?}). Scan the QR code instead, or try again in a few minutes."
                ),
                None => "WhatsApp did not answer the phone-number request. Scan the QR code instead.".into(),
            },
        }],
        Event::PairingQrCodesExhausted(_) => vec![ChannelEvent::Failed {
            message: "The QR code expired before it was scanned. Click Connect WhatsApp to get a new one.".into(),
        }],
        Event::QrScannedWithoutMultidevice(_) => vec![ChannelEvent::Failed {
            message: "Your phone's WhatsApp needs the multi-device feature. Update WhatsApp on the phone and try again.".into(),
        }],
        Event::PairSuccess(pair) => vec![ChannelEvent::Scanned {
            device_label: if pair.platform.is_empty() {
                "your phone".to_owned()
            } else {
                pair.platform.clone()
            },
        }],
        Event::PairError(error) => vec![ChannelEvent::Failed {
            message: format!("The phone did not finish linking: {}", error.error),
        }],
        Event::Connected(_) => vec![ChannelEvent::Connected {
            device: linked_device(client),
        }],
        Event::Disconnected(gone) => vec![ChannelEvent::Offline {
            reason: gone.reason.to_string(),
        }],
        Event::LoggedOut(out) => vec![ChannelEvent::LoggedOut {
            reason: out
                .logout_message
                .as_ref()
                .and_then(|message| message.header.clone())
                .unwrap_or_else(|| format!("{:?}", out.reason)),
        }],
        Event::StreamReplaced(_) => vec![ChannelEvent::Failed {
            message: "WhatsApp was linked from another computer, so this link closed. Click Connect WhatsApp to link again.".into(),
        }],
        Event::ClientOutdated(_) => vec![ChannelEvent::Failed {
            message: "WhatsApp no longer accepts this version of the link. Update Sovra and try again.".into(),
        }],
        Event::TemporaryBan(ban) => vec![ChannelEvent::Failed {
            message: format!(
                "WhatsApp paused this account for {} minutes ({:?}).",
                ban.expire.num_minutes(),
                ban.code
            ),
        }],
        Event::ConnectFailure(failure) => vec![ChannelEvent::Failed {
            message: match &failure.message {
                Some(text) if !text.is_empty() => format!("WhatsApp refused the connection: {text}"),
                _ => format!("WhatsApp refused the connection ({:?}).", failure.reason),
            },
        }],
        Event::Messages(batch) => {
            let own = own_users(client);
            batch
                .messages
                .iter()
                .filter_map(|inbound| {
                    let text = inbound.message.get_base_message().text_content()?;
                    let info = &inbound.info;
                    let chat = info.source.chat.to_non_ad_string();
                    let kind = if info.source.is_group {
                        ChatKind::Group
                    } else if own
                        .iter()
                        .any(|user| *user == info.source.chat.user.as_str())
                    {
                        ChatKind::SelfChat
                    } else {
                        ChatKind::Direct
                    };
                    Some(ChannelEvent::Message(IncomingMessage {
                        id: info.id.clone(),
                        chat_jid: chat,
                        sender_jid: info.source.sender.to_non_ad_string(),
                        sender_name: info.push_name.clone(),
                        text: text.to_owned(),
                        from_me: info.source.is_from_me,
                        chat: kind,
                        backlog: info.is_offline,
                    }))
                })
                .collect()
        }
        Event::ContactUpdate(update) => contacts_from_update(update),
        Event::HistorySync(history) => events_from_history(history),
        _ => Vec::new(),
    }
}

/// A phone-number contact from app-state. LID rows that carry a phone number
/// are kept. Groups and other servers are skipped.
fn contacts_from_update(update: &whatsapp_rust::types::events::ContactUpdate) -> Vec<ChannelEvent> {
    let action = update.action.as_ref();
    map::map_contact_row(map::ContactRow {
        jid: &update.jid.to_non_ad_string(),
        pn_jid: action.pn_jid.as_deref(),
        full_name: action.full_name.as_deref(),
        first_name: action.first_name.as_deref(),
        save_on_primary_addressbook: action.save_on_primary_addressbook,
    })
    .map(|contact| vec![ChannelEvent::Contacts(vec![contact])])
    .unwrap_or_default()
}

/// Conversation metadata from history bootstrap. Message bodies are not stored.
fn events_from_history(history: &LazyHistorySync) -> Vec<ChannelEvent> {
    let mut contacts = Vec::new();
    let mut groups = Vec::new();
    let mut stream = history.stream();
    let mut conversation = whatsapp_rust::waproto::whatsapp::Conversation::default();
    while let Ok(true) = stream.next_conversation_into(&mut conversation) {
        match map::map_history_chat(map::HistoryChat {
            id: &conversation.id,
            name: conversation.name.as_deref(),
            display_name: conversation.display_name.as_deref(),
            pn_jid: conversation.pn_jid.as_deref(),
        }) {
            map::MappedChat::Contact(contact) => contacts.push(contact),
            map::MappedChat::Group(group) => groups.push(group),
            map::MappedChat::Skip => {}
        }
    }
    let mut events = Vec::new();
    if !contacts.is_empty() {
        events.push(ChannelEvent::Contacts(contacts));
    }
    if !groups.is_empty() {
        events.push(ChannelEvent::Groups(groups));
    }
    events
}

/// The user parts of this account's phone-number and LID addresses. Chats with either
/// are the "Message yourself" chat.
fn own_users(client: &Client) -> Vec<String> {
    client
        .pn()
        .into_iter()
        .chain(client.lid())
        .map(|jid| jid.user.to_string())
        .collect()
}

/// Identity sent during pairing so Linked Devices names this computer.
fn companion_device_props() -> DevicePropsOverride {
    DevicePropsOverride::new()
        .with_os(COMPANION_NAME)
        .with_platform_type(PlatformType::DESKTOP)
}

fn linked_device(client: &Client) -> LinkedDevice {
    let pn = client.pn();
    LinkedDevice {
        phone_display: pn.as_ref().map_or_else(
            || "your WhatsApp".to_owned(),
            |jid| format!("+{}", jid.user),
        ),
        device_jid: pn.map_or_else(String::new, |jid| jid.to_string()),
        companion_name: COMPANION_NAME.to_owned(),
        linked_at: Timestamp(0),
    }
}

fn secs(duration: std::time::Duration) -> u16 {
    u16::try_from(duration.as_secs()).unwrap_or(u16::MAX)
}

/// The QR module grid for a pairing payload. An encoding failure (the payload is far
/// below the QR size limit) yields an empty grid, which the UI shows as "no code yet".
pub fn qr_matrix(payload: &str) -> Vec<Vec<bool>> {
    let Ok(code) = qrcode::QrCode::new(payload.as_bytes()) else {
        tracing::warn!("pairing payload could not be encoded as a QR code");
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|module| *module == qrcode::Color::Dark)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phone_lists_this_link_as_sovra_connection() {
        let props = companion_device_props();
        assert_eq!(props.os.as_deref(), Some(COMPANION_NAME));
        assert_eq!(props.platform_type, Some(PlatformType::DESKTOP));
        let src = include_str!("lib.rs");
        assert!(
            src.contains("with_device_props(companion_device_props())"),
            "Linked Devices naming must be sent on the live Bot builder"
        );
    }

    #[test]
    fn qr_grids_are_square_and_carry_finder_patterns() {
        let payload = format!(
            "2@{},{},{},{},7",
            "A".repeat(32),
            "B".repeat(44),
            "C".repeat(44),
            "D".repeat(44)
        );
        let grid = qr_matrix(&payload);
        assert!(
            grid.len() > 20,
            "a pairing payload needs a sizeable QR version"
        );
        assert!(grid.iter().all(|row| row.len() == grid.len()));
        // Every QR code starts with a 7×7 finder pattern whose corner is dark.
        assert!(grid[0][0] && grid[0][6] && grid[6][0]);
    }

    #[test]
    fn forgetting_removes_the_store_and_its_side_files() {
        let dir = std::env::temp_dir().join(format!("sovra-wa-{}", std::process::id()));
        let channel = WhatsAppChannel::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(channel.db_path(), b"x").unwrap();
        std::fs::write(dir.join(DB_SIDE_FILES[0]), b"x").unwrap();
        assert!(channel.has_saved_link());
        channel.forget().unwrap();
        assert!(!channel.has_saved_link());
        assert!(!dir.join(DB_SIDE_FILES[0]).exists());
        channel.forget().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
