//! Integration tests against a real embedded engine (`mem://`), exercising the port
//! exactly as the runtime will.

use sovra_core::{
    Embedding, EpisodeKind, MemoryError, MemoryStore, NewEpisode, NewFact, RecallQuery, Timestamp,
};
use sovra_store_surreal::{StoreConfig, SurrealStore};

const DIM: usize = 4;

async fn open_store() -> SurrealStore {
    let mut config = StoreConfig::in_memory();
    config.embedding_dimension = DIM;
    SurrealStore::open(config)
        .await
        .expect("open in-memory store")
}

fn embedding(vector: [f32; DIM]) -> Embedding {
    Embedding {
        model: "test-embedder".into(),
        vector: vector.to_vec(),
    }
}

fn episode(session: &str, content: &str, at: i64) -> NewEpisode {
    NewEpisode {
        session_id: session.into(),
        kind: EpisodeKind::UserMessage,
        content: content.into(),
        occurred_at: Timestamp(at),
    }
}

fn fact(content: &str, entities: &[&str], vector: Option<[f32; DIM]>) -> NewFact {
    NewFact {
        content: content.into(),
        entities: entities.iter().map(|&e| e.to_owned()).collect(),
        embedding: vector.map(embedding),
        source: None,
        valid_at: None,
        replaces: None,
    }
}

#[tokio::test]
async fn schema_is_idempotent_across_reopens() {
    let store = open_store().await;
    store.health().await.expect("healthy");
    // Second open against a fresh mem engine re-applies the same OVERWRITE schema.
    let again = open_store().await;
    again.health().await.expect("still healthy");
}

#[tokio::test]
async fn episodes_round_trip_newest_first() {
    let store = open_store().await;
    for (content, at) in [("first", 1_000), ("second", 2_000), ("third", 3_000)] {
        store
            .record_episode(episode("s1", content, at))
            .await
            .expect("record");
    }
    store
        .record_episode(episode("other", "noise", 4_000))
        .await
        .expect("record");

    let recent = store.recent_episodes("s1", 2).await.expect("recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].content, "third");
    assert_eq!(recent[1].content, "second");
    assert_eq!(recent[0].kind, EpisodeKind::UserMessage);
    assert_eq!(recent[0].session_id, "s1");
}

#[tokio::test]
async fn facts_save_and_recall_by_recency() {
    let store = open_store().await;
    store
        .save_fact(fact("Shaun prefers uv", &["Shaun"], None))
        .await
        .expect("save");
    store
        .save_fact(fact("Sovra targets desktop", &["Sovra"], None))
        .await
        .expect("save");

    let recalled = store.recall(RecallQuery::recent(10)).await.expect("recall");
    assert_eq!(recalled.len(), 2);
    assert_eq!(
        recalled[0].fact.content, "Sovra targets desktop",
        "newest first"
    );
    assert!(recalled.iter().all(|r| r.fact.is_current()));
}

#[tokio::test]
async fn entity_filter_restricts_recall() {
    let store = open_store().await;
    store
        .save_fact(fact("Shaun prefers uv", &["Shaun"], None))
        .await
        .expect("save");
    store
        .save_fact(fact("Sovra targets desktop", &["Sovra"], None))
        .await
        .expect("save");

    let mut query = RecallQuery::recent(10);
    query.entity = Some("Shaun".into());
    let recalled = store.recall(query).await.expect("recall");
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].fact.content, "Shaun prefers uv");
}

#[tokio::test]
async fn semantic_recall_ranks_nearest_vector_first() {
    let store = open_store().await;
    store
        .save_fact(fact(
            "about python tooling",
            &[],
            Some([1.0, 0.0, 0.0, 0.0]),
        ))
        .await
        .expect("save");
    store
        .save_fact(fact("about desktop apps", &[], Some([0.0, 1.0, 0.0, 0.0])))
        .await
        .expect("save");

    let query = RecallQuery::semantic(embedding([0.9, 0.1, 0.0, 0.0]), "python", 2);
    let recalled = store.recall(query).await.expect("recall");
    assert_eq!(
        recalled.first().map(|r| r.fact.content.as_str()),
        Some("about python tooling")
    );
}

#[tokio::test]
async fn superseding_invalidates_the_replaced_fact() {
    let store = open_store().await;
    let old = store
        .save_fact(fact("Shaun uses pip", &["Shaun"], None))
        .await
        .expect("save");

    let mut replacement = fact("Shaun uses uv", &["Shaun"], None);
    replacement.replaces = Some(old);
    store.save_fact(replacement).await.expect("supersede");

    let current = store.recall(RecallQuery::recent(10)).await.expect("recall");
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].fact.content, "Shaun uses uv");

    let mut history = RecallQuery::recent(10);
    history.include_expired = true;
    let all = store.recall(history).await.expect("history");
    assert_eq!(all.len(), 2);
    let superseded = all
        .iter()
        .find(|r| r.fact.content == "Shaun uses pip")
        .expect("kept");
    assert!(!superseded.fact.is_current());
    assert!(superseded.fact.expired_at.is_some());
}

#[tokio::test]
async fn invalidate_marks_without_deleting() {
    let store = open_store().await;
    let id = store
        .save_fact(fact("temporary belief", &[], None))
        .await
        .expect("save");
    store
        .invalidate_fact(&id, Timestamp(9_000))
        .await
        .expect("invalidate");

    let current = store.recall(RecallQuery::recent(10)).await.expect("recall");
    assert_eq!(
        current,
        vec![],
        "invalidated fact must not be recalled as current"
    );
    let mut history = RecallQuery::recent(10);
    history.include_expired = true;
    assert_eq!(store.recall(history).await.expect("history").len(), 1);
}

#[tokio::test]
async fn wrong_dimension_embeddings_are_rejected() {
    let store = open_store().await;
    let bad = NewFact {
        embedding: Some(Embedding {
            model: "test-embedder".into(),
            vector: vec![0.0; DIM + 1],
        }),
        ..fact("bad", &[], None)
    };
    match store.save_fact(bad).await {
        Err(MemoryError::EmbeddingMismatch { dimension, .. }) => assert_eq!(dimension, DIM),
        other => panic!("expected EmbeddingMismatch, got {other:?}"),
    }
}
