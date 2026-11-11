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
