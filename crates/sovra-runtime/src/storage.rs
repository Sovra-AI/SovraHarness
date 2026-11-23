//! Online storage places: the saved list, their keychain secrets, checks, the
//! page's file browser, and the guarded calls agent tools make.
//!
//! One task owns the list and is the only writer. Checks and listings run in
//! tasks it owns (a `JoinSet`, aborted when the runtime stops), and report
//! back as messages. Keychain work runs on blocking workers. The list lives in
//! an owner-private file; secrets never do.

use std::path::PathBuf;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use sovra_adk::StorageTools;
use sovra_core::Event;
use sovra_core::files::FileStore;
use sovra_core::storage::{
    Listing, MAX_LISTING, MAX_PLACES, MAX_READ_BYTES, MAX_SAVE_BYTES, MAX_TEXT_CHARS, PlaceStatus,
    PlaceView, StorageCommand, StorageConnector, StorageError, StorageEvent, StoragePlace,
    StorageRequest, StorageSecret, UNTRUSTED, clean_path, content_type, new_place_id,
    resolve_place,
};
use tokio::sync::RwLock;

use crate::private_storage::{read_private_bounded, write_private};

const PLACES_FILE: &str = "places.json";
/// Far more than [`MAX_PLACES`] places need.
const MAX_PLACES_BYTES: u64 = 256 * 1024;

/// What the composition root hands the runtime for online storage.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub connector: Arc<dyn StorageConnector>,
    /// An owner-private folder for the saved list.
    pub directory: PathBuf,
}

enum Message {
    Command(StorageCommand),
    SaveChecked {
        place: StoragePlace,
        secret: StorageSecret,
        result: Result<(), StorageError>,
    },
    Checked {
        id: String,
        status: PlaceStatus,
    },
}

/// The places tools may use, with the adapter they go through.
#[derive(Clone)]
struct Shared {
    places: Arc<RwLock<Vec<PlaceView>>>,
    connector: Arc<dyn StorageConnector>,
}

pub(crate) struct StorageRuntime {
    messages: Sender<Message>,
    shared: Option<Shared>,
    files: Option<Arc<dyn FileStore>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl StorageRuntime {
    pub(crate) fn start(
        config: Option<StorageConfig>,
        files: Option<Arc<dyn FileStore>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (messages, inbox) = async_channel::bounded(64);
        let Some(config) = config else {
            return Self {
                messages,
                shared: None,
                files,
                task: None,
            };
        };
        let shared = Shared {
            places: Arc::new(RwLock::new(Vec::new())),
            connector: config.connector,
        };
        let worker = Worker {
            shared: shared.clone(),
            directory: config.directory,
            events,
            messages: messages.clone(),
            tasks: tokio::task::JoinSet::new(),
            now_ms,
        };
        let task = Some(handle.spawn(worker.run(inbox)));
        Self {
            messages,
            shared: Some(shared),
            files,
            task,
        }
    }

    pub(crate) fn send(&self, command: StorageCommand) {
        if self.messages.try_send(Message::Command(command)).is_err() {
            tracing::warn!("storage command dropped: too many pending");
        }
    }

    /// Tools for this turn, while at least one place is linked.
    pub(crate) async fn tools(&self) -> Option<StorageTools> {
        let shared = self.shared.clone()?;
        if !shared
            .places
            .read()
            .await
            .iter()
            .any(|view| view.has_secret)
        {
            return None;
        }
        let files = self.files.clone();
        Some(StorageTools::from_fn(move |request| {
            let shared = shared.clone();
            let files = files.clone();
            async move { call(&shared, files, request).await }
        }))
    }
}

impl Drop for StorageRuntime {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, StorageError> + Send + 'static,
) -> Result<T, StorageError> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(StorageError::Failed(
            "Sovra's storage helper stopped unexpectedly. Please try again.".into(),
        ))
    })
}

async fn secret_of(
    connector: &Arc<dyn StorageConnector>,
    place: &StoragePlace,
) -> Result<StorageSecret, StorageError> {
    let connector = connector.clone();
    let place_for_keychain = place.clone();
    blocking(move || connector.secret(&place_for_keychain))
        .await?
        .ok_or_else(|| {
            StorageError::Failed(format!(
                "Sovra has no {} for {}. Open Connections and enter it again.",
                sovra_core::storage::in_sentence(place.service.secret_label()),
                place.name
            ))
        })
}

