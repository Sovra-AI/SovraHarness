//! Online storage through the real runtime, with an in-memory storage service.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sovra_core::storage::{
    Listing, PlaceStatus, StorageCommand, StorageConnector, StorageEntry, StorageError,
    StorageEvent, StoragePlace, StorageSecret, StorageService,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, RuntimeServices, StorageConfig};

/// Accepts only the secret `good`, and holds files in memory.
#[derive(Debug, Default)]
struct FakeStorage {
    secrets: Mutex<BTreeMap<String, String>>,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl FakeStorage {
    fn check(secret: &StorageSecret) -> Result<(), StorageError> {
        if secret.as_str() == "good" {
            Ok(())
        } else {
            Err(StorageError::Failed(
                "The storage service did not accept the sign-in.".into(),
            ))
        }
    }
}

#[async_trait::async_trait]
impl StorageConnector for FakeStorage {
    fn secret(&self, place: &StoragePlace) -> Result<Option<StorageSecret>, StorageError> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&place.secret_entry())
            .and_then(|secret| StorageSecret::new(secret.clone())))
    }

    fn set_secret(
        &self,
        place: &StoragePlace,
        secret: Option<&StorageSecret>,
    ) -> Result<(), StorageError> {
        let mut secrets = self.secrets.lock().unwrap();
        match secret {
            Some(secret) => secrets.insert(place.secret_entry(), secret.as_str().to_owned()),
            None => secrets.remove(&place.secret_entry()),
        };
        Ok(())
    }

    async fn list(
        &self,
        _place: &StoragePlace,
        secret: &StorageSecret,
        folder: &str,
        limit: usize,
    ) -> Result<Listing, StorageError> {
        Self::check(secret)?;
        let entries = self
            .files
            .lock()
            .unwrap()
            .keys()
            .filter(|path| sovra_core::storage::parent_folder(path) == folder)
            .map(|path| StorageEntry {
                path: path.clone(),
                name: StorageEntry::name_of(path),
                folder: false,
                size: None,
                modified: None,
            })
            .collect();
        Ok(Listing {
            entries,
            more: false,
        }
        .tidy(folder, limit))
    }

    async fn read(
        &self,
        _place: &StoragePlace,
        secret: &StorageSecret,
        path: &str,
        _max_bytes: usize,
    ) -> Result<Vec<u8>, StorageError> {
        Self::check(secret)?;
        self.files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| StorageError::Failed("That file was not found.".into()))
    }

    async fn write(
        &self,
        _place: &StoragePlace,
        secret: &StorageSecret,
        path: &str,
        bytes: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), StorageError> {
        Self::check(secret)?;
        self.files.lock().unwrap().insert(path.to_owned(), bytes);
        Ok(())
    }
}

fn start(storage: Arc<FakeStorage>, directory: &std::path::Path) -> Client {
    sovra_runtime::start_with_services(
        "test",
        AgentConfig::default(),
        RuntimeServices {
            storage: Some(StorageConfig {
                connector: storage,
                directory: directory.to_owned(),
            }),
            ..RuntimeServices::default()
        },
    )
    .unwrap()
}

/// The next storage event, skipping other events, within a bounded wait.
fn next(client: &Client) -> StorageEvent {
    let events = client.events();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "no storage event in time");
        if let Event::Storage(event) = events.recv_blocking().unwrap() {
            return event;
        }
    }
}

fn send(client: &Client, command: StorageCommand) {
    client.send(Command::Storage(command)).unwrap();
}

fn bucket() -> StoragePlace {
    StoragePlace {
        name: "Family photos".into(),
        bucket: "family-photos".into(),
        user: "AKIDEXAMPLE".into(),
        ..StoragePlace::new(StorageService::CloudflareR2)
    }
}

