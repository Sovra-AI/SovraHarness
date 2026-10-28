use super::*;
use sovra_core::file_index::{CatalogFilter, CatalogGroupBy, CatalogQuery, ExtractionState};

#[test]
fn inventory_filters_groups_and_pages_without_treating_unread_files_as_text() {
    let fixture = Fixture::new();
    fixture.write("reports/100%_budget.txt", "orchid budget is 420\n");
    fixture.write("reports/café.md", "orchid delivery is Monday\n");
    fixture.write("reports/100x_budget.txt", "rose budget is 900\n");
    let large = fixture.source().join("archive.bin");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(sovra_core::files::MAX_FILE_BYTES + 1)
        .unwrap();
    let dated = fixture.source().join("reports/100%_budget.txt");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&dated)
        .unwrap();
    file.set_times(
        std::fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
    )
    .unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.status, IndexStatus::Ready);
    let mut query = CatalogQuery::default();
    query.filter.roots.push(root.id.clone());
    let all = store.catalogue(&query).unwrap();
    assert_eq!(all.total_files, 4);
    assert_eq!(all.unknown_sizes, 0);
    assert_eq!(all.coverage[0].status, IndexStatus::Ready);
    let unread = all.files.iter().find(|f| f.path == "archive.bin").unwrap();
    assert_eq!(unread.extraction, ExtractionState::MetadataOnly);
    assert_eq!(
        unread.byte_size,
        Some(sovra_core::files::MAX_FILE_BYTES + 1)
    );
    assert!(unread.first_passage.is_none() && unread.content_digest.is_none());
    assert!(
        unread
            .note
            .as_ref()
            .unwrap()
            .contains("document-decoder budget")
    );

    query.filter.name_contains = Some("100%_budget".into());
    query.filter.mime_prefix = Some("text/".into());
    query.filter.keywords = Some("orchid budget".into());
    query.filter.modified_after = Some(1_700_000_000);
    query.filter.modified_before = Some(1_700_000_000);
    let selected = store.catalogue(&query).unwrap();
    assert_eq!(selected.total_files, 1);
    assert_eq!(selected.files[0].path, "reports/100%_budget.txt");
    assert_eq!(selected.files[0].modified_at, Some(1_700_000_000));
    assert!(selected.files[0].observed_at.unwrap() > 1_700_000_000);
    assert!(
        store
            .read(selected.files[0].first_passage.as_ref().unwrap())
            .unwrap()
            .text
            .contains("420")
    );
    query.filter.name_contains = Some("' OR 1=1 --".into());
    assert_eq!(store.catalogue(&query).unwrap().total_files, 0);

    query.filter = CatalogFilter {
        directory: Some(String::new()),
        extraction: Some(ExtractionState::MetadataOnly),
        ..CatalogFilter::default()
    };
    let top_level = store.catalogue(&query).unwrap();
    assert_eq!(top_level.total_files, 1);
    assert_eq!(top_level.files[0].path, "archive.bin");
    query.filter.directory = Some("report".into());
    assert_eq!(store.catalogue(&query).unwrap().total_files, 0);
    query.filter.directory = Some("reports".into());
    query.filter.extraction = Some(ExtractionState::Text);
    assert_eq!(store.catalogue(&query).unwrap().total_files, 3);

    query.filter = CatalogFilter {
        roots: vec![root.id.clone()],
        ..CatalogFilter::default()
    };
    query.group_by = Some(CatalogGroupBy::Extension);
    let grouped = store.catalogue(&query).unwrap();
    assert_eq!(grouped.groups.iter().map(|g| g.files).sum::<u64>(), 4);
    assert_eq!(
        grouped
            .groups
            .iter()
            .find(|g| g.key == "txt")
            .unwrap()
            .files,
        2
    );
    assert_eq!(
        grouped
            .groups
            .iter()
            .find(|g| g.key == "bin")
            .unwrap()
            .files_with_text,
        0
    );
    query.group_by = Some(CatalogGroupBy::Directory);
    query.filter.keywords = Some("orchid".into());
    let directories = store.catalogue(&query).unwrap();
    assert_eq!(directories.groups[0].key, "reports");
    assert_eq!(directories.groups[0].files, 2);
    assert_eq!(
        directories.groups[0].root_id.as_deref(),
        Some(root.id.as_str())
    );

    query.filter.keywords = None;
    query.group_by = None;
    query.limit = 1;
    let first = store.catalogue(&query).unwrap();
    let mut paths = vec![first.files[0].path.clone()];
    query.cursor = first.next_cursor;
    let mut changed_filter = query.clone();
    changed_filter.filter.has_text = Some(true);
    assert!(store.catalogue(&changed_filter).is_err());
    while query.cursor.is_some() {
        let page = store.catalogue(&query).unwrap();
        paths.push(page.files[0].path.clone());
        query.cursor = page.next_cursor;
    }
    assert_eq!(
        paths
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    query.cursor = store.catalogue(&query).unwrap().next_cursor;
    file.set_times(
        std::fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_001)),
    )
    .unwrap();
    let changed = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(
        changed.progress.indexed, 0,
        "content was unchanged; only metadata changed"
    );
    assert!(
        store
            .catalogue(&query)
            .unwrap_err()
            .to_string()
            .contains("changed between pages")
    );
    query.cursor = None;
    query.filter.roots = vec!["ungranted".into()];
    assert!(store.catalogue(&query).is_err());
    store.remove(&root.id).unwrap();
    assert_eq!(
        store
            .catalogue(&CatalogQuery::default())
            .expect("catalogue after the folder is removed")
            .files
            .len(),
        0
    );
    assert!(dated.exists() && large.exists());
}