/// One agent tool call, checked against the current list each time: the
/// person may remove a place mid-reply.
async fn call(
    shared: &Shared,
    files: Option<Arc<dyn FileStore>>,
    request: StorageRequest,
) -> Result<Value, String> {
    let places: Vec<StoragePlace> = shared
        .places
        .read()
        .await
        .iter()
        .filter(|view| view.has_secret)
        .map(|view| view.place.clone())
        .collect();
    let changes = request.changes_things();
    let result = match request {
        StorageRequest::Places => Ok(json!({
            "places": places.iter().map(|place| json!({
                "name": place.name,
                "service": place.service.label(),
                "where": place.summary(),
                "can_save": place.allow_saving,
            })).collect::<Vec<_>>(),
        })),
        StorageRequest::List { place, folder } => {
            list_for_tool(shared, &places, &place, &folder).await
        }
        StorageRequest::Read { place, path } => {
            read_for_tool(shared, files, &places, &place, &path).await
        }
        StorageRequest::Save {
            place,
            path,
            text,
            replace,
        } => save_for_tool(shared, &places, &place, &path, text, replace).await,
    };
    tracing::info!(changes, ok = result.is_ok(), "storage tool call");
    result.map_err(|error| error.to_string())
}

fn listing_json(place: &StoragePlace, folder: &str, listing: &Listing) -> Value {
    json!({
        "place": place.name,
        "folder": folder,
        "entries": listing.entries.iter().map(|entry| {
            let mut item = json!({ "path": entry.path, "folder": entry.folder });
            if let Some(size) = entry.size {
                item["size_bytes"] = json!(size);
            }
            if let Some(modified) = &entry.modified {
                item["modified"] = json!(modified);
            }
            item
        }).collect::<Vec<_>>(),
        "more_available": listing.more,
    })
}

async fn list_for_tool(
    shared: &Shared,
    places: &[StoragePlace],
    wanted: &str,
    folder: &str,
) -> Result<Value, StorageError> {
    let place = resolve_place(places, wanted)?;
    let folder = clean_path(folder)?;
    let secret = secret_of(&shared.connector, place).await?;
    let listing = shared
        .connector
        .list(place, &secret, &folder, MAX_LISTING)
        .await?;
    Ok(listing_json(place, &folder, &listing))
}

async fn read_for_tool(
    shared: &Shared,
    files: Option<Arc<dyn FileStore>>,
    places: &[StoragePlace],
    wanted: &str,
    path: &str,
) -> Result<Value, StorageError> {
    let place = resolve_place(places, wanted)?;
    let path = clean_path(path)?;
    if path.is_empty() {
        return Err(StorageError::Invalid(
            "Give the path of a file. Call storage_list_files to find one.".into(),
        ));
    }
    let secret = secret_of(&shared.connector, place).await?;
    let bytes = shared
        .connector
        .read(place, &secret, &path, MAX_READ_BYTES)
        .await?;
    let name = sovra_core::storage::StorageEntry::name_of(&path);
    let size = bytes.len();
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) if !text.contains('\u{0}') => Ok((text.to_owned(), None)),
        _ => {
            let files = files.ok_or_else(|| {
                StorageError::Failed(
                    "This copy of Sovra cannot read documents like this one.".into(),
                )
            })?;
            let name = name.clone();
            blocking(move || {
                files
                    .read_document(&name, &bytes)
                    .map(|content| (content.text, content.attachment.note))
                    .map_err(|error| StorageError::Failed(error.to_string()))
            })
            .await
        }
    }?;
    let (text, note) = text;
    Ok(json!({
        "untrusted": UNTRUSTED,
        "place": place.name,
        "path": path,
        "size_bytes": size,
        "text": sovra_core::connected_text::clip_text(&text, MAX_TEXT_CHARS),
        "truncated": text.chars().count() > MAX_TEXT_CHARS,
        "note": note,
    }))
}

async fn save_for_tool(
    shared: &Shared,
    places: &[StoragePlace],
    wanted: &str,
    path: &str,
    text: String,
    replace: bool,
) -> Result<Value, StorageError> {
    let place = resolve_place(places, wanted)?;
    if !place.allow_saving {
        return Err(StorageError::Invalid(format!(
            "Saving is turned off for {}. The user can turn on \"Let Sovra save files here\" for it on the Connections page.",
            place.name
        )));
    }
    let path = clean_path(path)?;
    if path.is_empty() {
        return Err(StorageError::Invalid(
            "Give a file name to save, such as notes/summary.md.".into(),
        ));
    }
    if text.len() > MAX_SAVE_BYTES {
        return Err(StorageError::Invalid(
            "That is too much to save in one file. Keep it under 5 MB.".into(),
        ));
    }
    let secret = secret_of(&shared.connector, place).await?;
    // Never lose work: a file already there is only replaced when asked.
    let parent = sovra_core::storage::parent_folder(&path);
    let existing = shared
        .connector
        .list(place, &secret, &parent, MAX_LISTING)
        .await?;
    let taken = existing
        .entries
        .iter()
        .any(|entry| entry.path == path && !entry.folder);
    if taken && !replace {
        return Err(StorageError::Invalid(format!(
            "{path} already exists in {}. Ask the user before replacing it, then call again with replace set to true, or choose another name.",
            place.name
        )));
    }
    let size = text.len();
    shared
        .connector
        .write(
            place,
            &secret,
            &path,
            text.into_bytes(),
            content_type(&path),
        )
        .await?;
    Ok(json!({
        "ok": true,
        "place": place.name,
        "path": path,
        "size_bytes": size,
        "replaced": taken,
    }))
}

