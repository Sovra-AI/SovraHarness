//! Public-corpus diagnostic of the real source admission and lexical retrieval paths.
//! Labels only score returned IDs; neither answers nor labels enter the index.

use super::*;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sovra_core::{MemoryContext, MemoryInput, MemorySearch};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Instant;

fn recall(expected: &BTreeSet<String>, returned: &[String], k: usize) -> Option<f64> {
    if expected.is_empty() {
        return None;
    }
    let found: BTreeSet<_> = returned.iter().take(k).collect();
    #[expect(clippy::cast_precision_loss, reason = "bounded corpus counts")]
    Some(expected.iter().filter(|id| found.contains(id)).count() as f64 / expected.len() as f64)
}

#[test]
fn recall_counts_distinct_evidence_and_leaves_unsupported_cases_unscored() {
    let expected = BTreeSet::from(["a".into(), "b".into()]);
    let returned = vec!["a".into(), "a".into(), "c".into(), "b".into()];
    assert_eq!(recall(&expected, &returned, 3), Some(0.5));
    assert_eq!(recall(&expected, &returned, 4), Some(1.0));
    assert_eq!(recall(&BTreeSet::new(), &returned, 4), None);
}

#[tokio::test]
#[ignore = "public corpus diagnostic; prepare with tests/evaluations/prepare_longmemeval.py"]
#[expect(
    clippy::disallowed_methods,
    reason = "explicit opt-in fixture paths, no app configuration"
)]
async fn longmemeval_source_retrieval_measurements() {
    let input = std::env::var("SOVRA_RETRIEVAL_EVAL_INPUT").expect("prepared fixture path");
    let output = std::env::var("SOVRA_RETRIEVAL_EVAL_OUTPUT").expect("fresh report file");
    assert!(!std::path::Path::new(&output).exists());
    let bytes = std::fs::read(input).unwrap();
    let mut fixture_hash = String::new();
    for byte in Sha256::digest(&bytes) {
        write!(fixture_hash, "{byte:02x}").unwrap();
    }
    let fixture: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fixture["suite"], "longmemeval-s-user-source-v1");
    assert_eq!(
        fixture["upstream_sha256"],
        "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442"
    );
    let mut report = json!({"suite":fixture["suite"], "status":"running",
        "upstream":fixture["upstream"], "revision":fixture["revision"],
        "upstream_sha256":fixture["upstream_sha256"],
        "fixture_sha256":fixture_hash,
        "selection":fixture["selection"], "scope":fixture["scope"],
        "engine":"SurrealDB / mem://", "model_calls":0,
        "limitations":["Source-ID recall does not establish that a bounded quote contains the answer.",
            "Assistant evidence is excluded because imported transcripts have no certified input lineage.",
            "Empty user turns cannot be admitted; their source keys are reported separately and must not be labeled evidence.",
            "Abstention cases report retrieval counts, not answer correctness.",
            "All upstream history is retained, including sessions dated after the question; no inferred temporal filter is applied.",
            "Latency excludes runtime scheduling, disk IO, embeddings and generation."],
        "cases":[]});
    std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let mut ids = BTreeMap::new();
        let mut empty_sources = Vec::new();
        let started = Instant::now();
        for source in case["sources"].as_array().unwrap() {
            let key = source["key"].as_str().unwrap();
            if source["text"].as_str().unwrap().trim().is_empty() {
                empty_sources.push(key);
                continue;
            }
            let id = store
                .enqueue_memory(MemoryInput {
                    key: key.into(),
                    episode: NewEpisode {
                        session_id: source["session"].as_str().unwrap().into(),
                        kind: EpisodeKind::UserMessage,
                        content: source["text"].as_str().unwrap().into(),
                        occurred_at: Timestamp(source["time"].as_i64().unwrap()),
                    },
                })
                .await
                .unwrap();
            ids.insert(id.0, key.to_owned());
        }
        let capture_us = started.elapsed().as_micros();
        let expected: BTreeSet<String> = case["expected_source_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        let query = MemorySearch {
            text: case["query"].as_str().unwrap().into(),
            ..Default::default()
        };
        // Import time is now; historical recorded-time filters would hide the freshly
        // admitted benchmark corpus. Event dates remain the upstream session dates.
        let started = Instant::now();
        let hits = store
            .search_memory_sources(query.clone(), Timestamp(i64::MAX))
            .await
            .unwrap();
        let search_us = started.elapsed().as_micros();
        let returned: Vec<String> = hits
            .iter()
            .map(|hit| ids[&hit.source_id.0].clone())
            .collect();
        let mut context = MemoryContext::default();
        for hit in hits {
            context.push_source(hit);
        }
        let injected: Vec<String> = context
            .sources
            .iter()
            .map(|hit| ids[&hit.source_id.0].clone())
            .collect();
        assert!(expected.iter().all(|key| ids.values().any(|id| id == key)));
        let abstention = case["category"] == "abstention";
        let scores = if abstention {
            json!(null)
        } else {
            json!({
            "recall_at_1":recall(&expected,&returned,1),
            "recall_at_4":recall(&expected,&returned,4),
            "recall_at_12":recall(&expected,&returned,12),
            "context_source_recall":recall(&expected,&injected,injected.len())})
        };
        report["cases"].as_array_mut().unwrap().push(json!({
            "id":case["id"], "category":case["category"],
            "sources":ids.len(), "capture_us":capture_us, "search_us":search_us,
            "expected_sources":expected.len(),
            "excluded_empty_source_keys":empty_sources,
            "excluded_assistant_evidence":case["excluded_assistant_evidence"],
            "returned_source_keys":returned, "expected_source_keys":expected,
            "context_source_keys":injected,
            "sources_after_question":case["sources"].as_array().unwrap().iter()
                .filter(|source| source["time"].as_i64().unwrap() > case["time"].as_i64().unwrap()).count(),
            "search_terms":query.terms(), "scores":scores}));
        std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    report["status"] = json!("complete");
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
