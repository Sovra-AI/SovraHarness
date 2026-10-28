//! A traversal failure must not prevent readable siblings from being refreshed.

use super::*;

#[cfg(unix)]
#[test]
fn inaccessible_subfolders_do_not_stop_siblings_or_retire_unvisited_sources() {
    use std::os::unix::fs::PermissionsExt as _;
    struct RestorePermissions(Vec<PathBuf>);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    let fixture = Fixture::new();
    for path in ["blocked-one/source.txt", "nested/blocked-two/source.txt"] {
        fixture.write(path, "Unreadvisitedneedle remains as an older snapshot.");
    }
    fixture.write(
        "missing.txt",
        "Missingneedle must survive an incomplete walk.",
    );
    for n in 0..24 {
        fixture.write(
            &format!("sibling-{n}/source.txt"),
            "Original sibling content.",
        );
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let first = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(first.status, IndexStatus::Ready);
    for n in 0..24 {
        fixture.write(
            &format!("sibling-{n}/source.txt"),
            &format!("Siblingfreshneedle {n}"),
        );
    }
    std::fs::remove_file(fixture.source().join("missing.txt")).unwrap();
    let blocked = RestorePermissions(vec![
        fixture.source().join("blocked-one"),
        fixture.source().join("nested/blocked-two"),
    ]);
    for path in &blocked.0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            std::fs::read_dir(path).is_err(),
            "this test needs ordinary filesystem permissions"
        );
    }
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    drop(blocked);
    assert_eq!(scanned.status, IndexStatus::Failed);
    assert_eq!(scanned.progress.failed, 2);
    assert_eq!(scanned.progress.indexed, 24);
    assert_eq!(scanned.progress.removed, 0);
    assert_eq!(scanned.completed_at, first.completed_at);
    assert_eq!(search(&store, "Siblingfreshneedle").len(), 24);
    assert_eq!(search(&store, "Unreadvisitedneedle").len(), 2);
    assert_eq!(search(&store, "Missingneedle").len(), 1);
    let page = store
        .report(&sovra_core::file_index::NoticeQuery {
            root_id: root.id.clone(),
            kind: Some(sovra_core::file_index::NoticeKind::Failed),
            contains: None,
            cursor: None,
            limit: 40,
        })
        .unwrap();
    assert_eq!(page.status, IndexStatus::Failed);
    assert_eq!(
        page.total, scanned.progress.failed,
        "each failed location is counted once"
    );
    for path in ["blocked-one", "nested/blocked-two"] {
        assert!(
            page.notices
                .iter()
                .any(|n| n.path == path && n.message.contains("permission")),
            "{page:?}"
        );
    }
    let recovered = fixture
        .store()
        .scan(&root.id, &Observer::default())
        .unwrap();
    assert_eq!(recovered.status, IndexStatus::Ready);
    assert_eq!(recovered.progress.failed, 0);
    assert_eq!(recovered.progress.removed, 1);
    assert_eq!(search(&store, "Missingneedle"), [] as [IndexPassage; 0]);
    assert_eq!(search(&store, "Unreadvisitedneedle").len(), 2);
}
