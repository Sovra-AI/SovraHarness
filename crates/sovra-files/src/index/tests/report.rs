use super::*;
use sovra_core::file_index::{NoticeKind, NoticeQuery};

fn query(root_id: &str) -> NoticeQuery {
    NoticeQuery {
        root_id: root_id.into(),
        kind: None,
        contains: None,
        match_case: false,
        cursor: None,
        limit: 17,
    }
}

#[test]
fn report_filters_ignore_unicode_casing_and_normalization_by_default() {
    let fixture = Fixture::new();
    fixture.write(".Straße.Café", "hidden");
    fixture.write(".STRASSE.cafe\u{301}", "hidden");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let mut query = query(&root.id);
    query.contains = Some("STRASSE.CAFÉ".into());
    query.limit = 1;
    let first = store.report(&query).unwrap();
    assert_eq!(first.total, 2);
    query.cursor = first.next_cursor;
    assert!(query.cursor.is_some());
    let second = store.report(&query).unwrap();
    assert_eq!(second.total, 2);
    assert_ne!(first.notices[0].path, second.notices[0].path);
    query.match_case = true;
    assert!(store.report(&query).is_err());
    query.cursor = None;
    assert_eq!(store.report(&query).unwrap().total, 0);
    query.contains = Some("Straße.Café".into());
    assert_eq!(store.report(&query).unwrap().total, 1);
    // The same matching mode applies to reasons, not just filenames.
    query.contains = Some(first.notices[0].message.to_uppercase());
    assert_eq!(store.report(&query).unwrap().total, 0);
    query.match_case = false;
    assert_eq!(store.report(&query).unwrap().total, 2);
    query.contains = Some("%_".into());
    assert_eq!(store.report(&query).unwrap().total, 0);
}

#[cfg(unix)]
#[test]
fn report_paths_distinguish_non_unicode_filenames_without_lossy_replacement() {
    use std::os::unix::ffi::OsStringExt as _;
    let fixture = Fixture::new();
    for byte in [0xfe, 0xff] {
        std::fs::write(
            fixture
                .source()
                .join(std::ffi::OsString::from_vec(vec![b'n', byte])),
            b"source",
        )
        .unwrap();
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let page = store.report(&query(&root.id)).unwrap();
    assert_eq!(page.total, 2);
    let paths: std::collections::BTreeSet<_> =
        page.notices.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(paths, ["n\\xfe", "n\\xff"].into_iter().collect());
    assert!(
        page.notices
            .iter()
            .all(|n| n.kind == NoticeKind::Skipped && n.message.contains("not valid Unicode"))
    );
}

#[test]
fn complete_scan_report_survives_reopen_and_pages_past_fifty_examples() {
    let fixture = Fixture::new();
    for n in 0..93 {
        fixture.write(&format!(".hidden-{n}"), "hidden source");
    }
    fixture.write("blank.txt", " \n");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.issues.len(), 50);
    drop(store);
    let store = fixture.store();
    let mut request = query(&root.id);
    let first = store.report(&request).unwrap();
    assert_eq!(first.total, 94);
    assert!(first.counts.contains(&(NoticeKind::Skipped, 93)));
    assert!(first.counts.contains(&(NoticeKind::Unreadable, 1)));
    assert_eq!(first.status, IndexStatus::Ready);
    let stale = first.next_cursor;
    let mut paths = std::collections::BTreeSet::new();
    loop {
        let page = store.report(&request).unwrap();
        assert_eq!(page.total, 94);
        assert!(page.notices.len() <= 17);
        for notice in page.notices {
            assert!(paths.insert(notice.path));
        }
        request.cursor = page.next_cursor;
        if request.cursor.is_none() {
            break;
        }
    }
    assert_eq!(paths.len(), 94);
    assert!(paths.contains(".hidden-92") && paths.contains("blank.txt"));
    request.kind = Some(NoticeKind::Unreadable);
    let unread = store.report(&request).unwrap();
    assert_eq!(unread.total, 1);
    assert_eq!(unread.notices[0].path, "blank.txt");
    request.contains = Some("' OR 1=1".into());
    assert_eq!(store.report(&request).unwrap().total, 0);
    request = query(&root.id);
    request.cursor = stale;
    request.contains = Some("hidden".into());
    assert!(store.report(&request).is_err());
    store.scan(&root.id, &Observer::default()).unwrap();
    request.contains = None;
    assert!(store.report(&request).unwrap_err().0.contains("Reload"));
    assert_eq!(
        store.report(&query(&root.id)).unwrap().total,
        94,
        "new scans replace old diagnostic rows"
    );
    store.remove(&root.id).unwrap();
    assert!(store.report(&query(&root.id)).is_err());
}

