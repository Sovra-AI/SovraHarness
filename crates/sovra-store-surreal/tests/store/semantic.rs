//! Real-engine consent generations, dynamic dimensions and bounded semantic recall.

use super::*;
use sovra_core::{
    FactId, MemorySearch, MemorySemanticSetting, MemoryVector,
    file_index::semantic::EmbeddingVector,
    models::{ModelConnection, ModelDestination, ModelInfo, ProviderKind},
};

fn target(model: &str) -> ModelDestination {
    let mut connection = ModelConnection::new("search", ProviderKind::OpenAiCompatible);
    connection.base_url = "http://localhost:1234/v1".into();
    ModelDestination::for_embeddings(
        &connection,
        &ModelInfo {
            id: model.into(),
            output_modalities: vec!["embeddings".into()],
            ..Default::default()
        },
    )
    .unwrap()
}

async fn saved(store: &SurrealStore, text: &str) -> FactId {
    let source = store
        .record_episode(episode("chat", text, 1))
        .await
        .unwrap();
    let mut input = fact(text, &[], None);
    input.source = Some(source);
    store.save_fact(input).await.unwrap()
}

fn vector(id: &FactId, values: &[f32]) -> MemoryVector {
    MemoryVector {
        fact_id: id.clone(),
        vector: EmbeddingVector::new(values.to_vec()).unwrap(),
    }
}

