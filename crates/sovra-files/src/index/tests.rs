use super::*;
mod catalogue;
mod graph;
mod history;
mod semantic;
mod summary;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sovra-index-{}-{}-{}",
            std::process::id(),
            now(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("source")).unwrap();
        Self(root)
    }
    fn store(&self) -> LocalFileIndex {
        LocalFileIndex::new(self.0.join("index"))
    }
    fn source(&self) -> PathBuf {
        self.0.join("source")
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.source().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Observer {
    cancel: AtomicBool,
    cancel_at: Option<u64>,
    interrupted: bool,
}
impl ScanObserver for Observer {
    fn cancellation(&self) -> Option<ScanCancellation> {
        if self.interrupted {
            Some(ScanCancellation::Interrupted)
        } else {
            self.cancel
                .load(Ordering::Relaxed)
                .then_some(ScanCancellation::Requested)
        }
    }
    fn progress(&self, root: &IndexedRoot) {
        if self.cancel_at.is_some_and(|n| root.progress.visited >= n) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[test]
fn index_storage_does_not_reinstate_the_former_one_gib_page_cap() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for _ in 0..2 {
        let db = store.connect().unwrap();
        let max_pages: i64 = db
            .query_row("PRAGMA max_page_count", [], |row| row.get(0))
            .unwrap();
        assert!(max_pages > 262_144);
    }
}

#[test]
fn former_scan_budget_thresholds_do_not_prevent_the_next_file() {
    let fixture = Fixture::new();
    fixture.write("next.txt", "Nextneedle is included.");
    let store = fixture.store();
    let mut root = store.add_root(&fixture.source()).unwrap();
    root.progress.visited = 100_000;
    root.progress.bytes_read = 1024 * 1024 * 1024;
    let mut db = store.connect().unwrap();
    let activity = super::history::begin(&db, &root.id).unwrap();
    scan::run(
        &mut db,
        &mut root,
        &store.directory,
        &Observer::default(),
        activity,
    )
    .unwrap();
    assert_eq!(root.progress.indexed, 1);
    assert!(root.progress.bytes_read > 1024 * 1024 * 1024);
}

#[test]
fn cancelling_a_large_source_transaction_preserves_its_previous_snapshot() {
    struct DuringWrite(AtomicBool, u64);
    impl ScanObserver for DuringWrite {
        fn cancellation(&self) -> Option<ScanCancellation> {
            self.0
                .load(Ordering::Relaxed)
                .then_some(ScanCancellation::Requested)
        }
        fn progress(&self, root: &IndexedRoot) {
            if root.progress.bytes_read > self.1 {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let fixture = Fixture::new();
    fixture.write(
        "notes.txt",
        "Originalneedle stays available until a complete replacement.",
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let text = format!(
        "{}\nReplacementneedle is new.",
        "Large source 京都.\n".repeat(400_000)
    );
    fixture.write("notes.txt", &text);
    let cancelled = store
        .scan(
            &root.id,
            &DuringWrite(AtomicBool::new(false), text.len() as u64),
        )
        .unwrap();
    assert_eq!(cancelled.status, IndexStatus::Cancelled);
    assert_eq!(search(&store, "Originalneedle").len(), 1);
    assert_eq!(search(&store, "Replacementneedle"), [] as [IndexPassage; 0]);
    let db = store.connect().unwrap();
    let versions: i64 = db
        .query_row("SELECT count(*) FROM index_versions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(versions, 1, "cancelled chunks and versions must roll back");
    drop(db);
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Replacementneedle").len(), 1);
    assert_eq!(search(&store, "Originalneedle"), [] as [IndexPassage; 0]);
}

#[test]
fn deep_folder_sources_do_not_require_manual_subfolder_grants() {
    let fixture = Fixture::new();
    let path = format!("{}deep.txt", "folder/".repeat(70));
    fixture.write(&path, "Deepneedle stays searchable.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.status, IndexStatus::Ready);
    assert_eq!(scanned.progress.indexed, 1);
}

#[test]
fn large_utf8_files_keep_searchable_content_beyond_attachment_limits() {
    use std::io::Write as _;
    let fixture = Fixture::new();
    let path = fixture.source().join("large.unfamiliar");
    let mut file = std::fs::File::create(&path).unwrap();
    let block = "A plain source line with Unicode 京都.\n".repeat(2048);
    while file.metadata().unwrap().len() <= sovra_core::files::MAX_FILE_BYTES {
        file.write_all(block.as_bytes()).unwrap();
    }
    file.write_all("Laterneedle: the complete source remains searchable.\n".as_bytes())
        .unwrap();
    drop(file);
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scan = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scan.status, IndexStatus::Ready);
    assert_eq!(scan.progress.indexed, 1);
    let hits = store
        .search(&IndexQuery {
            text: "Laterneedle".into(),
            roots: vec![root.id.clone()],
            path_prefix: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
        store
            .read(&hits[0].reference)
            .unwrap()
            .text
            .contains("Laterneedle")
    );
    let refreshed = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(refreshed.progress.reused, 1);
    drop(store);
    assert_eq!(
        fixture
            .store()
            .search(&IndexQuery {
                text: "Laterneedle".into(),
                roots: vec![],
                path_prefix: None,
                limit: 10
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn adding_folders_does_not_require_removing_an_existing_grant_at_sixteen() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for n in 0..18 {
        let path = fixture.source().join(format!("folder-{n}"));
        std::fs::create_dir(&path).unwrap();
        store.add_root(&path).unwrap();
    }
    assert_eq!(store.list().unwrap().len(), 18);
}

#[test]
fn shutting_down_during_a_scan_preserves_automatic_updates_for_reopening() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "source retained across restarts");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let interrupted = store
        .scan(
            &root.id,
            &Observer {
                interrupted: true,
                ..Observer::default()
            },
        )
        .unwrap();
    assert_eq!(interrupted.status, IndexStatus::Interrupted);
    assert!(fixture.store().list().unwrap()[0].auto_refresh);
    assert_eq!(search(&store, "retained").len(), 1);
}

fn search(store: &LocalFileIndex, text: &str) -> Vec<IndexPassage> {
    store
        .search(&IndexQuery {
            text: text.into(),
            roots: vec![],
            path_prefix: None,
            limit: 40,
        })
        .unwrap()
}

#[test]
fn index_survives_reopen_reuses_versions_and_retires_changed_or_removed_passages() {
    let fixture = Fixture::new();
    fixture.write(
        "notes/report.txt",
        "The orchid budget is 420 dollars.\nThe delivery is Monday.\n",
    );
    fixture.write("target/generated.txt", "generated orchid secret");
    fixture.write(".env", "orchid private token");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    assert_eq!(store.add_root(&fixture.source()).unwrap().id, root.id);
    let first = store.scan(&root.id, &Observer::default()).unwrap();
    assert!(first.auto_refresh);
    assert_eq!(first.status, IndexStatus::Ready, "{:?}", first.issues);
    assert_eq!(first.progress.indexed, 1, "{:?}", first.issues);
    let found = search(&store, "orchid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "notes/report.txt");
    assert_eq!((found[0].start_line, found[0].end_line), (1, 2));
    assert_eq!(store.read(&found[0].reference).unwrap(), found[0]);
    let reopened = fixture.store();
    reopened.set_auto_refresh(&root.id, false).unwrap();
    assert!(!fixture.store().list().unwrap()[0].auto_refresh);
    assert_eq!(reopened.list().unwrap()[0].status, IndexStatus::Ready);
    let again = reopened.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!((again.progress.indexed, again.progress.reused), (0, 1));
    fixture.write("notes/report.txt", "The tulip budget is 800 dollars.\n");
    reopened.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&reopened, "orchid"), [] as [IndexPassage; 0]);
    assert_eq!(search(&reopened, "tulip").len(), 1);
    assert!(reopened.read(&found[0].reference).is_err());
    std::fs::remove_file(fixture.source().join("notes/report.txt")).unwrap();
    assert_eq!(
        reopened
            .scan(&root.id, &Observer::default())
            .unwrap()
            .progress
            .removed,
        1
    );
    assert_eq!(search(&reopened, "tulip"), [] as [IndexPassage; 0]);
}

#[test]
fn native_watch_reports_nested_changes_and_releases_its_guard() {
    use sovra_core::file_index::IndexChanges;
    use std::sync::{Arc, mpsc};
    struct Changes(mpsc::SyncSender<Result<(), String>>);
    impl IndexChanges for Changes {
        fn changed(&self) {
            let _ = self.0.try_send(Ok(()));
        }
        fn failed(&self, message: String) {
            let _ = self.0.try_send(Err(message));
        }
    }
    let fixture = Fixture::new();
    fixture.write("notes/deep/report.txt", "old source");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let watch = store.watch(&root, Arc::new(Changes(sender))).unwrap();
    fixture.write("notes/deep/report.txt", "new orchid source");
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "orchid").len(), 1);
    drop(watch);
}

#[test]
fn cancelled_and_interrupted_scans_never_delete_unvisited_records() {
    let fixture = Fixture::new();
    for n in 0..40 {
        fixture.write(&format!("{n}.txt"), &format!("sharedneedle document {n}"));
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "sharedneedle").len(), 40);
    std::fs::remove_file(fixture.source().join("39.txt")).unwrap();
    let cancelled = store
        .scan(
            &root.id,
            &Observer {
                cancel_at: Some(32),
                ..Observer::default()
            },
        )
        .unwrap();
    assert_eq!(cancelled.status, IndexStatus::Cancelled);
    assert!(!fixture.store().list().unwrap()[0].auto_refresh);
    assert_eq!(search(&store, "sharedneedle").len(), 40);
    let mut interrupted = cancelled;
    interrupted.status = IndexStatus::Scanning;
    save_root(&store.connect().unwrap(), &interrupted).unwrap();
    fixture.store().recover().unwrap();
    assert_eq!(store.list().unwrap()[0].status, IndexStatus::Interrupted);
    let completed = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(completed.progress.removed, 1);
    assert_eq!(search(&store, "sharedneedle").len(), 39);
}

#[test]
fn searches_bind_literal_text_and_scope_filters_and_removal_preserves_sources() {
    let fixture = Fixture::new();
    fixture.write("100%/notes.txt", "sunflower budget information");
    fixture.write("100x/notes.txt", "sunflower budget information");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let query = IndexQuery {
        text: "sunflower".into(),
        roots: vec![root.id.clone()],
        path_prefix: Some("100%".into()),
        limit: 10,
    };
    let found = store.search(&query).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "100%/notes.txt");
    assert_eq!(
        search(&store, "sunflower OR absent"),
        [] as [IndexPassage; 0]
    );
    assert_eq!(
        search(&store, "\"; DROP TABLE index_roots;--"),
        [] as [IndexPassage; 0]
    );
    store.remove(&root.id).unwrap();
    assert_eq!(store.list().unwrap(), [] as [IndexedRoot; 0]);
    assert!(store.read(&found[0].reference).is_err());
    assert!(fixture.source().join("100%/notes.txt").exists());
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_expand_scope_and_renames_keep_the_file_identity() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let fixture = Fixture::new();
    fixture.write("original.txt", "zinnia identity remains");
    std::fs::write(fixture.0.join("outside.txt"), "forbiddenneedle").unwrap();
    symlink(
        fixture.0.join("outside.txt"),
        fixture.source().join("linked.txt"),
    )
    .unwrap();
    symlink(&fixture.0, fixture.source().join("escape")).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let result = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(result.progress.skipped, 2);
    assert_eq!(search(&store, "forbiddenneedle"), [] as [IndexPassage; 0]);
    let before = search(&store, "zinnia").remove(0);
    std::fs::rename(
        fixture.source().join("original.txt"),
        fixture.source().join("renamed.txt"),
    )
    .unwrap();
    let next = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(next.progress.reused, 1);
    let after = search(&store, "zinnia").remove(0);
    assert_eq!(before.reference, after.reference);
    assert_eq!(after.path, "renamed.txt");
    assert_eq!(
        std::fs::metadata(&store.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(store.directory.join("files.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn inclusion_options_persist_and_excluded_files_leave_current_search_before_rescan() {
    use sovra_core::file_index::ScanOptions;
    let fixture = Fixture::new();
    fixture.write(".notes/hidden.txt", "Hiddenneedle is explicitly included.");
    fixture.write("target/generated.txt", "Generatedneedle can be useful too.");
    fixture.write("normal.txt", "Normalneedle is always included.");
    fixture.write(
        "dev-credentials/key",
        "Protectedneedle must not be collected.",
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Hiddenneedle"), [] as [IndexPassage; 0]);
    let all = ScanOptions {
        include_hidden: true,
        include_generated: true,
    };
    store.set_scan_options(&root.id, all).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let old = search(&store, "Hiddenneedle").remove(0);
    assert_eq!(search(&store, "Generatedneedle").len(), 1);
    assert_eq!(search(&store, "Protectedneedle"), [] as [IndexPassage; 0]);
    drop(store);
    let store = fixture.store();
    assert_eq!(store.list().unwrap()[0].scan_options, all);
    store
        .set_scan_options(&root.id, ScanOptions::default())
        .unwrap();
    assert_eq!(search(&store, "Hiddenneedle"), [] as [IndexPassage; 0]);
    assert_eq!(search(&store, "Generatedneedle"), [] as [IndexPassage; 0]);
    assert!(store.read(&old.reference).is_err());
    assert_eq!(search(&store, "Normalneedle").len(), 1);
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Hiddenneedle"), [] as [IndexPassage; 0]);
}
