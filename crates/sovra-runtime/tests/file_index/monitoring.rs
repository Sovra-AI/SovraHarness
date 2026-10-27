//! Monitoring lifecycle driven through the public runtime client.

use sovra_core::{
    AgentConfig, Command, Event,
    file_index::{
        CatalogPage, CatalogQuery, FileIndex, IndexChanges, IndexCommand, IndexError, IndexEvent,
        IndexPassage, IndexProgress, IndexQuery, IndexStatus, IndexWatch, IndexedRoot, MonitorMode,
        PassageRef, ScanObserver,
    },
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct MonitorStore {
    root: Mutex<Option<IndexedRoot>>,
    changes: Mutex<Option<Arc<dyn IndexChanges>>>,
    scans: AtomicUsize,
    pause_fails: AtomicBool,
}
impl std::fmt::Debug for MonitorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MonitorStore")
    }
}
struct Guard;
impl IndexWatch for Guard {}

impl FileIndex for MonitorStore {
    fn history(
        &self,
        _: &sovra_core::file_index::FileHistoryQuery,
    ) -> Result<sovra_core::file_index::FileHistory, IndexError> {
        Err(IndexError("No history in this scan fixture.".into()))
    }
    fn read_history(
        &self,
        _: &sovra_core::file_index::HistoryPassageRef,
    ) -> Result<sovra_core::file_index::HistoricalPassage, IndexError> {
        Err(IndexError("No history in this scan fixture.".into()))
    }
    fn recover(&self) -> Result<(), IndexError> {
        Ok(())
    }
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        Ok(self
            .root
            .lock()
            .expect("root lock")
            .iter()
            .cloned()
            .collect())
    }
    fn add_root(&self, _: &Path) -> Result<IndexedRoot, IndexError> {
        self.list()?
            .into_iter()
            .next()
            .ok_or_else(|| IndexError("No root".into()))
    }
    fn set_auto_refresh(&self, _: &str, enabled: bool) -> Result<(), IndexError> {
        if !enabled && self.pause_fails.load(Ordering::SeqCst) {
            return Err(IndexError("Synthetic preference write failure".into()));
        }
        self.root
            .lock()
            .expect("root lock")
            .as_mut()
            .ok_or_else(|| IndexError("No root".into()))?
            .auto_refresh = enabled;
        Ok(())
    }
    fn scan(&self, _: &str, observer: &dyn ScanObserver) -> Result<IndexedRoot, IndexError> {
        let mut root = self.root.lock().expect("root lock");
        let root = root.as_mut().ok_or_else(|| IndexError("No root".into()))?;
        root.status = IndexStatus::Scanning;
        observer.progress(root);
        self.scans.fetch_add(1, Ordering::SeqCst);
        root.status = IndexStatus::Ready;
        observer.progress(root);
        Ok(root.clone())
    }
    fn watch(
        &self,
        _: &IndexedRoot,
        changes: Arc<dyn IndexChanges>,
    ) -> Result<Box<dyn IndexWatch>, IndexError> {
        *self.changes.lock().expect("changes lock") = Some(changes);
        Ok(Box::new(Guard))
    }
    fn remove(&self, _: &str) -> Result<(), IndexError> {
        self.root.lock().expect("root lock").take();
        Ok(())
    }
    fn search(&self, _: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        Ok(vec![])
    }
    fn read(&self, _: &PassageRef) -> Result<IndexPassage, IndexError> {
        Err(IndexError("No passage".into()))
    }
    fn catalogue(&self, query: &CatalogQuery) -> Result<CatalogPage, IndexError> {
        query.validate()?;
        Ok(CatalogPage {
            revision: 0,
            total_files: 0,
            total_known_bytes: 0,
            unknown_sizes: 0,
            coverage: vec![],
            files: Vec::new(),
            groups: Vec::new(),
            next_cursor: None,
            has_more: false,
        })
    }
}

fn monitoring(client: &sovra_runtime::Client, mode: MonitorMode) {
    // A quiet run finishes in a few seconds. Parallel workspace tests can leave
    // this runtime unscheduled long enough to miss a 10 second wall-clock bound.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "Timed out waiting for {mode:?}"
        );
        if let Event::FileIndex(IndexEvent::Monitoring { status, .. }) =
            match client.events().try_recv() {
                Ok(event) => event,
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::park_timeout(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(async_channel::TryRecvError::Closed) => {
                    panic!("runtime stopped before monitoring event")
                }
            }
            && status.mode == mode
        {
            return;
        }
    }
}

#[test]
fn resume_reconciles_and_native_failure_falls_back_then_pause_and_remove_revoke_updates() {
    let store = Arc::new(MonitorStore {
        root: Mutex::new(Some(IndexedRoot {
            id: "root".into(),
            path: "/selected".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: false,
        })),
        ..MonitorStore::default()
    });
    let client = sovra_runtime::start_with_file_index(
        "test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        Some(store.clone()),
    )
    .unwrap();
    monitoring(&client, MonitorMode::Paused);
    assert_eq!(store.scans.load(Ordering::SeqCst), 0);
    client
        .send(Command::FileIndex(IndexCommand::SetAutoRefresh {
            root_id: "root".into(),
            enabled: true,
        }))
        .unwrap();
    monitoring(&client, MonitorMode::Native);
    loop {
        if matches!(client.events().recv_blocking().unwrap(), Event::FileIndex(IndexEvent::Progress(root)) if root.status == IndexStatus::Ready)
        {
            break;
        }
    }
    monitoring(&client, MonitorMode::Native);
    assert_eq!(store.scans.load(Ordering::SeqCst), 1);
    let signal = store.changes.lock().unwrap().clone().unwrap();
    signal.failed("Synthetic dropped event stream".into());
    monitoring(&client, MonitorMode::Periodic);
    store.pause_fails.store(true, Ordering::SeqCst);
    client
        .send(Command::FileIndex(IndexCommand::Cancel {
            root_id: "root".into(),
        }))
        .unwrap();
    monitoring(&client, MonitorMode::Paused);
    assert!(
        store.list().unwrap()[0].auto_refresh,
        "the fixture's persisted value remains unchanged after its write fails"
    );
    loop {
        if let Event::FileIndex(IndexEvent::Error(message)) =
            client.events().recv_blocking().unwrap()
        {
            assert!(message.contains("paused for this session"));
            assert!(message.contains("Synthetic preference write failure"));
            break;
        }
    }
    for _ in 0..1_000 {
        signal.changed();
    }
    client.send(Command::Ping).unwrap();
    loop {
        if client.events().recv_blocking().unwrap() == Event::Pong {
            break;
        }
    }
    assert_eq!(store.scans.load(Ordering::SeqCst), 1);
    store.pause_fails.store(false, Ordering::SeqCst);
    client
        .send(Command::FileIndex(IndexCommand::Remove {
            root_id: "root".into(),
        }))
        .unwrap();
    loop {
        if matches!(client.events().recv_blocking().unwrap(), Event::FileIndex(IndexEvent::Roots(roots)) if roots.is_empty())
        {
            break;
        }
    }
    signal.changed();
    assert_eq!(store.list().unwrap(), [] as [IndexedRoot; 0]);
    client.send(Command::Shutdown).unwrap();
}
