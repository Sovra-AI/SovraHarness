//! `WhatsApp` adapter: implements `sovra_core::connections::MessagingChannel` with the
//! open-source `whatsapp-rust` client (`WhatsApp` Web multi-device protocol: Noise
//! handshake, Signal end-to-end encryption, QR and phone-number linking), so a personal
//! `WhatsApp` account links to this app with no sidecar, server or business account.
//!
//! This is the only crate that names `whatsapp-rust` and `qrcode`. Credentials and
//! Signal state live in one SQLite file under the app data directory. Policy (which
//! messages to answer) stays in `sovra-core`; this crate only reports what happens and
//! sends what it is told to.
//!
//! `process_sync_task` is one large generated future. Awaiting it from this
//! crate's connect handler nests that layout past rustc's default query depth.
#![recursion_limit = "256"]

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
use whatsapp_rust::store::{AppSyncStore, SqliteStore};
use whatsapp_rust::sync_task::MajorSyncTask;
use whatsapp_rust::types::events::Event;
use whatsapp_rust::types::events::LazyHistorySync;
use whatsapp_rust::wacore::appstate::hash::HashState;
use whatsapp_rust::wacore::appstate::patch_decode::WAPatchName;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore::store::device::default_history_sync_config;
use whatsapp_rust::waproto::whatsapp as wa;
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
/// Days of chat history a full companion backfill asks the phone to share.
const FULL_SYNC_DAYS: u32 = 365;

