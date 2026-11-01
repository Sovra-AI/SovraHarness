//! The parallel pipeline: every discovered file settles once, is classified, and the
//! progress a person sees is consistent with what was saved.

use super::*;
use sovra_core::file_index::live::ScanPicture;
use sovra_core::file_index::ontology::{FileKind, LifeArea};

#[test]
fn a_large_folder_settles_every_file_once_and_counts_what_it_found() {
    let fixture = Fixture::new();
    for n in 0..240 {
        fixture.write(
            &format!("Taxes 2024/statement-{n}.txt"),
            &format!("Parallelneedle{n} balance"),
        );
    }
    for n in 0..60 {
        fixture.write(&format!("Holiday/notes-{n}.md"), "Beach day");
    }
    fixture.write("Holiday/plan.csv", "day,place\n1,beach\n");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    let progress = &scanned.progress;
    assert_eq!(scanned.status, IndexStatus::Ready);
    assert_eq!(progress.discovered, 301);
    assert_eq!(progress.handled, 301, "every discovered file settles once");
    assert!(progress.walk_complete);
    assert_eq!(progress.indexed, 301);
    assert_eq!(progress.kinds.get(&FileKind::Bill), Some(&240));
    assert_eq!(progress.kinds.get(&FileKind::Document), Some(&60));
    assert_eq!(progress.kinds.get(&FileKind::Spreadsheet), Some(&1));
    assert_eq!(progress.areas.get(&LifeArea::Money), Some(&240));
    assert_eq!(progress.areas.get(&LifeArea::Travel), Some(&61));
    for n in [0, 119, 239] {
        assert_eq!(search(&store, &format!("Parallelneedle{n}")).len(), 1);
    }
    let picture = ScanPicture::of(progress);
    assert_eq!(picture.percent, Some(100.0));
    assert_eq!(picture.found[0], (FileKind::Bill, 240));
}