struct Worker {
    shared: Shared,
    directory: PathBuf,
    events: Sender<Event>,
    messages: Sender<Message>,
    tasks: tokio::task::JoinSet<()>,
    now_ms: fn() -> i64,
}

impl Worker {
    async fn run(mut self, inbox: Receiver<Message>) {
        while let Ok(message) = inbox.recv().await {
            // Finished checks are collected so the set stays small.
            while self.tasks.try_join_next().is_some() {}
            match message {
                Message::Command(command) => self.command(command).await,
                Message::SaveChecked {
                    place,
                    secret,
                    result,
                } => self.save_checked(place, secret, result).await,
                Message::Checked { id, status } => {
                    if self.find(&id).await.is_some() {
                        self.emit(StorageEvent::Status { id, status }).await;
                    }
                }
            }
        }
        self.tasks.abort_all();
    }

    async fn emit(&self, event: StorageEvent) {
        let _ = self.events.send(Event::Storage(event)).await;
    }

    async fn find(&self, id: &str) -> Option<PlaceView> {
        self.shared
            .places
            .read()
            .await
            .iter()
            .find(|view| view.place.id == id)
            .cloned()
    }

    async fn publish(&self) {
        let places = self.shared.places.read().await.clone();
        self.emit(StorageEvent::Places(places)).await;
    }