#[test]
fn a_place_is_kept_only_after_the_service_says_yes_and_survives_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Arc::new(FakeStorage::default());
    storage
        .files
        .lock()
        .unwrap()
        .insert("notes.md".into(), b"hello".to_vec());
    // The runtime makes its own owner-only folder, as it does in the app.
    let places_dir = directory.path().join("storage");
    let client = start(storage.clone(), &places_dir);
    assert_eq!(
        next(&client),
        StorageEvent::Places(Vec::new()),
        "loads on start"
    );

    // R2 needs its own address; the form's example is refused.
    let mut place = bucket();
    place.endpoint = StorageService::CloudflareR2.address_example().into();
    send(
        &client,
        StorageCommand::Save {
            place: place.clone(),
            secret: StorageSecret::new("good"),
        },
    );
    let StorageEvent::SaveRefused(message) = next(&client) else {
        panic!("an example address must be refused");
    };
    assert!(message.contains("< >"), "{message}");

    place.endpoint = "https://0123abcd.r2.cloudflarestorage.com".into();
    send(
        &client,
        StorageCommand::Save {
            place: place.clone(),
            secret: StorageSecret::new("wrong"),
        },
    );
    assert_eq!(
        next(&client),
        StorageEvent::SaveRefused("The storage service did not accept the sign-in.".into())
    );
    assert!(storage.secrets.lock().unwrap().is_empty(), "nothing kept");

    send(
        &client,
        StorageCommand::Save {
            place: place.clone(),
            secret: StorageSecret::new("good"),
        },
    );
    let event = next(&client);
    let StorageEvent::Places(places) = event else {
        panic!("the list follows a save: {event:?}");
    };


    assert_eq!(places.len(), 1);
    let saved = places[0].place.clone();
    assert!(places[0].has_secret);
    assert!(saved.id.starts_with("place-"));
    assert_eq!(saved.region, "auto");
    assert!(matches!(
        next(&client),
        StorageEvent::Status {
            status: PlaceStatus::Works,
            ..
        }
    ));
    assert_eq!(
        next(&client),
        StorageEvent::Saved {
            id: saved.id.clone(),
            name: "Family photos".into()
        }
    );

    // The saved list holds no secret.
    let file = std::fs::read_to_string(places_dir.join("places.json")).unwrap();
    assert!(
        file.contains("Family photos") && !file.contains("good"),
        "{file}"
    );

    // Editing without retyping keeps the saved secret.
    let mut renamed = saved.clone();
    renamed.name = "Photos".into();
    send(
        &client,
        StorageCommand::Save {
            place: renamed,
            secret: None,
        },
    );
    let StorageEvent::Places(places) = next(&client) else {
        panic!("the list follows an edit");
    };
    assert_eq!(places[0].place.name, "Photos");
    assert_eq!(places[0].place.id, saved.id);

    send(
        &client,
        StorageCommand::Browse {
            id: saved.id.clone(),
            folder: String::new(),
        },
    );
    let listing = loop {
        if let StorageEvent::Listing { result, .. } = next(&client) {
            break result.unwrap();
        }
    };
    assert_eq!(listing.entries[0].path, "notes.md");
    client.shutdown_blocking().unwrap();

    let client = start(storage.clone(), &places_dir);
    let StorageEvent::Places(places) = next(&client) else {
        panic!("loads on start");
    };
    assert_eq!(places.len(), 1);
    assert!(places[0].has_secret);
    assert!(
        matches!(
            next(&client),
            StorageEvent::Status {
                status: PlaceStatus::Works,
                ..
            }
        ),
        "checked again on start"
    );

    send(&client, StorageCommand::Remove { id: saved.id });
    assert_eq!(next(&client), StorageEvent::Places(Vec::new()));
    let StorageEvent::Notice(notice) = next(&client) else {
        panic!("removing says so");
    };
    assert!(notice.contains("not touched"), "{notice}");
    assert!(storage.secrets.lock().unwrap().is_empty(), "secret removed");
    assert!(
        storage.files.lock().unwrap().contains_key("notes.md"),
        "files untouched"
    );
    client.shutdown_blocking().unwrap();
}