#[test]
fn failed_and_blank_extractions_keep_metadata_and_never_become_readable_on_refresh() {
    let fixture = Fixture::new();
    // This non-streaming decoder still reports expansion failures; large UTF-8
    // sources use a separate complete-text path and must no longer fail here.
    let mut wide = vec![0xff, 0xfe];
    wide.extend([b'x', 0].repeat(sovra_core::files::MAX_DOCUMENT_BYTES + 1));
    std::fs::write(fixture.source().join("too-much-text.txt"), wide).unwrap();
    fixture.write("blank.txt", " \t\n\u{2003}\n");
    fixture.write("empty.txt", "");
    std::fs::write(fixture.source().join("unknown.bin"), [0, 255, 128]).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    for _ in 0..2 {
        let scanned = store.scan(&root.id, &Observer::default()).unwrap();
        assert_eq!(scanned.progress.failed, 1);
        let inventory = store.catalogue(&CatalogQuery::default()).unwrap();
        assert_eq!(inventory.total_files, 4);
        for path in ["blank.txt", "empty.txt", "unknown.bin"] {
            let unread = inventory.files.iter().find(|f| f.path == path).unwrap();
            assert_eq!(unread.extraction, ExtractionState::MetadataOnly, "{path}");
            assert_eq!(unread.first_passage, None);
        }
        let failed = store
            .catalogue(&CatalogQuery {
                filter: CatalogFilter {
                    extraction: Some(ExtractionState::Failed),
                    ..CatalogFilter::default()
                },
                ..CatalogQuery::default()
            })
            .unwrap();
        assert_eq!(failed.total_files, 1);
        assert_eq!(failed.files[0].path, "too-much-text.txt");
        assert_eq!(failed.files[0].first_passage, None);
        assert!(failed.files[0].note.is_some());
    }
}

#[test]
fn metadata_only_replacement_revokes_the_previous_text_version() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "orchid source text");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let old = search(&store, "orchid").remove(0);
    std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.source().join("notes.txt"))
        .unwrap()
        .set_len(sovra_core::files::MAX_FILE_BYTES + 1)
        .unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let inventory = store.catalogue(&CatalogQuery::default()).unwrap();
    assert_eq!(inventory.files[0].file_id, old.reference.file_id);
    assert_eq!(inventory.files[0].extraction, ExtractionState::MetadataOnly);
    assert!(store.read(&old.reference).is_err());
    assert_eq!(search(&store, "orchid"), [] as [IndexPassage; 0]);
}

#[test]
fn legacy_index_migrates_atomically_without_inventing_sizes_or_losing_citations() {
    let fixture = Fixture::new();
    let store = fixture.store();
    crate::private_directory(&store.directory).unwrap();
    let db = Connection::open(store.directory.join("files.sqlite3")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    db.execute_batch(include_str!("../schema.sql")).unwrap();
    let root = IndexedRoot {
        id: "legacy".into(),
        path: fixture.source(),
        status: IndexStatus::Ready,
        progress: IndexProgress::default(),
        issues: vec![],
        completed_at: Some(1),
        auto_refresh: false,
        scan_options: sovra_core::file_index::ScanOptions::default(),
    };
    db.execute(
        "INSERT INTO index_roots(id,path,identity,details) VALUES('legacy',?1,'identity',?2)",
        params![root.path.display().to_string(), encode(&root).unwrap()],
    )
    .unwrap();
    db.execute_batch("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES('file','legacy','notes/old.txt','identity','v1',1,1);
        INSERT INTO index_versions(id,file_id,digest,mime,observed,modified) VALUES('v1','file','digest','text/plain',1,1);
        INSERT INTO index_chunks(id,body) VALUES('chunk','orchid source');
        INSERT INTO index_version_chunks(version_id,ordinal,chunk_id,start_line,end_line) VALUES('v1',0,'chunk',1,1);").unwrap();
    drop(db);
    let paths = [store.directory.clone(), store.directory.clone()];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = paths.map(|directory| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            LocalFileIndex::new(directory).catalogue(&CatalogQuery::default())
        })
    });
    for thread in threads {
        let page = thread.join().unwrap().unwrap();
        assert_eq!(page.total_files, 1);
        assert_eq!(page.unknown_sizes, 1);
        assert_eq!(page.files[0].byte_size, None);
        assert_eq!(page.files[0].path, "notes/old.txt");
        let history = store
            .history(&sovra_core::file_index::FileHistoryQuery {
                root_id: "legacy".into(),
                file_id: Some("file".into()),
                ..Default::default()
            })
            .unwrap();
        let imported = &history.observations[0];
        assert_eq!(
            imported.basis.tier,
            sovra_core::file_index::EvidenceTier::LegacyImport
        );
        assert_eq!(imported.basis.confidence, None);
        assert_eq!(imported.basis.observed_at, None);
        assert_eq!(imported.basis.valid_time, None);
        assert!(imported.activity.is_none());
        assert!(
            imported
                .representation
                .as_ref()
                .unwrap()
                .extractor_revision
                .is_none()
        );
        assert_eq!(
            store
                .read(page.files[0].first_passage.as_ref().unwrap())
                .unwrap()
                .text,
            "orchid source"
        );
    }
    assert_eq!(
        store
            .connect()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}
