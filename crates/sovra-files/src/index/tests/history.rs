use super::*;
use sovra_core::file_index::{EvidenceTier, FileHistoryQuery, ObservedPresence};

fn history(
    store: &LocalFileIndex,
    root: &IndexedRoot,
    path: &str,
) -> sovra_core::file_index::FileHistory {
    store
        .history(&FileHistoryQuery {
            root_id: root.id.clone(),
            path: Some(path.into()),
            ..FileHistoryQuery::default()
        })
        .unwrap()
}

#[test]
fn history_preserves_renames_versions_absence_and_scoped_historical_reads() {
    let fixture = Fixture::new();
    fixture.write(
        "original.txt",
        &"Orchid original source paragraph.\n".repeat(1000),
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let first = history(&store, &root, "original.txt");
    assert_eq!(first.observations.len(), 1);
    assert_eq!(first.coverage.status, IndexStatus::Ready);
    let original = first.observations[0].clone();
    assert_eq!(original.basis.tier, EvidenceTier::FilesystemObservation);
    assert_eq!(
        original.basis.confidence.unwrap().value().to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(original.basis.valid_time, None);
    assert_eq!(
        original.activity.as_ref().unwrap().status,
        IndexStatus::Ready
    );
    let representation = original.representation.as_ref().unwrap();
    assert_eq!(
        representation.extractor_revision.as_deref(),
        Some("extract-v2")
    );
    assert_eq!(representation.generated_by, original.basis.activity_id);
    let reference = representation.first_passage.clone().unwrap();
    assert!(store.read_history(&reference).unwrap().next.is_some());

    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(history(&store, &root, "original.txt").observations.len(), 1);
    fixture.write("original.txt", "Revised orchid source.");
    store.scan(&root.id, &Observer::default()).unwrap();
    std::fs::rename(
        fixture.source().join("original.txt"),
        fixture.source().join("renamed.txt"),
    )
    .unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let renamed = history(&store, &root, "original.txt");
    assert_eq!(renamed.file_id, first.file_id);
    assert_eq!(renamed.current_path, "renamed.txt");
    assert_eq!(renamed.observations.len(), 3);
    assert_eq!(renamed.observations[0].physical_id, original.physical_id);
    assert_eq!(renamed.observations[1].replaces, Some(original.sequence));

    let mut query = FileHistoryQuery {
        root_id: root.id.clone(),
        file_id: Some(first.file_id),
        limit: 1,
        ..FileHistoryQuery::default()
    };
    let page = store.history(&query).unwrap();
    query.before = page.next_before;

    std::fs::remove_file(fixture.source().join("renamed.txt")).unwrap();
    let cancelled = Observer {
        cancel: AtomicBool::new(true),
        ..Observer::default()
    };
    store.scan(&root.id, &cancelled).unwrap();
    assert_eq!(
        history(&store, &root, "original.txt").coverage.status,
        IndexStatus::Cancelled
    );
    assert_eq!(history(&store, &root, "original.txt").observations.len(), 3);
    store.scan(&root.id, &Observer::default()).unwrap();
    let absent = history(&store, &root, "renamed.txt");
    assert!(!absent.currently_present);
    assert_eq!(
        absent.observations[0].presence,
        ObservedPresence::NotObserved
    );
    assert_eq!(
        absent.observations[0].activity.as_ref().unwrap().status,
        IndexStatus::Ready
    );
    let older = store.history(&query).unwrap();
    assert_eq!(
        older.observations[0].sequence,
        renamed.observations[1].sequence
    );
    let historical = store.read_history(&reference).unwrap();
    assert_eq!(historical.passage.path, "original.txt");
    assert!(historical.passage.text.contains("original source"));
    assert!(
        store.read(&historical.passage.reference).is_err(),
        "current reads must reject historical snapshots"
    );
    let mut wrong_scope = reference.clone();
    wrong_scope.root_id = "ungranted".into();
    assert!(store.read_history(&wrong_scope).is_err());
    let mut wrong_file = reference.clone();
    wrong_file.file_id = "another-file".into();
    assert!(store.read_history(&wrong_file).is_err());
    let mut wrong_passage = reference.clone();
    wrong_passage.ordinal = u32::MAX;
    assert!(store.read_history(&wrong_passage).is_err());
    store.remove(&root.id).unwrap();
    assert!(store.history(&query).is_err());
    assert!(store.read_history(&reference).is_err());
}

#[test]
fn incomplete_scans_are_explicit_in_lineage_and_recovery_finishes_open_activities() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", "Observed source.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let db = store.connect().unwrap();
    let id = super::super::history::begin(&db, &root.id).unwrap();
    db.execute(
        "UPDATE index_files SET detail='interrupted observation' WHERE root_id=?1",
        [&root.id],
    )
    .unwrap();
    let file_id = history(&store, &root, "notes.txt").file_id;
    super::super::history::observe(&db, &file_id, id).unwrap();
    drop(db);
    store.recover().unwrap();
    let recovered = history(&store, &root, "notes.txt");
    let activity = recovered.observations[0].activity.as_ref().unwrap();
    assert_eq!(activity.status, IndexStatus::Interrupted);
    assert!(activity.finished_at.is_some());
    assert_eq!(
        recovered.observations[0].note.as_deref(),
        Some("interrupted observation")
    );
}