#[test]
fn incomplete_and_failed_reports_do_not_claim_full_folder_coverage() {
    let fixture = Fixture::new();
    for n in 0..100 {
        fixture.write(&format!(".hidden-{n}"), "hidden");
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store
        .scan(
            &root.id,
            &Observer {
                cancel_at: Some(32),
                ..Observer::default()
            },
        )
        .unwrap();
    let partial = store.report(&query(&root.id)).unwrap();
    assert_eq!(partial.status, IndexStatus::Cancelled);
    assert!(partial.total > 0 && partial.total < 100);
    std::fs::rename(fixture.source(), fixture.0.join("old-source")).unwrap();
    std::fs::create_dir(fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let failed = store.report(&query(&root.id)).unwrap();
    assert_eq!(failed.status, IndexStatus::Failed);
    assert_eq!(failed.total, 1);
    assert_eq!(failed.notices[0].kind, NoticeKind::Failed);
    assert!(failed.notices[0].message.contains("replaced or moved"));
}

#[test]
fn a_report_opened_during_scanning_keeps_its_snapshot_and_announces_newer_entries() {
    struct DuringScan<'a> {
        store: &'a LocalFileIndex,
        captured: Mutex<Option<sovra_core::file_index::NoticePage>>,
    }
    impl ScanObserver for DuringScan<'_> {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, root: &IndexedRoot) {
            let mut captured = self.captured.lock().unwrap();
            if captured.is_none()
                && root.status == IndexStatus::Scanning
                && root.progress.skipped >= 32
            {
                *captured = Some(self.store.report(&query(&root.id)).unwrap());
            }
        }
    }
    let fixture = Fixture::new();
    for n in 0..100 {
        fixture.write(&format!(".entry-{n}"), "hidden");
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let observer = DuringScan {
        store: &store,
        captured: Mutex::new(None),
    };
    store.scan(&root.id, &observer).unwrap();
    let captured = observer.captured.into_inner().unwrap().unwrap();
    assert_eq!(captured.status, IndexStatus::Scanning);
    assert_eq!(captured.total, 32);
    let next = store
        .report(&NoticeQuery {
            cursor: captured.next_cursor,
            ..query(&root.id)
        })
        .unwrap();
    assert_eq!(next.total, 32);
    assert_eq!(next.notices.len(), 15);
    assert!(next.has_newer && next.next_cursor.is_none());
    let latest = store.report(&query(&root.id)).unwrap();
    assert_eq!(latest.total, 100);
    assert!(!latest.has_newer);
}

#[test]
fn schema_eight_migration_keeps_sources_and_distinguishes_missing_old_reports() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "Sourcemarker remains readable.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let reference = search(&store, "Sourcemarker")[0].reference.clone();
    let db = store.connect().unwrap();
    db.execute_batch(
        "ALTER TABLE index_versions DROP COLUMN partial; DROP TABLE index_scan_notices; DROP TABLE index_scan_reports; PRAGMA user_version=8;",
    )
    .unwrap();
    drop(db);
    drop(store);
    let store = fixture.store();
    assert!(
        store
            .read(&reference)
            .unwrap()
            .text
            .contains("Sourcemarker")
    );
    assert_eq!(store.report(&query(&root.id)).unwrap().scan_id, None);
    store.scan(&root.id, &Observer::default()).unwrap();
    let page = store.report(&query(&root.id)).unwrap();
    assert!(page.scan_id.is_some());
    assert_eq!(page.total, 0);
}

#[test]
fn schema_nine_migration_preserves_saved_text_and_existing_report_pages() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "Migrationneedle remains readable.");
    fixture.write(".hidden.txt", "Excluded source.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let reference = search(&store, "Migrationneedle")[0].reference.clone();
    let before = store.report(&query(&root.id)).unwrap();
    let db = store.connect().unwrap();
    db.execute_batch("ALTER TABLE index_versions DROP COLUMN partial;
        CREATE TABLE old_notices (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            root_id TEXT NOT NULL REFERENCES index_scan_reports(root_id) ON DELETE CASCADE,
            kind INTEGER NOT NULL CHECK(kind IN (0,1,2)), path TEXT NOT NULL, message TEXT NOT NULL);
        INSERT INTO old_notices SELECT * FROM index_scan_notices;
        DROP TABLE index_scan_notices;
        ALTER TABLE old_notices RENAME TO index_scan_notices;
        CREATE INDEX scan_notice_root ON index_scan_notices(root_id,sequence);
        CREATE INDEX scan_notice_kind ON index_scan_notices(root_id,kind,sequence);
        PRAGMA user_version=9;").unwrap();
    drop(db);
    drop(store);
    let reopened = fixture.store();
    assert_eq!(reopened.report(&query(&root.id)).unwrap(), before);
    assert!(
        reopened
            .read(&reference)
            .unwrap()
            .text
            .contains("Migrationneedle")
    );
    assert_eq!(
        reopened
            .scan(&root.id, &Observer::default())
            .unwrap()
            .progress
            .reused,
        1
    );
    let db = reopened.connect().unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert!(db.execute("INSERT INTO index_scan_notices(root_id,kind,path,message) VALUES(?1,3,'partial.pptx','Some slides were not read.')", [&root.id]).is_ok());
    let page = reopened
        .report(&NoticeQuery {
            kind: Some(NoticeKind::Partial),
            ..query(&root.id)
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.notices[0].kind, NoticeKind::Partial);
}
