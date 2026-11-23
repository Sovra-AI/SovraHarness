//! Opt-in measurements of the real disk catalogue, FTS and vector scan.

use super::*;
use serde_json::json;
use sovra_core::file_index::semantic::{EmbeddingVector, SemanticIndex as _};
use std::time::Instant;

fn source(index: usize) -> String {
    format!(
        "Project fixturecode{index} rehearsal notes.\nVenue: Cedar Hall.\nContact: Mira.\nBring the printed agenda and review the lighting before the rehearsal.\n"
    )
}

fn vector(text: &str) -> EmbeddingVector {
    // Deterministic fixture vectors isolate index cost from model/network latency.
    // These vectors deliberately do not measure semantic relevance.
    let hash = Sha256::digest(text.as_bytes());
    let mut state = u64::from_le_bytes(hash[..8].try_into().unwrap()).max(1);
    let values = (0..768)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let bounded = u16::try_from(state & 65_535).unwrap();
            f32::from(bounded) / 65_535.0 - 0.5
        })
        .collect();
    EmbeddingVector::new(values).unwrap()
}

#[test]
#[ignore = "isolated 100/1000/10000-file benchmark; set SOVRA_FILE_BENCH_OUTPUT"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in measurement reads one explicit report destination"
)]
fn file_capture_and_search_measurements() {
    let output =
        PathBuf::from(std::env::var("SOVRA_FILE_BENCH_OUTPUT").expect("fresh report file"));
    assert!(!output.exists());
    let fixture = Fixture::new();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store
        .configure(&root.id, Some(&semantic::target()))
        .unwrap();
    let setting = store.settings().unwrap().remove(0);
    let mut inserted = 0;
    let mut report = json!({"suite":"synthetic-file-index-v1","status":"running",
        "engine":"SQLite / FTS5 / exhaustive cosine scan", "dimensions":768,
        "samples_per_size":20, "model_calls":0,
        "scope":"Synthetic short UTF-8 files; no PDF/OCR extraction or semantic relevance score.",
        "stages":[]});
    std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for size in [100, 1_000, 10_000] {
        while inserted < size {
            fixture.write(&format!("project-{inserted:05}.txt"), &source(inserted));
            inserted += 1;
        }
        let started = Instant::now();
        let scan = store.scan(&root.id, &Observer::default()).unwrap();
        let scan_us = started.elapsed().as_micros();
        assert_eq!(scan.status, IndexStatus::Ready);
        assert_eq!(scan.progress.failed, 0);
        let started = Instant::now();
        let unchanged = store.scan(&root.id, &Observer::default()).unwrap();
        let unchanged_us = started.elapsed().as_micros();
        assert_eq!(unchanged.progress.reused, u64::try_from(size).unwrap());
        assert_eq!(unchanged.progress.indexed, 0);
        let started = Instant::now();
        loop {
            let inputs = store.pending(&setting).unwrap();
            if inputs.is_empty() {
                break;
            }
            let vectors = inputs
                .iter()
                .map(|input| vector(&input.text))
                .collect::<Vec<_>>();
            store.commit(&setting, &inputs, &vectors).unwrap();
        }
        let vector_write_us = started.elapsed().as_micros();
        let mut lexical_us = Vec::new();
        let mut vector_us = Vec::new();
        for sample in 0..20 {
            let index = sample * size / 20;
            let query = IndexQuery {
                text: format!("fixturecode{index}"),
                roots: vec![root.id.clone()],
                path_prefix: None,
                limit: 4,
            };
            let expected = format!("project-{index:05}.txt");
            let started = Instant::now();
            let hits = store.search(&query).unwrap();
            lexical_us.push(started.elapsed().as_micros());
            assert_eq!(hits[0].path, expected);
            assert_eq!(store.read(&hits[0].reference).unwrap(), hits[0]);
            let embedding = vector(&hits[0].text);
            let started = Instant::now();
            let hits = store.nearest(&setting, &query, &embedding).unwrap();
            vector_us.push(started.elapsed().as_micros());
            assert_eq!(hits[0].path, expected);
        }
        report["stages"].as_array_mut().unwrap().push(json!({
            "files":size,"scan_us":scan_us,"scan":scan.progress,
            "unchanged_scan_us":unchanged_us,"unchanged_scan":unchanged.progress,
            "synthetic_vector_write_us":vector_write_us,
            "lexical_us":lexical_us,"vector_scan_us":vector_us,
            "exact_lexical_hits":20,"exact_vector_hits":20}));
        std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    let old = search(&store, "fixturecode0").remove(0);
    fixture.write(
        "project-00000.txt",
        "Changedneedle: rehearsal moves to Birch Hall.",
    );
    store.scan(&root.id, &Observer::default()).unwrap();
    assert!(store.read(&old.reference).is_err());
    assert_eq!(search(&store, "fixturecode0"), [] as [IndexPassage; 0]);
    assert_eq!(search(&store, "Changedneedle").len(), 1);
    let reopened = fixture.store();
    assert_eq!(search(&reopened, "Changedneedle").len(), 1);
    reopened.remove(&root.id).unwrap();
    assert_eq!(search(&reopened, "Changedneedle"), [] as [IndexPassage; 0]);
    report["status"] = json!("complete");
    report["lifecycle"] = json!({"replacement":true,"reopen":true,"revocation":true});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

/// A real on-disk scan, including the folder walk, decoders, writer, FTS, and history.
/// Set TMPDIR to the disk under test. Fixtures and validation are outside the timed section.
#[test]
#[ignore = "end-to-end folder indexing benchmark; optionally set SOVRA_SCAN_BENCH_FILES"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in benchmark corpus size and machine-readable results"
)]
fn end_to_end_folder_indexing() {
    use std::fmt::Write as _;
    use std::time::{Duration, SystemTime};
    fn seed(f: &Fixture, path: &str, data: &[u8]) {
        let path = f.source().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
            .unwrap();
    }
    fn measured(store: &LocalFileIndex, id: &str, stage: &str) -> IndexedRoot {
        let start = Instant::now();
        let root = store.scan(id, &Observer::default()).unwrap();
        let micros = start.elapsed().as_micros();
        assert_eq!(root.status, IndexStatus::Ready);
        assert_eq!(root.progress.failed, 0);
        println!(
            "SCAN_BENCH {}",
            json!({"stage":stage, "us":micros, "files":root.progress.handled,"new":root.progress.indexed,
            "reused":root.progress.reused, "removed":root.progress.removed, "bytes_read":root.progress.bytes_read})
        );
        root
    }
    let count = std::env::var("SOVRA_SCAN_BENCH_FILES")
        .ok()
        .map_or(10_000, |n| n.parse::<usize>().unwrap());
    assert!(count >= 100 && count.is_multiple_of(100));
    let f = Fixture::new();
    let store = f.store();
    let root = store.add_root(&f.source()).unwrap();
    let name = |i| format!("dir-{:02}/entry-{i:06}.txt", i % 100);
    for i in 0..count {
        seed(&f, &name(i), source(i).as_bytes());
    }
    assert_eq!(
        measured(&store, &root.id, "fresh_small").progress.indexed,
        count as u64
    );
    let old = search(&store, "fixturecode0").remove(0).reference;
    let unchanged = measured(&store, &root.id, "unchanged");
    assert_eq!(
        (unchanged.progress.reused, unchanged.progress.bytes_read),
        (count as u64, 0)
    );
    for i in (0..count).step_by(100) {
        seed(
            &f,
            &name(i),
            format!("Changed marker newfixture{i}").as_bytes(),
        );
    }
    std::fs::rename(f.source().join(name(1)), f.source().join("renamed.txt")).unwrap();
    std::fs::remove_file(f.source().join(name(2))).unwrap();
    seed(&f, "addition.txt", b"New additionalneedle");
    let changed = measured(&store, &root.id, "one_percent_changed");
    assert_eq!(changed.progress.indexed, (count / 100 + 1) as u64);
    assert_eq!(changed.progress.removed, 1);
    assert!(store.read(&old).is_err());
    assert_eq!(search(&store, "fixturecode1")[0].path, "renamed.txt");
    assert_eq!(search(&store, "newfixture0")[0].path, name(0));
    assert_eq!(search(&store, "fixturecode2"), [] as [IndexPassage; 0]);
    assert_eq!(search(&f.store(), "additionalneedle").len(), 1);

    for (stage, files, lines) in [
        ("line_rich_documents", 8, 20_000),
        ("streamed_text", 2, 95_000),
    ] {
        let f = Fixture::new();
        let store = f.store();
        let root = store.add_root(&f.source()).unwrap();
        for document in 0..files {
            let mut text = String::new();
            for i in 0..lines {
                if i % 128 == 0 {
                    writeln!(text, "# Section {i}").unwrap();
                }
                writeln!(
                    text,
                    "Project records sample {document} row {i} at Cedar Hall."
                )
                .unwrap();
            }
            writeln!(text, "endofdocumentmarker{document}").unwrap();
            seed(&f, &format!("record-{document}.txt"), text.as_bytes());
        }
        assert_eq!(measured(&store, &root.id, stage).progress.indexed, files);
        assert_eq!(
            search(&store, "endofdocumentmarker0")[0].path,
            "record-0.txt"
        );
        if stage == "streamed_text" {
            let refreshed = measured(&store, &root.id, "unchanged_streamed_text");
            assert_eq!(refreshed.progress.reused, files);
        }
    }
    let f = Fixture::new();
    let store = f.store();
    let root = store.add_root(&f.source()).unwrap();
    for i in 0..40 {
        let pdf = documents::document(3, |page| format!("Project catalogpdf{i} page {page}"));
        seed(&f, &format!("archive/report-{i}.pdf"), &pdf);
        seed(
            &f,
            &format!("slides-{i}.pptx"),
            &crate::format_tests::packages::presentation_with_slides(4),
        );
        seed(
            &f,
            &format!("data-{i}.csv"),
            format!("name,value\nitem{i},95\n").as_bytes(),
        );
    }
    assert_eq!(
        measured(&store, &root.id, "mixed_formats").progress.indexed,
        120
    );
    assert_eq!(
        search(&store, "catalogpdf0")[0].path,
        "archive/report-0.pdf"
    );
}

#[test]
#[ignore = "native watch registration benchmark on a 20,000-file tree"]
fn native_watch_registration_measurements() {
    struct Changes;
    impl sovra_core::file_index::IndexChanges for Changes {
        fn changed(&self) {}
        fn failed(&self, _: String) {}
    }
    let fixture = Fixture::new();
    let store = fixture.store();
    for i in 0..20_000 {
        fixture.write(
            &format!("top-{}/sub-{}/entry-{i}.txt", i % 20, (i / 20) % 10),
            "test",
        );
    }
    let root = store.add_root(&fixture.source()).unwrap();
    for _ in 0..5 {
        let start = Instant::now();
        let guard = store.watch(&root, std::sync::Arc::new(Changes)).unwrap();
        let elapsed = start.elapsed().as_micros();
        drop(guard);
        println!("WATCH_BENCH {elapsed}");
    }
}
