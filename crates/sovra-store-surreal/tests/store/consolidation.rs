//! Consolidation must preserve different subjects, dates and correction versions.

use super::*;
use sovra_core::{FactId, MemoryCandidate, MemoryInput, MemoryListQuery};

#[tokio::test]
async fn supporting_record_browser_pages_all_reused_sources_without_unrelated_records() {
    use sovra_core::MemorySourceQuery;
    let store = open_store().await;
    let id = extract(
        &store,
        "support-0",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let first = store.memory_fact(&id).await.unwrap().source.unwrap();
    let mut expected = vec![first];
    for index in 1..4 {
        let text = format!("Lisbon is still where I live, conversation {index}.");
        let source = store
            .enqueue_memory(MemoryInput {
                key: format!("support-{index}"),
                episode: episode(&format!("conversation-{index}"), &text, 1_500),
            })
            .await
            .unwrap();
        let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
        let mut repeated = candidate("User resides in Lisbon");
        repeated.evidence = text;
        repeated.same_as = Some(id.clone());
        store
            .complete_memory(&claim, vec![repeated], "fixture", Timestamp(2_001))
            .await
            .unwrap();
        expected.push(source);
    }
    store
        .record_episode(episode(
            "unrelated",
            "Lisbon is a city I might visit",
            9_000,
        ))
        .await
        .unwrap();
    let mut query = MemorySourceQuery {
        fact: Some(id.clone()),
        limit: 2,
        ..Default::default()
    };
    let mut actual = vec![];
    for _ in 0..4 {
        let page = store.list_memory_sources(query.clone()).await.unwrap();
        assert!(
            page.sources
                .iter()
                .all(|source| source.metadata.detail_count == Some(1))
        );
        actual.extend(page.sources.into_iter().map(|s| s.id));
        query.before = page.next;
        if query.before.is_none() {
            break;
        }
    }
    assert!(query.before.is_none());
    actual.sort_by(|a, b| a.0.cmp(&b.0));
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(actual, expected);
    let filtered = store
        .list_memory_sources(MemorySourceQuery {
            fact: Some(id.clone()),
            search: "CONVERSATION 2".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(filtered.sources.len(), 1);
    let stopped = filtered.sources[0].id.clone();
    let record = store.review_memory_source(&stopped).await.unwrap();
    assert_eq!(record.metadata.detail_count, Some(1));
    let details = store
        .list_memories(MemoryListQuery {
            source: Some(stopped.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        details
            .facts
            .iter()
            .map(|fact| &fact.id)
            .collect::<Vec<_>>(),
        vec![&id]
    );
    store
        .stop_memory_source(&stopped, Timestamp(10_000))
        .await
        .unwrap();
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                source: Some(stopped.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .facts
            .len(),
        0
    );
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                source: Some(stopped.clone()),
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .facts
            .len(),
        1
    );
    let mut query = MemorySourceQuery {
        fact: Some(id),
        ..Default::default()
    };
    assert_eq!(
        store
            .list_memory_sources(query.clone())
            .await
            .unwrap()
            .sources
            .len(),
        3
    );
    query.include_stopped = true;
    let retained = store.list_memory_sources(query).await.unwrap();
    assert_eq!(retained.sources.len(), 4);
    assert!(
        retained
            .sources
            .iter()
            .any(|s| s.id == stopped && s.metadata.revoked_at.is_some())
    );
}

#[tokio::test]
async fn saved_extraction_survives_a_failed_lease_and_is_cleared_after_commit() {
    let store = open_store().await;
    store
        .enqueue_memory(memory_input("staged", "one"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let extracted = sovra_core::MemoryExtractedChunk {
        candidates: vec![candidate("User lives in Lisbon")],
        model: "fixture".into(),
    };
    store
        .save_memory_extraction(&claim, &extracted, Timestamp(2_001))
        .await
        .unwrap();
    store
        .fail_memory(&claim, "interrupted", Timestamp(2_002))
        .await
        .unwrap();
    let resumed = store
        .claim_memory(Timestamp(20_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.extracted, Some(extracted.clone()));
    assert!(
        store
            .save_memory_extraction(&claim, &extracted, Timestamp(20_001))
            .await
            .is_err()
    );
    store
        .complete_memory(&resumed, extracted.candidates, "fixture", Timestamp(20_002))
        .await
        .unwrap();
    assert!(
        store
            .claim_memory(Timestamp(30_000))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn learned_aliases_keep_withdrawn_paraphrases_suppressed_after_disk_reopen() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sovra-alias-{}-{unique}", std::process::id()));
    let config = StoreConfig::on_disk(&dir);
    let store = SurrealStore::open(config.clone()).await.unwrap();
    let id = extract(
        &store,
        "first",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let mut paraphrase = candidate("User resides in Lisbon");
    paraphrase.same_as = Some(id.clone());
    assert_eq!(
        extract(&store, "second", "I live in Lisbon", paraphrase.clone()).await,
        id
    );
    store.stop_memory(&id, Timestamp(3_000)).await.unwrap();
    drop(store);
    let store = reopen(config).await;
    paraphrase.same_as = None;
    assert_eq!(
        extract(&store, "third", "I live in Lisbon", paraphrase).await,
        id
    );
    assert!(store.memory_fact(&id).await.unwrap().stopped_at.is_some());
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .facts
            .len(),
        1
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn paraphrases_reuse_current_facts_and_keep_each_original_source_and_interpretation() {
    let store = open_store().await;
    let id = extract(
        &store,
        "first",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let original = store.memory_fact(&id).await.unwrap();
    let mut repeated = candidate("User resides in Lisbon");
    repeated.evidence = "Lisbon is where I live".into();
    repeated.same_as = Some(id.clone());
    let source = store
        .enqueue_memory(MemoryInput {
            key: "paraphrase".into(),
            episode: episode("consolidation", &repeated.evidence, 1_500),
        })
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    for _ in 0..2 {
        assert_eq!(
            store
                .complete_memory(
                    &claim,
                    vec![repeated.clone()],
                    "another-model",
                    Timestamp(2_001)
                )
                .await
                .unwrap(),
            vec![id.clone()]
        );
    }
    assert_eq!(store.memory_fact(&id).await.unwrap(), original);
    let job = store.memory_job(&source).await.unwrap();
    assert_eq!(job.candidates, vec![repeated]);
    assert_eq!(job.extraction_model.as_deref(), Some("another-model"));
    assert_eq!(
        store.episode(&source).await.unwrap().content,
        "Lisbon is where I live"
    );
    assert_eq!(
        store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap()
            .facts,
        vec![original]
    );
}

#[tokio::test]
async fn stale_or_incompatible_reuse_rolls_back_the_entire_batch() {
    let store = open_store().await;
    let id = extract(
        &store,
        "first",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    store
        .enqueue_memory(memory_input("consolidation", "next"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let mut reuse = candidate("User resides in Lisbon");
    reuse.same_as = Some(id.clone());
    let mut wrong_entity = reuse.clone();
    wrong_entity.entities = vec!["Someone else".into()];
    let mut wrong_date = reuse.clone();
    wrong_date.valid_at = Some(Timestamp(5));
    let mut missing = reuse.clone();
    missing.same_as = Some(FactId("fact:missing".into()));
    for invalid in [wrong_entity, wrong_date, missing] {
        assert!(
            store
                .complete_memory(
                    &claim,
                    vec![candidate("A distinct new claim"), invalid],
                    "fixture",
                    Timestamp(2_001)
                )
                .await
                .is_err()
        );
        assert_eq!(
            store
                .list_memories(MemoryListQuery::default())
                .await
                .unwrap()
                .facts
                .len(),
            1
        );
    }
    store.stop_memory(&id, Timestamp(3_000)).await.unwrap();
    assert!(
        store
            .complete_memory(
                &claim,
                vec![candidate("A distinct new claim"), reuse],
                "fixture",
                Timestamp(2_001)
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .facts
            .len(),
        1
    );
}

async fn extract(
    store: &SurrealStore,
    key: &str,
    source: &str,
    candidate: MemoryCandidate,
) -> FactId {
    store
        .enqueue_memory(MemoryInput {
            key: key.into(),
            episode: episode("consolidation", source, 1_000),
        })
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    store
        .complete_memory(&claim, vec![candidate], "fixture", Timestamp(2_001))
        .await
        .unwrap()
        .remove(0)
}

#[tokio::test]
async fn correcting_back_to_an_earlier_statement_creates_a_new_version() {
    let store = open_store().await;
    let first = extract(
        &store,
        "first",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let mut moved = candidate("User lives in Rome");
    moved.evidence = "I live in Rome".into();
    moved.entities = vec!["Rome".into()];
    moved.replaces = Some(first.clone());
    let second = extract(&store, "second", "I live in Rome", moved).await;
    let mut returned = candidate("User lives in Lisbon");
    returned.replaces = Some(second.clone());
    let third = extract(&store, "third", "I live in Lisbon", returned).await;
    assert_ne!(
        third, first,
        "a correction is a new version, not a duplicate of history"
    );
    let current = store.recall(RecallQuery::recent(10)).await.unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].fact.id, third);
    assert_eq!(current[0].fact.replaces.as_ref(), Some(&second));
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .facts
            .len(),
        3
    );
}

#[tokio::test]
async fn identical_wording_does_not_collapse_different_entities_or_explicit_dates() {
    let store = open_store().await;
    let mut detail = candidate("Lives in Lisbon");
    detail.entities = vec!["Mina".into()];
    detail.valid_at = Some(Timestamp(100));
    let first = extract(&store, "mina", "I live in Lisbon", detail.clone()).await;
    detail.entities = vec!["Hana".into()];
    let second = extract(&store, "hana", "I live in Lisbon", detail.clone()).await;
    assert_ne!(first, second);
    detail.valid_at = Some(Timestamp(200));
    let third = extract(&store, "hana-later", "I live in Lisbon", detail).await;
    assert_ne!(second, third);
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap().len(),
        3
    );
}

#[tokio::test]
async fn withdrawing_a_fact_also_hides_duplicate_and_later_source_evidence() {
    let store = open_store().await;
    let first = extract(
        &store,
        "first",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let second = extract(
        &store,
        "second",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    assert_eq!(first, second);
    store.stop_memory(&first, Timestamp(4_000)).await.unwrap();
    extract(
        &store,
        "third",
        "I live in Lisbon",
        candidate("User lives in Lisbon"),
    )
    .await;
    let sources = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "Lisbon".into(),
                ..Default::default()
            },
            Timestamp(9_000),
        )
        .await
        .unwrap();
    assert_eq!(sources, Vec::new());
}
