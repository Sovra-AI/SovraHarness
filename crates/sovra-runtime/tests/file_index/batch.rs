//! A multi-folder selection remains one request while scans stay independently controllable.

use super::*;
use sovra_core::file_index::{
    FileHistory, FileHistoryQuery, HistoricalPassage, HistoryPassageRef, IndexChanges, IndexWatch,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct BatchStore {
    roots: Mutex<BTreeMap<String, IndexedRoot>>,
    scans: Mutex<Vec<String>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl FileIndex for BatchStore {
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        Ok(self
            .roots
            .lock()
            .expect("batch fixture lock")
            .values()
            .cloned()
            .collect())
    }
    fn recover(&self) -> Result<(), IndexError> {
        Ok(())
    }
    fn add_root(&self, path: &Path) -> Result<IndexedRoot, IndexError> {
        if path == Path::new("/missing") {
            return Err(IndexError("Folder is unavailable".into()));
        }
        let id = path.to_string_lossy().into_owned();
        let root = IndexedRoot {
            id: id.clone(),
            path: path.into(),
            status: IndexStatus::Pending,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: true,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        };
        Ok(self
            .roots
            .lock()
            .expect("batch fixture lock")
            .entry(id)
            .or_insert(root)
            .clone())
    }
    fn scan(&self, id: &str, observer: &dyn ScanObserver) -> Result<IndexedRoot, IndexError> {
        let mut root = self
            .roots
            .lock()
            .expect("batch fixture lock")
            .get(id)
            .expect("registered batch folder")
            .clone();
        root.status = IndexStatus::Scanning;
        observer.progress(&root);
        self.scans
            .lock()
            .expect("batch fixture lock")
            .push(id.into());
        if id == "/selected/00" {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !observer.cancelled() {
                match self
                    .release
                    .lock()
                    .expect("batch fixture operation succeeds")
                    .recv_timeout(Duration::from_millis(20))
                {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => assert!(Instant::now() < deadline),
                }
            }
        }
        root.status = if observer.cancelled() {
            IndexStatus::Cancelled
        } else {
            IndexStatus::Ready
        };
        self.roots
            .lock()
            .expect("batch fixture lock")
            .insert(id.into(), root.clone());
        observer.progress(&root);
        Ok(root)
    }
    fn set_auto_refresh(&self, id: &str, enabled: bool) -> Result<(), IndexError> {
        self.roots
            .lock()
            .expect("batch fixture lock")
            .get_mut(id)
            .expect("registered batch folder")
            .auto_refresh = enabled;
        Ok(())
    }
    fn watch(
        &self,
        _: &IndexedRoot,
        _: Arc<dyn IndexChanges>,
    ) -> Result<Box<dyn IndexWatch>, IndexError> {
        Err(IndexError("Polling fixture".into()))
    }
    fn remove(&self, id: &str) -> Result<(), IndexError> {
        self.roots.lock().expect("batch fixture lock").remove(id);
        Ok(())
    }
    fn search(&self, _: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        Ok(vec![])
    }
    fn read(&self, _: &PassageRef) -> Result<IndexPassage, IndexError> {
        Err(IndexError("Unused".into()))
    }
    fn catalogue(&self, _: &CatalogQuery) -> Result<CatalogPage, IndexError> {
        Err(IndexError("Unused".into()))
    }
    fn history(&self, _: &FileHistoryQuery) -> Result<FileHistory, IndexError> {
        Err(IndexError("Unused".into()))
    }
    fn read_history(&self, _: &HistoryPassageRef) -> Result<HistoricalPassage, IndexError> {
        Err(IndexError("Unused".into()))
    }
}

fn until(client: &sovra_runtime::Client, mut predicate: impl FnMut(Event) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for batch operation"
        );
        match client.events().try_recv() {
            Ok(event) => {
                if predicate(event) {
                    return;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::park_timeout(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => {
                panic!("Runtime closed during batch operation")
            }
        }
    }
}

#[test]
fn selection_beyond_queue_capacity_registers_every_folder_and_keeps_pause_remove_and_ping_live() {
    let (release, receiver) = mpsc::channel();
    let store = Arc::new(BatchStore {
        roots: Mutex::new(BTreeMap::new()),
        scans: Mutex::new(vec![]),
        release: Mutex::new(receiver),
    });
    let client = sovra_runtime::start_with_file_index(
        "batch-test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        Some(store.clone()),
    )
    .unwrap();
    until(&client, |event| {
        matches!(event, Event::FileIndex(IndexEvent::Roots(_)))
    });
    let mut paths: Vec<_> = (0..24)
        .map(|i| format!("/selected/{i:02}").into())
        .collect();
    paths.insert(1, "/missing".into());
    paths.push("/selected/01".into());
    client
        .send(Command::FileIndex(IndexCommand::AddRoots { paths }))
        .unwrap();
    let mut reported = false;
    until(&client, |event| {
        match event {
            Event::FileIndex(IndexEvent::FoldersAdded(result)) => {
                assert_eq!(result.added.len(), 24);
                assert_eq!(result.failed.len(), 1);
                assert_eq!(result.failed[0].path, Path::new("/missing"));
                assert!(result.failed[0].reason.contains("unavailable"));
                reported = true;
            }
            Event::FileIndex(IndexEvent::Progress(root))
                if root.status == IndexStatus::Scanning =>
            {
                assert!(reported, "Selection results must precede any scan");
                assert_eq!(root.id, "/selected/00");
                return true;
            }
            Event::FileIndex(IndexEvent::Error(message)) => {
                panic!("Unexpected queue error: {message}")
            }
            _ => {}
        }
        false
    });
    assert_eq!(store.list().unwrap().len(), 24);
    client
        .send(Command::FileIndex(IndexCommand::Cancel {
            root_id: "/selected/01".into(),
        }))
        .unwrap();
    client
        .send(Command::FileIndex(IndexCommand::Remove {
            root_id: "/selected/02".into(),
        }))
        .unwrap();
    client.send(Command::Ping).unwrap();
    until(&client, |event| event == Event::Pong);
    assert_eq!(
        *store.scans.lock().expect("batch fixture lock"),
        vec!["/selected/00"]
    );
    release.send(()).unwrap();
    until(&client, |event| {
        matches!(event, Event::FileIndex(IndexEvent::Roots(roots))
        if roots.len() == 23 && roots.iter().filter(|root| root.status == IndexStatus::Ready).count() == 22)
    });
    let scanned = store.scans.lock().expect("batch fixture lock").clone();
    assert_eq!(scanned.len(), 22);
    assert!(
        !scanned
            .iter()
            .any(|id| id == "/selected/01" || id == "/selected/02")
    );
    assert!(!store.roots.lock().expect("batch fixture lock")["/selected/01"].auto_refresh);
    // Reselecting a connected folder keeps its paused preference, and duplicate
    // selections do not create more scopes or trigger another scan.
    client
        .send(Command::FileIndex(IndexCommand::AddRoots {
            paths: vec![
                "/selected/01".into(),
                "/selected/01".into(),
                "/missing".into(),
            ],
        }))
        .unwrap();
    until(&client, |event| {
        matches!(event, Event::FileIndex(IndexEvent::FoldersAdded(result))
        if result.added.len() == 1 && result.failed.len() == 1)
    });
    assert_eq!(store.scans.lock().expect("batch fixture lock").len(), 22);
    assert!(!store.roots.lock().expect("batch fixture lock")["/selected/01"].auto_refresh);
    client.send(Command::Shutdown).unwrap();
}
