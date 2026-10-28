//! Database and property-list indexing uses folder budgets, not attachment excerpts.

use super::*;
use rusqlite::{Connection, MAIN_DB};
use sovra_core::file_index::{DocumentBudget, ExtractionState, NoticeKind, NoticeQuery};

fn database(rows: usize, width: usize) -> Vec<u8> {
    let mut db = Connection::open_in_memory().unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("CREATE TABLE records (position INTEGER, content TEXT)")
        .unwrap();
    let text = "x".repeat(width);
    for position in 0..rows {
        let content = if position + 1 == rows {
            "Lastdatabaserowneedle"
        } else {
            &text
        };
        tx.execute(
            "INSERT INTO records VALUES (?1,?2)",
            params![i64::try_from(position).unwrap(), content],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    db.serialize(MAIN_DB).unwrap().to_vec()
}

#[test]
fn database_indexing_keeps_rows_tables_and_columns_beyond_attachment_limits() {
    let mut db = Connection::open_in_memory().unwrap();
    let bytes = database(10_002, 512);
    db.deserialize_read_exact(MAIN_DB, bytes.as_slice(), bytes.len(), false)
        .unwrap();
    for index in 0..101 {
        db.execute_batch(&format!("CREATE TABLE a_{index:03} (value TEXT)"))
            .unwrap();
    }
    db.execute_batch(
        "CREATE TABLE z_last (value TEXT); INSERT INTO z_last VALUES ('Lastdatabasetableneedle')",
    )
    .unwrap();
    let columns = (0..300)
        .map(|i| format!("c{i} TEXT"))
        .collect::<Vec<_>>()
        .join(",");
    db.execute_batch(&format!(
        "CREATE TABLE wide ({columns}); INSERT INTO wide (c299) VALUES ('Lastdatabasecolumnneedle')"
    ))
    .unwrap();
    let bytes = db.serialize(MAIN_DB).unwrap();
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("large.sqlite"), &*bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.progress.failed, 0);
    for needle in [
        "Lastdatabaserowneedle",
        "Lastdatabasetableneedle",
        "Lastdatabasecolumnneedle",
    ] {
        assert_eq!(search(&store, needle).len(), 1, "{needle}");
    }
    let page = store.catalogue(&CatalogQuery::default()).unwrap();
    assert_eq!(page.files[0].extraction, ExtractionState::Text);
}

#[test]
fn database_budget_keeps_partial_rows_and_reports_how_to_read_the_rest() {
    let fixture = Fixture::new();
    let bytes = database(18_000, 1024);
    std::fs::write(fixture.source().join("budget.sqlite"), bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let mut options = root.scan_options;
    options.documents = DocumentBudget::LowerMemory;
    store.set_scan_options(&root.id, options).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    for store in [&store, &fixture.store()] {
        let page = store.catalogue(&CatalogQuery::default()).unwrap();
        assert_eq!(page.files[0].extraction, ExtractionState::PartialText);
        assert!(page.files[0].first_passage.is_some());
        assert!(
            page.files[0]
                .note
                .as_deref()
                .unwrap()
                .contains("Folder settings")
        );
        assert_eq!(search(store, "Lastdatabaserowneedle"), []);
        let report = store
            .report(&NoticeQuery {
                root_id: root.id.clone(),
                kind: Some(NoticeKind::Partial),
                contains: None,
                match_case: false,
                cursor: None,
                limit: 40,
            })
            .unwrap();
        assert_eq!(report.total, 1);
    }
    options.documents = DocumentBudget::Standard;
    store.set_scan_options(&root.id, options).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Lastdatabaserowneedle").len(), 1);
    assert_eq!(
        store.catalogue(&CatalogQuery::default()).unwrap().files[0].extraction,
        ExtractionState::Text
    );
}

#[test]
fn property_lists_keep_values_beyond_attachment_text_and_event_limits() {
    let mut values = vec![plist::Value::Boolean(true); 100_001];
    values.push(plist::Value::String("context ".repeat(600_000)));
    values.push(plist::Value::String("Finalpropertylistneedle".into()));
    let mut bytes = Vec::new();
    plist::Value::Array(values)
        .to_writer_binary(&mut bytes)
        .unwrap();
    assert!(crate::prepare_bytes("attachment", "data.plist", &bytes).is_err());
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("data.plist"), bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Finalpropertylistneedle").len(), 1);
    assert_eq!(
        store.catalogue(&CatalogQuery::default()).unwrap().files[0].extraction,
        ExtractionState::Text
    );
}

#[test]
fn cancelling_database_and_property_list_reads_preserves_previous_text() {
    struct StopAfterChecks(std::sync::atomic::AtomicUsize);
    impl ScanObserver for StopAfterChecks {
        fn cancellation(&self) -> Option<ScanCancellation> {
            (self.0.fetch_add(1, Ordering::Relaxed) >= 32).then_some(ScanCancellation::Requested)
        }
        fn progress(&self, _: &IndexedRoot) {}
    }
    let mut plist = Vec::new();
    plist::Value::Array(vec![
        plist::Value::String(
            "Replacementstructuredneedle".into()
        );
        100
    ])
    .to_writer_binary(&mut plist)
    .unwrap();
    for (name, bytes, needle) in [
        ("data.sqlite", database(100, 8), "Lastdatabaserowneedle"),
        ("data.plist", plist, "Replacementstructuredneedle"),
    ] {
        let fixture = Fixture::new();
        fixture.write(
            name,
            "Originalstructuredneedle remains until a complete replacement.",
        );
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        store.scan(&root.id, &Observer::default()).unwrap();
        let original = search(&store, "Originalstructuredneedle")
            .remove(0)
            .reference;
        std::fs::write(fixture.source().join(name), bytes).unwrap();
        let cancelled = store
            .scan(
                &root.id,
                &StopAfterChecks(std::sync::atomic::AtomicUsize::new(0)),
            )
            .unwrap();
        assert_eq!(cancelled.status, IndexStatus::Cancelled, "{name}");
        assert_eq!(cancelled.progress.failed, 0);
        assert_eq!(cancelled.progress.indexed, 0);
        assert!(store.read(&original).is_ok());
        assert_eq!(search(&store, needle), []);
        store.scan(&root.id, &Observer::default()).unwrap();
        assert_ne!(search(&store, needle), []);
    }
}

#[test]
fn attachment_database_row_caps_and_unread_cells_have_explicit_notes() {
    let attached = crate::prepare_bytes("db", "data.sqlite", &database(10_002, 1)).unwrap();
    assert!(!attached.text.contains("Lastdatabaserowneedle"));
    assert!(!attached.text.contains("Row inventory limited"));
    assert!(attached.attachment.note.unwrap().contains("10,000 rows"));
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE source (text_value TEXT, binary_value BLOB); INSERT INTO source VALUES (CAST(x'ff' AS TEXT), x'00ff')").unwrap();
    let bytes = db.serialize(MAIN_DB).unwrap();
    let indexed =
        crate::prepare_indexed_bytes("db", "data.sqlite", &bytes, DocumentBudget::Standard, None)
            .unwrap();
    assert!(indexed.partial);
    let note = indexed.content.attachment.note.unwrap();
    assert!(note.contains("invalid UTF-8"));
    assert!(note.contains("Binary database cells"));
}
