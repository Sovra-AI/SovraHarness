use std::collections::BTreeMap;
use std::sync::Mutex;

use sovra_core::storage::{StorageEntry, StorageService};

use super::*;

/// One place's files in memory; any secret works.
#[derive(Debug, Default)]
struct Memory(Mutex<BTreeMap<String, Vec<u8>>>);

#[async_trait::async_trait]
impl StorageConnector for Memory {
    fn secret(&self, _: &StoragePlace) -> Result<Option<StorageSecret>, StorageError> {
        Ok(StorageSecret::new("secret"))
    }
    fn set_secret(&self, _: &StoragePlace, _: Option<&StorageSecret>) -> Result<(), StorageError> {
        Ok(())
    }
    async fn list(
        &self,
        _: &StoragePlace,
        _: &StorageSecret,
        folder: &str,
        limit: usize,
    ) -> Result<Listing, StorageError> {
        let entries = self
            .0
            .lock()
            .expect("lock")
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
        _: &StoragePlace,
        _: &StorageSecret,
        path: &str,
        _: usize,
    ) -> Result<Vec<u8>, StorageError> {
        self.0
            .lock()
            .expect("lock")
            .get(path)
            .cloned()
            .ok_or_else(|| StorageError::Failed("That file was not found.".into()))
    }
    async fn write(
        &self,
        _: &StoragePlace,
        _: &StorageSecret,
        path: &str,
        bytes: Vec<u8>,
        _: &str,
    ) -> Result<(), StorageError> {
        self.0.lock().expect("lock").insert(path.to_owned(), bytes);
        Ok(())
    }
}

fn shared(allow_saving: bool) -> (Shared, Arc<Memory>) {
    let memory = Arc::new(Memory::default());
    memory
        .0
        .lock()
        .expect("lock")
        .insert("notes/old.md".into(), b"# Old notes".to_vec());
    let place = StoragePlace {
        id: "place-1".into(),
        name: "Notes".into(),
        bucket: "notes".into(),
        user: "key".into(),
        allow_saving,
        ..StoragePlace::new(StorageService::AmazonS3)
    };
    let shared = Shared {
        places: Arc::new(RwLock::new(vec![PlaceView {
            place,
            has_secret: true,
        }])),
        connector: memory.clone(),
    };
    (shared, memory)
}

fn save(path: &str, replace: bool) -> StorageRequest {
    StorageRequest::Save {
        place: "notes".into(),
        path: path.into(),
        text: "new".into(),
        replace,
    }
}

#[tokio::test]
async fn tools_list_and_read_text_inside_the_named_place() {
    let (shared, _) = shared(false);
    let places = call(&shared, None, StorageRequest::Places)
        .await
        .expect("places");
    assert_eq!(places["places"][0]["name"], "Notes");
    assert_eq!(places["places"][0]["can_save"], false);
    let listing = call(
        &shared,
        None,
        StorageRequest::List {
            place: String::new(),
            folder: "/notes/".into(),
        },
    )
    .await
    .expect("one place needs no name");
    assert_eq!(listing["entries"][0]["path"], "notes/old.md");
    let read = call(
        &shared,
        None,
        StorageRequest::Read {
            place: "Notes".into(),
            path: "notes/old.md".into(),
        },
    )
    .await
    .expect("read");
    assert_eq!(read["text"], "# Old notes");
    assert_eq!(read["untrusted"], UNTRUSTED);
    let escape = call(
        &shared,
        None,
        StorageRequest::Read {
            place: "Notes".into(),
            path: "../etc/passwd".into(),
        },
    )
    .await;
    assert!(escape.is_err());
}

#[tokio::test]
async fn saving_needs_permission_and_never_replaces_without_being_asked() {
    let (read_only, memory) = shared(false);
    let refused = call(&read_only, None, save("notes/new.md", false))
        .await
        .expect_err("saving is off");
    assert!(refused.contains("turned off"), "{refused}");
    assert!(!memory.0.lock().expect("lock").contains_key("notes/new.md"));

    let (writable, memory) = shared(true);
    let saved = call(&writable, None, save("notes/new.md", false))
        .await
        .expect("saved");
    assert_eq!(saved["replaced"], false);
    assert_eq!(
        memory
            .0
            .lock()
            .expect("lock")
            .get("notes/new.md")
            .map(Vec::as_slice),
        Some(b"new".as_slice())
    );

    let kept = call(&writable, None, save("notes/old.md", false))
        .await
        .expect_err("an existing file is kept");
    assert!(kept.contains("already exists"), "{kept}");
    assert_eq!(
        memory
            .0
            .lock()
            .expect("lock")
            .get("notes/old.md")
            .map(Vec::as_slice),
        Some(b"# Old notes".as_slice())
    );
    let replaced = call(&writable, None, save("notes/old.md", true))
        .await
        .expect("replaced when asked");
    assert_eq!(replaced["replaced"], true);
}