    async fn persist(&self) -> Result<(), String> {
        let places: Vec<StoragePlace> = self
            .shared
            .places
            .read()
            .await
            .iter()
            .map(|view| view.place.clone())
            .collect();
        write_private(&self.directory, PLACES_FILE, &places)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "storage places could not be saved");
                "Sovra could not save the list of storage places on this computer.".to_owned()
            })
    }

    async fn command(&mut self, command: StorageCommand) {
        match command {
            StorageCommand::Load => self.load().await,
            StorageCommand::Save { place, secret } => self.save(place, secret).await,
            StorageCommand::Remove { id } => self.remove(&id).await,
            StorageCommand::Check { id } => {
                if let Some(view) = self.find(&id).await {
                    self.check(view.place);
                    self.emit(StorageEvent::Status {
                        id,
                        status: PlaceStatus::Checking,
                    })
                    .await;
                }
            }
            StorageCommand::Browse { id, folder } => self.browse(id, folder).await,
        }
    }

    async fn load(&mut self) {
        let saved = match read_private_bounded(&self.directory, PLACES_FILE, MAX_PLACES_BYTES).await
        {
            Ok(bytes) => {
                serde_json::from_slice::<Vec<StoragePlace>>(&bytes).unwrap_or_else(|error| {
                    tracing::warn!(%error, "storage places file unreadable; starting empty");
                    Vec::new()
                })
            }
            Err(_) => Vec::new(),
        };
        let mut views = Vec::new();
        for place in saved.into_iter().take(MAX_PLACES) {
            let connector = self.shared.connector.clone();
            let lookup = place.clone();
            let has_secret = blocking(move || connector.secret(&lookup))
                .await
                .ok()
                .flatten()
                .is_some();
            views.push(PlaceView { place, has_secret });
        }
        views
            .iter()
            .filter(|view| view.has_secret)
            .for_each(|view| self.check(view.place.clone()));
        *self.shared.places.write().await = views;
        self.publish().await;
    }

    /// Lists the top of `place` in a task this worker owns.
    fn check(&mut self, place: StoragePlace) {
        let connector = self.shared.connector.clone();
        let messages = self.messages.clone();
        self.tasks.spawn(async move {
            let status = match secret_of(&connector, &place).await {
                Ok(secret) => match connector.list(&place, &secret, "", 1).await {
                    Ok(_) => PlaceStatus::Works,
                    Err(error) => PlaceStatus::Problem(error.to_string()),
                },
                Err(error) => PlaceStatus::Problem(error.to_string()),
            };
            let _ = messages
                .send(Message::Checked {
                    id: place.id,
                    status,
                })
                .await;
        });
    }

    async fn save(&mut self, place: StoragePlace, secret: Option<StorageSecret>) {
        let mut place = place.tidied();
        let existing = if place.id.is_empty() {
            None
        } else {
            self.find(&place.id).await
        };
        if !place.id.is_empty() && existing.is_none() {
            self.emit(StorageEvent::SaveRefused(
                "That storage place was removed. Add it again.".into(),
            ))
            .await;
            return;
        }
        if existing.is_none() {
            let places = self.shared.places.read().await;
            if places.len() >= MAX_PLACES {
                drop(places);
                self.emit(StorageEvent::SaveRefused(format!(
                    "Sovra keeps at most {MAX_PLACES} storage places. Remove one first."
                )))
                .await;
                return;
            }
            let used: Vec<_> = places.iter().map(|view| view.place.clone()).collect();
            drop(places);
            place.id = new_place_id((self.now_ms)(), &used);
        }
        if let Err(error) = place.validate() {
            self.emit(StorageEvent::SaveRefused(error.to_string()))
                .await;
            return;
        }
        let secret = match self.secret_for_save(&place, secret, existing.as_ref()).await {
            Ok(secret) => secret,
            Err(message) => {
                self.emit(StorageEvent::SaveRefused(message)).await;
                return;
            }
        };
        // A place is only kept once the service has said yes to it.
        let connector = self.shared.connector.clone();
        let messages = self.messages.clone();
        self.tasks.spawn(async move {
            let result = connector.list(&place, &secret, "", 1).await.map(|_| ());
            let _ = messages
                .send(Message::SaveChecked {
                    place,
                    secret,
                    result,
                })
                .await;
        });
    }

    /// The secret a save uses: the one typed, else the one already saved.
    async fn secret_for_save(
        &self,
        place: &StoragePlace,
        typed: Option<StorageSecret>,
        existing: Option<&PlaceView>,
    ) -> Result<StorageSecret, String> {
        if let Some(secret) = typed {
            secret.validate().map_err(|error| error.to_string())?;
            return Ok(secret);
        }
        let Some(view) = existing.filter(|view| view.has_secret) else {
            return Err(format!(
                "Type the {}.",
                sovra_core::storage::in_sentence(place.service.secret_label())
            ));
        };
        secret_of(&self.shared.connector, &view.place)
            .await
            .map_err(|error| error.to_string())
    }

    async fn save_checked(
        &mut self,
        place: StoragePlace,
        secret: StorageSecret,
        result: Result<(), StorageError>,
    ) {
        if let Err(error) = result {
            self.emit(StorageEvent::SaveRefused(error.to_string()))
                .await;
            return;
        }
        let connector = self.shared.connector.clone();
        let for_keychain = place.clone();
        if let Err(error) =
            blocking(move || connector.set_secret(&for_keychain, Some(&secret))).await
        {
            self.emit(StorageEvent::SaveRefused(error.to_string()))
                .await;
            return;
        }
        {
            let mut places = self.shared.places.write().await;
            let view = PlaceView {
                place: place.clone(),
                has_secret: true,
            };
            match places.iter_mut().find(|view| view.place.id == place.id) {
                Some(slot) => *slot = view,
                None => places.push(view),
            }
        }
        if let Err(message) = self.persist().await {
            self.emit(StorageEvent::Notice(message)).await;
        }
        tracing::info!(service = place.service.slug(), "storage place saved");
        self.publish().await;
        self.emit(StorageEvent::Status {
            id: place.id.clone(),
            status: PlaceStatus::Works,
        })
        .await;
        self.emit(StorageEvent::Saved {
            id: place.id,
            name: place.name,
        })
        .await;
    }

    async fn remove(&mut self, id: &str) {
        let Some(view) = self.find(id).await else {
            return;
        };
        let connector = self.shared.connector.clone();
        let place = view.place.clone();
        if let Err(error) = blocking(move || connector.set_secret(&place, None)).await {
            tracing::warn!(%error, "storage secret could not be removed");
        }
        self.shared
            .places
            .write()
            .await
            .retain(|view| view.place.id != id);
        let notice = match self.persist().await {
            Ok(()) => format!(
                "{} is no longer linked. The files in it were not touched.",
                view.place.name
            ),
            Err(message) => message,
        };
        self.publish().await;
        self.emit(StorageEvent::Notice(notice)).await;
    }

    async fn browse(&mut self, id: String, folder: String) {
        let Some(view) = self.find(&id).await else {
            return;
        };
        let folder_clean = match clean_path(&folder) {
            Ok(folder) => folder,
            Err(error) => {
                self.emit(StorageEvent::Listing {
                    id,
                    folder,
                    result: Err(error.to_string()),
                })
                .await;
                return;
            }
        };
        let connector = self.shared.connector.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            let result = match secret_of(&connector, &view.place).await {
                Ok(secret) => connector
                    .list(&view.place, &secret, &folder_clean, MAX_LISTING)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = events
                .send(Event::Storage(StorageEvent::Listing { id, folder, result }))
                .await;
        });
    }
}

#[cfg(test)]
mod tests;