/// A personal `WhatsApp` account linked through this app.
#[derive(Clone)]
pub struct WhatsAppChannel {
    dir: PathBuf,
    live: Arc<Mutex<Option<Arc<Client>>>>,
    /// Contacts and groups already seen on this link. The online pull returns
    /// this book, including rows that arrive while the group query is running.
    book: Arc<Mutex<map::LearnedBook>>,
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
            book: Arc::new(Mutex::new(map::LearnedBook::default())),
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
        let book = Arc::clone(&self.book);
        // Concurrent delivery does not drop a large address book. Contact rows
        // are independent, and an empty later name does not clear a saved one.
        let mut builder = Bot::builder()
            .with_backend(store)
            .with_device_props(companion_device_props())
            .with_event_delivery(EventDelivery::Concurrent)
            .on_event(move |event, client| {
                let sink = sink.clone();
                let book = Arc::clone(&book);
                async move {
                    let connected = matches!(event.as_ref(), Event::Connected(_));
                    for mapped in map_event(event.as_ref(), client.as_ref(), &book) {
                        sink(mapped);
                    }
                    // The link is reported online before the replay, so a slow
                    // collection fetch cannot hide that the phone connected.
                    // The book lock is not held across this await.
                    if connected {
                        replay_contact_and_chat_collections(&client).await;
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
            Ok(found) => Ok(found
                .into_iter()
                .filter_map(|(jid, meta)| map::map_group(&jid.to_non_ad_string(), &meta.subject))
                .collect()),
            Err(error) => {
                tracing::warn!(%error, "whatsapp group list failed; keeping contacts already learned");
                Err(())
            }
        };
        // Read after the await so rows that arrived while groups were loading
        // are part of this snapshot. A group-list failure still returns them.
        let contacts = self
            .book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot_contacts();
        Ok(map::snapshot_keeping_contacts(contacts, groups))
    }

    fn forget(&self) -> Result<(), ChannelError> {
        self.forget_client();
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
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
fn map_event(event: &Event, client: &Client, book: &Mutex<map::LearnedBook>) -> Vec<ChannelEvent> {
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
        Event::ContactUpdate(update) => contacts_from_update(update, book),
        Event::HistorySync(history) => events_from_history(history, book),
        Event::PinUpdate(update) => named_chat(book, &update.jid),
        Event::MuteUpdate(update) => named_chat(book, &update.jid),
        Event::ArchiveUpdate(update) => named_chat(book, &update.jid),
        Event::MarkChatAsReadUpdate(update) => named_chat(book, &update.jid),
        Event::DeleteChatUpdate(update) => deleted_chat(book, &update.jid),
        // Clearing a chat keeps the person. Muting someone's status is not a
        // chat to store. The wildcard also covers receipts, presence and calls.
        _ => Vec::new(),
    }
}

fn publish(book: &Mutex<map::LearnedBook>, batch: map::SyncBatch<'_>) -> Vec<ChannelEvent> {
    book.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish(map::map_sync(batch))
}

fn named_chat(book: &Mutex<map::LearnedBook>, jid: &Jid) -> Vec<ChannelEvent> {
    let id = jid.to_non_ad_string();
    publish(
        book,
        map::SyncBatch {
            named_chat_ids: &[&id],
            ..map::SyncBatch::empty()
        },
    )
}

fn deleted_chat(book: &Mutex<map::LearnedBook>, jid: &Jid) -> Vec<ChannelEvent> {
    let id = jid.to_non_ad_string();
    publish(
        book,
        map::SyncBatch {
            deleted_chat_ids: &[&id],
            ..map::SyncBatch::empty()
        },
    )
}

/// A phone-number contact from app-state. The library only emits a set, so this
/// row is not a removal. "Not saved on the phone" still keeps the chat.
fn contacts_from_update(
    update: &whatsapp_rust::types::events::ContactUpdate,
    book: &Mutex<map::LearnedBook>,
) -> Vec<ChannelEvent> {
    let jid = update.jid.to_non_ad_string();
    let action = update.action.as_ref();
    let row = map::ContactRow {
        jid: &jid,
        pn_jid: action.pn_jid.as_deref(),
        full_name: action.full_name.as_deref(),
        first_name: action.first_name.as_deref(),
        save_on_primary_addressbook: action.save_on_primary_addressbook,
        removed: false,
    };
    publish(
        book,
        map::SyncBatch {
            contacts: &[row],
            ..map::SyncBatch::empty()
        },
    )
}

/// Conversation metadata from one history chunk. Every sync type is mapped the
/// same way. Message bodies are not stored.
fn events_from_history(
    history: &LazyHistorySync,
    book: &Mutex<map::LearnedBook>,
) -> Vec<ChannelEvent> {
    let collected = collect_history(history);
    let chats: Vec<map::HistoryChat<'_>> = collected
        .chats
        .iter()
        .map(|chat| map::HistoryChat {
            id: &chat.id,
            name: chat.name.as_deref(),
            display_name: chat.display_name.as_deref(),
            pn_jid: chat.pn_jid.as_deref(),
        })
        .collect();
    let push_names: Vec<(&str, &str)> = collected
        .push_names
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect();
    let phone_by_hidden_id: Vec<(&str, &str)> = collected
        .phone_map
        .iter()
        .map(|(hidden, phone)| (hidden.as_str(), phone.as_str()))
        .collect();
    let contacts: Vec<map::ContactRow<'_>> = collected
        .inline
        .iter()
        .map(|row| map::ContactRow {
            jid: &row.jid,
            pn_jid: row.pn_jid.as_deref(),
            full_name: row.full_name.as_deref(),
            first_name: row.first_name.as_deref(),
            save_on_primary_addressbook: None,
            removed: false,
        })
        .collect();
    publish(
        book,
        map::SyncBatch {
            contacts: &contacts,
            chats: &chats,
            push_names: &push_names,
            phone_by_hidden_id: &phone_by_hidden_id,
            ..map::SyncBatch::empty()
        },
    )
}

struct OwnedChat {
    id: String,
    name: Option<String>,
    display_name: Option<String>,
    pn_jid: Option<String>,
}

struct OwnedInline {
    jid: String,
    pn_jid: Option<String>,
    full_name: Option<String>,
    first_name: Option<String>,
}

#[derive(Default)]
struct CollectedHistory {
    chats: Vec<OwnedChat>,
    push_names: Vec<(String, String)>,
    phone_map: Vec<(String, String)>,
    inline: Vec<OwnedInline>,
}

fn collect_history(history: &LazyHistorySync) -> CollectedHistory {
    let mut stream = history.stream();
    let mut conversation = wa::Conversation::default();
    let mut chats = Vec::new();
    loop {
        match stream.next_conversation_into(&mut conversation) {
            Ok(true) => chats.push(owned_chat(&conversation)),
            Ok(false) => break,
            Err(error) => {
                tracing::warn!(%error, "whatsapp history chunk could not be streamed");
                return collect_decoded(history.get());
            }
        }
    }
    match stream.remainder() {
        Ok(rest) => {
            let mut collected = fields_from_history(&rest);
            collected.chats = chats;
            collected
        }
        Err(error) => {
            tracing::warn!(%error, "whatsapp history chunk remainder could not be read");
            let mut decoded = collect_decoded(history.get());
            if decoded.chats.is_empty() {
                decoded.chats = chats;
            }
            decoded
        }
    }
}

fn collect_decoded(history: Option<&wa::HistorySync>) -> CollectedHistory {
    history.map_or(CollectedHistory::default(), fields_from_history)
}

fn fields_from_history(history: &wa::HistorySync) -> CollectedHistory {
    CollectedHistory {
        chats: history.conversations.iter().map(owned_chat).collect(),
        push_names: history
            .pushnames
            .iter()
            .filter_map(|push| {
                let id = push.id.as_deref().filter(|id| !id.is_empty())?;
                let name = push.pushname.as_deref().filter(|name| !name.is_empty())?;
                Some((id.to_owned(), name.to_owned()))
            })
            .collect(),
        phone_map: history
            .phone_number_to_lid_mappings
            .iter()
            .filter_map(|pair| {
                Some((
                    pair.lid_jid
                        .as_deref()
                        .filter(|id| !id.is_empty())?
                        .to_owned(),
                    pair.pn_jid
                        .as_deref()
                        .filter(|id| !id.is_empty())?
                        .to_owned(),
                ))
            })
            .collect(),
        inline: history
            .inline_contacts
            .iter()
            .filter_map(|contact| {
                let jid = contact
                    .lid_jid
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .or_else(|| contact.pn_jid.as_deref().filter(|id| !id.is_empty()))?;
                Some(OwnedInline {
                    jid: jid.to_owned(),
                    pn_jid: contact.pn_jid.clone(),
                    full_name: contact.full_name.clone(),
                    first_name: contact.first_name.clone(),
                })
            })
            .collect(),
    }
}

fn owned_chat(conversation: &wa::Conversation) -> OwnedChat {
    OwnedChat {
        id: conversation.id.clone(),
        name: conversation.name.clone(),
        display_name: conversation.display_name.clone(),
        pn_jid: conversation.pn_jid.clone(),
    }
}

/// App-state collections that name contacts and chats. Asking again with a
/// snapshot does not delete the session file. Full history is a pairing-time
/// choice and is not replayed here.
fn contact_and_chat_collections() -> [WAPatchName; 3] {
    [
        WAPatchName::CriticalUnblockLow,
        WAPatchName::RegularLow,
        WAPatchName::RegularHigh,
    ]
}

/// `whatsapp-rust` 0.7.0 drops a snapshot when the saved version is already at
/// least the snapshot's (`persisted > 0 && snapshot <= persisted`), even if
/// the request asked for one. Version 0 and an empty ltHash are the library's
/// fresh baseline, so the same snapshot is applied and its rows are dispatched.
/// Old mutation MACs are cleared with that baseline; leftover ones would
/// disagree with the next patch. The session file is not deleted.
async fn fresh_contact_and_chat_baseline(backend: &dyn AppSyncStore) {
    for name in contact_and_chat_collections() {
        let collection = name.as_str();
        // Lower the version first. Clearing MACs while the version is still
        // current would make the next incremental patch fail its check.
        if let Err(error) = backend.set_version(collection, HashState::default()).await {
            tracing::warn!(
                collection,
                %error,
                "could not lower the WhatsApp collection before asking for its snapshot"
            );
            continue;
        }
        if let Err(error) = backend.clear_mutation_macs(collection).await {
            tracing::warn!(
                collection,
                %error,
                "could not clear old WhatsApp chat rows before the snapshot"
            );
        }
    }
}

async fn replay_contact_and_chat_collections(client: &Arc<Client>) {
    let backend = client.persistence_manager().backend();
    let sync: &dyn AppSyncStore = backend.as_ref();
    fresh_contact_and_chat_baseline(sync).await;
    for name in contact_and_chat_collections() {
        client
            .process_sync_task(MajorSyncTask::AppStateSync {
                name,
                full_sync: true,
            })
            .await;
    }
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

/// Identity sent during pairing so Linked Devices names this computer, and so
/// the phone is asked for the full companion backfill rather than recent chats
/// only. The visible name stays a desktop label.
fn companion_device_props() -> DevicePropsOverride {
    DevicePropsOverride::new()
        .with_os(COMPANION_NAME)
        .with_platform_type(PlatformType::DESKTOP)
        .with_require_full_sync(true)
        .with_history_sync_config(wa::device_props::HistorySyncConfig {
            full_sync_days_limit: Some(FULL_SYNC_DAYS),
            on_demand_ready: Some(true),
            complete_on_demand_ready: Some(true),
            ..default_history_sync_config()
        })
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
    fn the_phone_lists_this_link_as_sovra_connection_and_is_asked_for_full_history() {
        let props = companion_device_props();
        assert_eq!(props.os.as_deref(), Some(COMPANION_NAME));
        assert_eq!(props.platform_type, Some(PlatformType::DESKTOP));
        assert_eq!(props.require_full_sync, Some(true));
        let config = props
            .history_sync_config
            .as_ref()
            .expect("full sync asks for a history config");
        assert_eq!(config.full_sync_days_limit, Some(FULL_SYNC_DAYS));
        assert_eq!(config.on_demand_ready, Some(true));
        assert_eq!(config.complete_on_demand_ready, Some(true));
        let src = link_source();
        assert!(
            src.contains("with_device_props(companion_device_props())"),
            "Linked Devices naming must be sent on the live Bot builder"
        );
        assert!(
            src.contains("EventDelivery::Concurrent"),
            "a large address book must not be dropped by a bounded mailbox"
        );
        assert!(
            !src.contains("skip_history_sync"),
            "ack-and-discard of history drops contacts and groups"
        );
        assert!(
            !src.contains("EventDelivery::Ordered"),
            "the ordered mailbox drops events once it is full"
        );
    }

    /// Production adapter only. Assertions live below `#[cfg(test)]` and must
    /// not count as the live builder.
    fn link_source() -> &'static str {
        include_str!("lib.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source")
    }

    #[test]
    fn a_history_chunk_keeps_the_hidden_id_the_push_name_and_the_inline_contact() {
        let history = wa::HistorySync {
            sync_type: wa::history_sync::HistorySyncType::INITIAL_BOOTSTRAP,
            conversations: vec![wa::Conversation {
                id: "123456789012345@lid".into(),
                name: Some("Ada Lovelace".into()),
                ..Default::default()
            }],
            pushnames: vec![wa::Pushname {
                id: Some("447700900666@s.whatsapp.net".into()),
                pushname: Some("Pat Lee".into()),
            }],
            phone_number_to_lid_mappings: vec![wa::PhoneNumberToLIDMapping {
                pn_jid: Some("447700900222@s.whatsapp.net".into()),
                lid_jid: Some("123456789012345@lid".into()),
            }],
            inline_contacts: vec![wa::InlineContact {
                pn_jid: Some("447700900111@s.whatsapp.net".into()),
                full_name: Some("Sam Rivera".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let raw = whatsapp_rust::waproto::codec::history_sync_to_vec(&history);
        let compressed = zlib_compress(&raw);
        // Initial, full, recent, and on-demand chunks all use the same mapping.
        for sync_type in [0, 2, 3, 6] {
            let lazy = LazyHistorySync::new(
                whatsapp_rust::bytes::Bytes::from(compressed.clone()),
                raw.len(),
                sync_type,
                None,
                None,
            );
            let book = Mutex::new(map::LearnedBook::default());
            let events = events_from_history(&lazy, &book);
            let contacts = events.iter().find_map(|event| match event {
                ChannelEvent::Contacts(contacts) => Some(contacts),
                _ => None,
            });
            let Some(contacts) = contacts else {
                panic!("sync type {sync_type} must store the people in the chunk");
            };
            assert!(
                contacts.iter().any(
                    |contact| contact.phone == "447700900222" && contact.name == "Ada Lovelace"
                ),
                "hidden id whose phone is only in the chunk map, sync type {sync_type}"
            );
            assert!(
                contacts
                    .iter()
                    .any(|contact| contact.phone == "447700900666" && contact.name == "Pat Lee")
            );
            assert!(
                contacts
                    .iter()
                    .any(|contact| contact.phone == "447700900111" && contact.name == "Sam Rivera")
            );
            assert!(
                !contacts
                    .iter()
                    .any(|contact| contact.phone == "123456789012345")
            );
        }
    }

    #[test]
    fn a_later_connect_asks_for_contact_and_chat_collections_without_deleting_the_session() {
        let names: Vec<_> = contact_and_chat_collections()
            .into_iter()
            .map(|name| name.as_str())
            .collect();
        assert_eq!(
            names,
            ["critical_unblock_low", "regular_low", "regular_high"]
        );
        let src = link_source();
        let start = src
            .find("async fn replay_contact_and_chat_collections")
            .expect("replay");
        let body = &src[start..];
        let end = body.find("\nfn ").expect("next item");
        let body = &body[..end];
        assert!(body.contains("full_sync: true"));
        assert!(body.contains("process_sync_task"));
        let fresh = body
            .find("fresh_contact_and_chat_baseline")
            .expect("a matched collection is lowered before the snapshot");
        let sync = body.find("process_sync_task").expect("snapshot request");
        assert!(
            fresh < sync,
            "a version-matched collection must be lowered before the snapshot is requested"
        );
        assert!(!body.contains("forget("));
        assert!(!body.contains("remove_file"));
        assert!(!body.contains(DB_FILE));
        let handler = src.find(".on_event(").expect("handler");
        let handler = &src[handler..src.find("if let Some(phone_number)").expect("pair")];
        let sink_at = handler.find("sink(mapped)").expect("connected is reported");
        let replay_at = handler
            .find("replay_contact_and_chat_collections")
            .expect("replay follows");
        assert!(
            sink_at < replay_at,
            "the link is reported online before contact collections are fetched again"
        );
    }

    #[test]
    fn a_version_matched_contact_collection_is_made_fresh_before_replay() {
        use whatsapp_rust::store::AppSyncStore;
        use whatsapp_rust::wacore::appstate::processor::AppStateMutationMAC;
        let dir = std::env::temp_dir().join(format!(
            "sovra-wa-baseline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("baseline dir");
        let url = dir.join(DB_FILE).to_string_lossy().into_owned();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let store = SqliteStore::new(&url).await.expect("session store");
                let matched = HashState {
                    version: 9,
                    hash: [7; 128],
                    index_value_map: std::collections::HashMap::new(),
                    mac_mismatch_fatal: true,
                };
                let mac = AppStateMutationMAC {
                    index_mac: vec![1, 2, 3, 4],
                    value_mac: vec![9, 9, 9],
                };
                for name in contact_and_chat_collections() {
                    store
                        .set_version(name.as_str(), matched.clone())
                        .await
                        .expect("seed version");
                    store
                        .put_mutation_macs(
                            name.as_str(),
                            matched.version,
                            std::slice::from_ref(&mac),
                        )
                        .await
                        .expect("seed mac");
                }
                let untouched = WAPatchName::CriticalBlock.as_str();
                store
                    .set_version(untouched, matched.clone())
                    .await
                    .expect("seed other collection");
                store
                    .put_mutation_macs(untouched, matched.version, std::slice::from_ref(&mac))
                    .await
                    .expect("seed other mac");

                fresh_contact_and_chat_baseline(&store).await;

                for name in contact_and_chat_collections() {
                    let state = store.get_version(name.as_str()).await.expect("version");
                    assert_eq!(
                        state.version,
                        0,
                        "{} must not stay at the matched version",
                        name.as_str()
                    );
                    assert_eq!(state.hash, [0; 128], "{}", name.as_str());
                    assert!(state.index_value_map.is_empty(), "{}", name.as_str());
                    assert!(!state.mac_mismatch_fatal, "{}", name.as_str());
                    let kept = store
                        .get_mutation_mac(name.as_str(), &mac.index_mac)
                        .await
                        .expect("mac read");
                    assert_eq!(kept, None, "{}", name.as_str());
                }
                let other = store.get_version(untouched).await.expect("other version");
                assert_eq!(other.version, 9);
                assert_eq!(other.hash, [7; 128]);
                assert!(other.mac_mismatch_fatal);
                let other_mac = store
                    .get_mutation_mac(untouched, &mac.index_mac)
                    .await
                    .expect("other mac");
                assert_eq!(other_mac.as_deref(), Some(mac.value_mac.as_slice()));
            });
        std::fs::remove_dir_all(&dir).expect("remove baseline dir");
    }

    #[test]
    fn forgetting_drops_learned_people_and_a_missing_link_pulls_nothing() {
        let dir = std::env::temp_dir().join(format!("sovra-wa-book-{}", std::process::id()));
        let channel = WhatsAppChannel::new(&dir);
        channel
            .book
            .lock()
            .expect("book")
            .publish(map::map_sync(map::SyncBatch {
                contacts: &[map::ContactRow {
                    jid: "447700900111@s.whatsapp.net",
                    pn_jid: None,
                    full_name: Some("Sam Rivera"),
                    first_name: None,
                    save_on_primary_addressbook: Some(true),
                    removed: false,
                }],
                ..map::SyncBatch::empty()
            }));
        assert_eq!(
            channel.book.lock().expect("book").snapshot_contacts().len(),
            1
        );
        channel.forget().unwrap();
        assert_eq!(
            channel.book.lock().expect("book").snapshot_contacts().len(),
            0
        );
        let pulled = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(channel.pull_contacts())
            .expect("pull");
        assert!(pulled.is_empty());
    }

    fn zlib_compress(raw: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(raw).expect("zlib write");
        encoder.finish().expect("zlib finish")
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