#[test]
fn a_refresh_reuses_saved_versions_instead_of_reading_them_again() {
    let fixture = Fixture::new();
    for n in 0..50 {
        fixture.write(&format!("doc-{n}.txt"), &format!("Reuseneedle{n}"));
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    fixture.write("doc-7.txt", "Changedneedle");
    let again = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(again.progress.reused, 49);
    assert_eq!(again.progress.indexed, 1);
    assert_eq!(again.progress.handled, 50);
    assert_eq!(search(&store, "Changedneedle").len(), 1);
    assert_eq!(search(&store, "Reuseneedle7").len(), 0);
}

#[test]
fn progress_events_arrive_during_the_scan_and_never_go_backwards() {
    struct Recorder(Mutex<Vec<(u64, u64, bool)>>);
    impl ScanObserver for Recorder {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, root: &IndexedRoot) {
            let p = &root.progress;
            self.0
                .lock()
                .unwrap()
                .push((p.discovered, p.handled, p.walk_complete));
        }
    }
    let fixture = Fixture::new();
    for n in 0..200 {
        fixture.write(&format!("f-{n}.txt"), "x");
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let recorder = Recorder(Mutex::new(Vec::new()));
    store.scan(&root.id, &recorder).unwrap();
    let events = recorder.0.into_inner().unwrap();
    assert!(
        events.len() >= 3,
        "scan start, batches and the end: {events:?}"
    );
    for pair in events.windows(2) {
        assert!(pair[1].0 >= pair[0].0 && pair[1].1 >= pair[0].1, "{pair:?}");
        assert!(!pair[0].2 || pair[1].2, "walk completion never reverts");
    }
    let last = events.last().unwrap();
    assert_eq!((last.0, last.1, last.2), (200, 200, true));
}

#[cfg(unix)]
#[test]
fn a_rename_and_a_new_hard_link_in_one_scan_keep_both_files() {
    // Readers judge rename preservation from committed state; the writer may disagree once
    // an earlier file in the same batch claims the identity. Neither file may be failed,
    // and a completed walk must not retire either as removed.
    let fixture = Fixture::new();
    fixture.write("original.txt", "Hardlinkneedle shared body");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let source = fixture.source();
    std::fs::rename(source.join("original.txt"), source.join("renamed.txt")).unwrap();
    std::fs::hard_link(source.join("renamed.txt"), source.join("linked.txt")).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    let p = &scanned.progress;
    assert_eq!(scanned.status, IndexStatus::Ready);
    assert_eq!(p.failed, 0, "{:?}", scanned.issues);
    assert_eq!(p.handled, 2);
    assert_eq!(p.indexed + p.reused, 2);
    assert_eq!(
        p.removed, 0,
        "the original's record moved; nothing was removed"
    );
}

fn set_modified(path: &Path, seconds_ago: u64) {
    let when = SystemTime::now() - Duration::from_secs(seconds_ago);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn unchanged_files_are_confirmed_without_reading_them_again() {
    let fixture = Fixture::new();
    for n in 0..20 {
        fixture.write(&format!("old-{n}.txt"), &format!("Skipneedle{n} stays"));
        set_modified(&fixture.source().join(format!("old-{n}.txt")), 3_600);
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let first = store.scan(&root.id, &Observer::default()).unwrap();
    assert!(first.progress.bytes_read > 0);

    let again = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(again.progress.bytes_read, 0, "nothing re-read");
    assert_eq!(again.progress.reused, 20);
    assert_eq!(again.progress.handled, 20);
    assert_eq!(again.progress.kinds.get(&FileKind::Document), Some(&20));
    assert_eq!(search(&store, "Skipneedle7").len(), 1);

    // Same size, newer timestamp: read again and replaced.
    let edited = fixture.source().join("old-7.txt");
    std::fs::write(&edited, "Skipneedle7 moved").unwrap();
    set_modified(&edited, 60);
    let changed = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(changed.progress.indexed, 1);
    assert_eq!(changed.progress.reused, 19);
    assert_eq!(search(&store, "moved").len(), 1);

    // A different document budget is a different reading: everything is read again.
    let mut options = root.scan_options;
    options.documents = sovra_core::file_index::DocumentBudget::Large;
    store.set_scan_options(&root.id, options).unwrap();
    let rebudgeted = store.scan(&root.id, &Observer::default()).unwrap();
    assert!(rebudgeted.progress.bytes_read > 0);
    assert_eq!(rebudgeted.progress.indexed, 20);
}

#[test]
fn files_modified_in_the_same_second_as_the_last_scan_are_never_skipped() {
    // Written and scanned within the same second: the timestamp cannot prove the file
    // was not edited after it was read, so the next scan must read it again.
    let fixture = Fixture::new();
    fixture.write("fresh.txt", "Racyneedle");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let modified = std::fs::metadata(fixture.source().join("fresh.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let again = store.scan(&root.id, &Observer::default()).unwrap();
    let observed_same_second = SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age.as_secs() == 0);
    if observed_same_second {
        assert!(again.progress.bytes_read > 0, "racy file must be re-read");
    }
    assert_eq!(again.progress.reused, 1);
}

/// Manual check against a real folder: `SOVRA_LIVE_SCAN_DIR=/some/folder cargo nextest run
/// -p sovra-files live_panel_against_a_real_folder --run-ignored only --no-capture`.
/// Prints each distinct state the live panel would show.
#[test]
#[ignore = "reads a real folder chosen by the developer"]
fn live_panel_against_a_real_folder() {
    struct Printer(Mutex<Option<String>>);
    impl ScanObserver for Printer {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, root: &IndexedRoot) {
            let picture = ScanPicture::of(&root.progress);
            let found: Vec<String> = picture
                .found
                .iter()
                .take(4)
                .map(|(kind, n)| format!("{} {n}", kind.label()))
                .collect();
            let line = format!(
                "{:<22} {:>5.1}% | {} | {} | found: {}",
                picture.headline,
                picture.percent.unwrap_or(-1.0),
                picture.count_line,
                picture.time_left.as_deref().unwrap_or("-"),
                found.join(", ")
            );
            let mut last = self.0.lock().unwrap();
            if last.as_deref() != Some(&line) {
                println!("{line}");
                *last = Some(line);
            }
        }
    }
    let Some(dir) = std::env::var_os("SOVRA_LIVE_SCAN_DIR") else {
        return;
    };
    let fixture = Fixture::new();
    let store = fixture.store();
    let root = store.add_root(Path::new(&dir)).unwrap();
    let done = store.scan(&root.id, &Printer(Mutex::new(None))).unwrap();
    let p = &done.progress;
    println!(
        "status={:?} discovered={} handled={} indexed={} skipped={} failed={} elapsed={}ms",
        done.status, p.discovered, p.handled, p.indexed, p.skipped, p.failed, p.elapsed_ms
    );
    println!(
        "{}",
        sovra_core::file_index::live::found_sentence(p).unwrap_or_default()
    );
    let areas: Vec<String> = ScanPicture::of(p)
        .areas
        .iter()
        .map(|(area, n)| format!("{} {n}", area.label()))
        .collect();
    println!("about: {}", areas.join(", "));
}