async fn search(
    store: &SurrealStore,
    setting: &MemorySemanticSetting,
    values: &[f32],
) -> Vec<sovra_core::MemorySemanticHit> {
    store
        .semantic_memories(
            setting,
            &EmbeddingVector::new(values.to_vec()).unwrap(),
            MemorySearch {
                text: "Which beverage do I like?".into(),
                ..Default::default()
            },
            Timestamp(i64::MAX),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn semantic_memory_requires_consent_and_pins_the_first_batch_dimension() {
    let store = open_store().await;
    assert!(store.memory_semantics().await.unwrap().setting.is_none());
    let tea = saved(&store, "I prefer jasmine tea").await;
    let other = saved(&store, "My bicycle is red").await;
    let unsupported = store
        .save_fact(fact("No user source", &[], None))
        .await
        .unwrap();
    let setting = store
        .configure_memory_semantics(Some(target("small-search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    let pending = store
        .memory_embedding_inputs(&setting, &[tea.clone(), other.clone(), unsupported])
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending
            .iter()
            .any(|input| input.text == "I prefer jasmine tea")
    );
    assert!(
        store
            .memory_embedding_inputs(&setting, &vec![tea.clone(); 9])
            .await
            .is_err()
    );
    let policy = store.memory_policy().await.unwrap();
    // These two-dimensional vectors are independent of the legacy four-dimensional index.
    store
        .commit_memory_vectors(
            &setting,
            policy,
            vec![vector(&tea, &[1., 0.]), vector(&other, &[0., 1.])],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .memory_semantics()
            .await
            .unwrap()
            .setting
            .unwrap()
            .dimension,
        Some(2)
    );
    assert_eq!(
        store
            .memory_embedding_inputs(&setting, std::slice::from_ref(&tea))
            .await
            .unwrap()
            .len(),
        0
    );
    let found = search(&store, &setting, &[1., 0.]).await;
    assert_eq!(
        found.len(),
        1,
        "orthogonal distractors do not pass the candidate floor"
    );
    assert_eq!(found[0].fact.id, tea);
    assert!(found[0].similarity > 0.99);
    store
        .commit_memory_vectors(&setting, policy, vec![vector(&tea, &[1., 0.])])
        .await
        .unwrap();
    assert!(
        store
            .commit_memory_vectors(&setting, policy, vec![vector(&tea, &[0., 1.])])
            .await
            .is_err()
    );
    assert!(
        store
            .commit_memory_vectors(&setting, policy, vec![vector(&other, &[1., 0., 0.])])
            .await
            .is_err()
    );
    store.invalidate_fact(&tea, Timestamp(2)).await.unwrap();
    assert_eq!(search(&store, &setting, &[1., 0.]).await.len(), 0);
}

#[tokio::test]
async fn semantic_memory_rejects_stale_permissions_and_switches_without_mixing_indexes() {
    let store = open_store().await;
    let id = saved(&store, "I prefer tea").await;
    let first = store
        .configure_memory_semantics(Some(target("first")))
        .await
        .unwrap()
        .setting
        .unwrap();
    let old_policy = store.memory_policy().await.unwrap();
    store.set_memory_enabled(false).await.unwrap();
    assert!(
        store
            .memory_embedding_inputs(&first, std::slice::from_ref(&id))
            .await
            .is_err()
    );
    store.set_memory_enabled(true).await.unwrap();
    assert!(
        store
            .commit_memory_vectors(&first, old_policy, vec![vector(&id, &[1., 0.])])
            .await
            .is_err()
    );
    assert_eq!(
        store
            .memory_semantics()
            .await
            .unwrap()
            .setting
            .unwrap()
            .dimension,
        None
    );
    let policy = store.memory_policy().await.unwrap();
    store
        .commit_memory_vectors(&first, policy, vec![vector(&id, &[1., 0.])])
        .await
        .unwrap();
    let second = store
        .configure_memory_semantics(Some(target("second")))
        .await
        .unwrap()
        .setting
        .unwrap();
    assert!(second.generation > first.generation);
    assert!(
        store
            .memory_embedding_inputs(&first, std::slice::from_ref(&id))
            .await
            .is_err()
    );
    assert!(
        store
            .commit_memory_vectors(&first, policy, vec![vector(&id, &[1., 0.])])
            .await
            .is_err()
    );
    assert_eq!(search(&store, &second, &[1., 0., 0.]).await.len(), 0);
    store
        .commit_memory_vectors(&second, policy, vec![vector(&id, &[1., 0., 0.])])
        .await
        .unwrap();
    assert_eq!(search(&store, &second, &[1., 0., 0.]).await[0].fact.id, id);
    let disabled = store.configure_memory_semantics(None).await.unwrap();
    assert!(disabled.setting.is_none());
    assert!(
        store
            .memory_embedding_inputs(&second, std::slice::from_ref(&id))
            .await
            .is_err()
    );
    assert!(
        store.memory_fact(&id).await.unwrap().is_current(),
        "disabling the index does not change facts"
    );
}

#[tokio::test]
async fn a_failed_first_vector_batch_rolls_back_the_index_dimension_and_all_rows() {
    let store = open_store().await;
    let id = saved(&store, "I prefer tea").await;
    let setting = store
        .configure_memory_semantics(Some(target("search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    let policy = store.memory_policy().await.unwrap();
    assert!(
        store
            .commit_memory_vectors(
                &setting,
                policy,
                vec![
                    vector(&id, &[1., 0.]),
                    vector(&FactId("fact:missing".into()), &[0., 1.])
                ]
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .memory_semantics()
            .await
            .unwrap()
            .setting
            .unwrap()
            .dimension,
        None
    );
    assert_eq!(
        store
            .memory_embedding_inputs(&setting, std::slice::from_ref(&id))
            .await
            .unwrap()
            .len(),
        1
    );
    store
        .commit_memory_vectors(&setting, policy, vec![vector(&id, &[1., 0., 0.])])
        .await
        .unwrap();
    assert_eq!(
        store
            .memory_semantics()
            .await
            .unwrap()
            .setting
            .unwrap()
            .dimension,
        Some(3)
    );
    assert_eq!(search(&store, &setting, &[1., 0., 0.]).await.len(), 1);
}

#[tokio::test]
async fn competing_first_batches_cannot_pin_different_dimensions() {
    let store = open_store().await;
    let a = saved(&store, "I prefer tea").await;
    let b = saved(&store, "I ride a bike").await;
    let setting = store
        .configure_memory_semantics(Some(target("search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    let policy = store.memory_policy().await.unwrap();
    let (first, second) = tokio::join!(
        store.commit_memory_vectors(&setting, policy, vec![vector(&a, &[1., 0.])]),
        store.commit_memory_vectors(&setting, policy, vec![vector(&b, &[1., 0., 0.])]),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        store
            .memory_semantics()
            .await
            .unwrap()
            .setting
            .unwrap()
            .dimension,
        Some(if first.is_ok() { 2 } else { 3 })
    );
    assert_eq!(
        store
            .memory_embedding_inputs(&setting, &[a, b])
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn semantic_memory_reopens_with_consent_vectors_and_disabled_policy_intact() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sovra-semantic-{}-{unique}", std::process::id()));
    let mut config = StoreConfig::on_disk(&dir);
    config.embedding_dimension = DIM;
    let store = SurrealStore::open(config.clone()).await.unwrap();
    let id = saved(&store, "I prefer jasmine tea").await;
    let setting = store
        .configure_memory_semantics(Some(target("search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    store
        .commit_memory_vectors(
            &setting,
            store.memory_policy().await.unwrap(),
            vec![vector(&id, &[1., 0.])],
        )
        .await
        .unwrap();
    let expected = store.memory_semantics().await.unwrap();
    store.set_memory_enabled(false).await.unwrap();
    drop(store);
    let store = reopen(config.clone()).await;
    assert_eq!(store.memory_semantics().await.unwrap(), expected);
    assert_eq!(search(&store, &setting, &[1., 0.]).await.len(), 0);
    store.set_memory_enabled(true).await.unwrap();
    assert!(
        store
            .memory_embedding_inputs(&setting, std::slice::from_ref(&id))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(search(&store, &setting, &[1., 0.]).await[0].fact.id, id);
    store.stop_memory(&id, Timestamp(42)).await.unwrap();
    assert_eq!(search(&store, &setting, &[1., 0.]).await.len(), 0);
    store.configure_memory_semantics(None).await.unwrap();
    drop(store);
    let store = reopen(config).await;
    assert_eq!(
        store.memory_fact(&id).await.unwrap().stopped_at,
        Some(Timestamp(42))
    );
    assert!(store.memory_semantics().await.unwrap().setting.is_none());
    assert_eq!(
        store.memory_fact(&id).await.unwrap().content,
        "I prefer jasmine tea"
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn semantic_memory_filters_entities_and_distinguishes_valid_time_from_known_time() {
    let store = open_store().await;
    let source = store
        .record_episode(episode("chat", "Maya prefers tea", 1))
        .await
        .unwrap();
    let mut input = fact("Maya prefers tea", &["Maya"], None);
    input.source = Some(source);
    input.valid_at = Some(Timestamp(10));
    let id = store.save_fact(input).await.unwrap();
    let created = store.memory_fact(&id).await.unwrap().created_at;
    let setting = store
        .configure_memory_semantics(Some(target("search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    store
        .commit_memory_vectors(
            &setting,
            store.memory_policy().await.unwrap(),
            vec![vector(&id, &[1., 0.])],
        )
        .await
        .unwrap();
    store.invalidate_fact(&id, Timestamp(20)).await.unwrap();
    let vector = EmbeddingVector::new(vec![1., 0.]).unwrap();
    let query = MemorySearch {
        text: "beverage".into(),
        valid_at: Some(Timestamp(15)),
        entity: Some("Maya".into()),
        ..Default::default()
    };
    let found = store
        .semantic_memories(&setting, &vector, query.clone(), Timestamp(i64::MAX))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].fact.id, id);
    for invalid in [
        MemorySearch {
            entity: Some("Other person".into()),
            ..query.clone()
        },
        MemorySearch {
            valid_at: Some(Timestamp(20)),
            ..query.clone()
        },
        MemorySearch {
            known_at: Some(Timestamp(created.0 - 1)),
            ..query.clone()
        },
    ] {
        assert_eq!(
            store
                .semantic_memories(&setting, &vector, invalid, Timestamp(i64::MAX))
                .await
                .unwrap()
                .len(),
            0
        );
    }
    store.stop_memory(&id, Timestamp(30)).await.unwrap();
    store.stop_memory(&id, Timestamp(40)).await.unwrap();
    let kept = store.memory_fact(&id).await.unwrap();
    assert_eq!(kept.stopped_at, Some(Timestamp(30)));
    assert_eq!(
        kept.invalid_at,
        Some(Timestamp(20)),
        "withdrawal preserves temporal history"
    );
    assert_eq!(
        store
            .semantic_memories(&setting, &vector, query.clone(), Timestamp(i64::MAX))
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .recall(query.query("tea".into(), Timestamp(i64::MAX)))
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .list_memories(sovra_core::MemoryListQuery {
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .facts[0],
        kept
    );
    assert!(
        store
            .stop_memory(&FactId("episode:wrong".into()), Timestamp(30))
            .await
            .is_err()
    );
    assert!(
        store
            .stop_memory(&FactId("fact:missing".into()), Timestamp(30))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn historical_semantic_backfill_includes_superseded_but_not_withdrawn_versions() {
    let store = open_store().await;
    let source = store
        .record_episode(episode("past", "I preferred tea before coffee", 1))
        .await
        .unwrap();
    let mut old = fact("I preferred tea", &[], None);
    old.source = Some(source.clone());
    old.valid_at = Some(Timestamp(1));
    let old = store.save_fact(old).await.unwrap();
    let mut new = fact("I prefer coffee", &[], None);
    new.source = Some(source);
    new.replaces = Some(old.clone());
    new.valid_at = Some(Timestamp(20));
    let new = store.save_fact(new).await.unwrap();
    store.stop_memory(&new, Timestamp(30)).await.unwrap();
    let setting = store
        .configure_memory_semantics(Some(target("search")))
        .await
        .unwrap()
        .setting
        .unwrap();
    let inputs = store
        .memory_embedding_inputs(&setting, &[old.clone(), new.clone()])
        .await
        .unwrap();
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.fact_id.clone())
            .collect::<Vec<_>>(),
        vec![old.clone()]
    );
    let policy = store.memory_policy().await.unwrap();
    store
        .commit_memory_vectors(&setting, policy, vec![vector(&old, &[1., 0.])])
        .await
        .unwrap();
    assert!(
        store
            .commit_memory_vectors(&setting, policy, vec![vector(&new, &[1., 0.])])
            .await
            .is_err()
    );
    assert_eq!(search(&store, &setting, &[1., 0.]).await.len(), 0);
    let hits = store
        .semantic_memories(
            &setting,
            &EmbeddingVector::new(vec![1., 0.]).unwrap(),
            MemorySearch {
                text: "beverage".into(),
                valid_at: Some(Timestamp(10)),
                ..Default::default()
            },
            Timestamp(i64::MAX),
        )
        .await
        .unwrap();
    assert_eq!(
        hits.iter()
            .map(|hit| hit.fact.id.clone())
            .collect::<Vec<_>>(),
        vec![old]
    );
}
