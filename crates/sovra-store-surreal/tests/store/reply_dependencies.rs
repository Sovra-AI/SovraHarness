//! Helper evidence is useful only while its immutable input dependencies remain permitted.

use super::*;
use sovra_core::{
    EpisodeId, FactId, MemoryCorrection, MemoryInput, MemorySearch, MemorySourceAdmission,
    MemorySourceAuthor, MemorySourceDependencies, MemorySourceOrigin,
};

fn reply(key: &str) -> MemorySourceAdmission {
    let mut source = MemorySourceAdmission::user(MemoryInput {
        key: key.into(),
        episode: episode(
            "helper-dependencies",
            "A helper suggested a juniper itinerary.",
            100,
        ),
    });
    source.dependencies = Some(MemorySourceDependencies::default());
    source.author = MemorySourceAuthor::Assistant { speaker: None };
    source.episode.kind = EpisodeKind::AgentMessage;
    source
}

#[tokio::test]
async fn fully_tracked_helper_evidence_can_be_recalled_without_becoming_a_personal_fact() {
    let store = open_store().await;
    let source = reply("standalone");
    let receipt = store.admit_memory_source(source).await.unwrap();
    let evidence = store
        .search_memory_sources(
            MemorySearch {
                text: "juniper".into(),
                ..Default::default()
            },
            Timestamp(1000),
        )
        .await
        .unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].source_id, receipt.source_id);
    assert_eq!(evidence[0].origin, MemorySourceOrigin::AssistantOutput);
    assert!(!evidence[0].support_changed);
    assert!(!receipt.has_processing_job);
    assert!(store.claim_memory(Timestamp(1000)).await.unwrap().is_none());
}

async fn user_source(store: &SurrealStore) -> EpisodeId {
    store
        .enqueue_memory(MemoryInput {
            key: "user-input".into(),
            episode: episode("helper-dependencies", "The venue is on the coast.", 1),
        })
        .await
        .unwrap()
}

async fn save_reply(
    store: &SurrealStore,
    key: &str,
    sources: Vec<EpisodeId>,
    facts: Vec<FactId>,
) -> EpisodeId {
    let mut source = reply(key);
    source.dependencies = Some(MemorySourceDependencies { sources, facts });
    store.admit_memory_source(source).await.unwrap().source_id
}

async fn lookup(store: &SurrealStore) -> Vec<sovra_core::MemorySourceEvidence> {
    store
        .search_memory_sources(
            MemorySearch {
                text: "juniper".into(),
                ..Default::default()
            },
            Timestamp(i64::MAX),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn ancestor_withdrawal_hides_transitive_helpers_without_hiding_unrelated_replies() {
    for withdraw_parent in [false, true] {
        let store = open_store().await;
        let root = user_source(&store).await;
        let first = save_reply(&store, "first", vec![root.clone()], vec![]).await;
        let second = save_reply(&store, "second", vec![first.clone()], vec![]).await;
        let unrelated = save_reply(&store, "unrelated", vec![], vec![]).await;
        assert_eq!(lookup(&store).await.len(), 3);
        store
            .stop_memory_source(if withdraw_parent { &first } else { &root }, Timestamp(300))
            .await
            .unwrap();
        let hits = lookup(&store).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, unrelated);
        save_reply(&store, "late", vec![second.clone()], vec![]).await;
        assert_eq!(
            lookup(&store).await.len(),
            1,
            "admission after revocation must not revive an ancestor"
        );
        assert_eq!(
            store
                .review_memory_source(&second)
                .await
                .unwrap()
                .episode
                .content,
            reply("second").episode.content
        );
    }
}

#[tokio::test]
async fn corrections_label_helper_support_and_withdrawal_removes_it_through_fact_dependencies() {
    let store = open_store().await;
    let root = user_source(&store).await;
    let fact = store
        .save_fact(NewFact {
            source: Some(root),
            ..fact("The venue is on the coast.", &[], None)
        })
        .await
        .unwrap();
    let known = store.memory_fact(&fact).await.unwrap().created_at;
    let first = save_reply(&store, "first", vec![], vec![fact.clone()]).await;
    save_reply(&store, "second", vec![first], vec![]).await;
    assert!(
        lookup(&store)
            .await
            .iter()
            .all(|source| !source.support_changed)
    );
    let corrected = store
        .correct_memory(MemoryCorrection {
            target: fact,
            text: "The venue is inland.".into(),
        })
        .await
        .unwrap();
    let hits = lookup(&store).await;
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|source| source.support_changed));
    for valid_at in [None, Some(Timestamp(i64::MAX))] {
        let historical = store
            .search_memory_sources(
                MemorySearch {
                    text: "juniper".into(),
                    known_at: Some(known),
                    valid_at,
                    ..Default::default()
                },
                Timestamp(i64::MAX),
            )
            .await
            .unwrap();
        assert_eq!(historical.len(), 2);
        assert!(
            historical.iter().all(|source| !source.support_changed),
            "later correction metadata must not leak into an earlier knowledge date"
        );
    }
    store
        .remove_memory(&corrected, Timestamp(i64::MAX))
        .await
        .unwrap();
    assert_eq!(lookup(&store).await, Vec::new());
}

#[tokio::test]
async fn unknown_missing_and_changed_dependency_manifests_never_gain_recall_permission() {
    let store = open_store().await;
    let mut unknown = reply("unknown");
    unknown.dependencies = None;
    let untracked = store
        .admit_memory_source(unknown.clone())
        .await
        .unwrap()
        .source_id;
    save_reply(&store, "inherits-unknown", vec![untracked], vec![]).await;
    save_reply(
        &store,
        "missing-source",
        vec![EpisodeId("episode:absent".into())],
        vec![],
    )
    .await;
    save_reply(
        &store,
        "missing-fact",
        vec![],
        vec![FactId("fact:absent".into())],
    )
    .await;
    assert_eq!(lookup(&store).await, Vec::new());
    unknown.dependencies = Some(MemorySourceDependencies::default());
    assert!(
        store.admit_memory_source(unknown).await.is_err(),
        "an existing version cannot be upgraded by replacing its manifest"
    );
    let root = user_source(&store).await;
    let mut first = reply("idempotent");
    first.dependencies = Some(MemorySourceDependencies {
        sources: vec![root.clone(), root.clone()],
        facts: vec![],
    });
    let receipt = store.admit_memory_source(first.clone()).await.unwrap();
    first.dependencies.as_mut().unwrap().sources = vec![root];
    assert_eq!(
        store.admit_memory_source(first.clone()).await.unwrap(),
        receipt
    );
    first.dependencies = Some(MemorySourceDependencies::default());
    assert!(store.admit_memory_source(first).await.is_err());
}

#[tokio::test]
async fn dependency_closure_overflow_keeps_evidence_but_never_truncates_permissions() {
    let store = open_store().await;
    let mut previous = None;
    let mut at_limit = None;
    for index in 0..=sovra_core::MAX_MEMORY_SOURCE_DEPENDENCIES + 1 {
        let mut source = reply(&format!("chain-{index}"));
        source.episode.content = format!("Juniper chainmarker{index}.");
        source.dependencies = Some(MemorySourceDependencies {
            sources: previous.into_iter().collect(),
            facts: vec![],
        });
        let id = store.admit_memory_source(source).await.unwrap().source_id;
        if index == sovra_core::MAX_MEMORY_SOURCE_DEPENDENCIES {
            at_limit = Some(id.clone());
        }
        previous = Some(id);
    }
    for (index, expected) in [
        (sovra_core::MAX_MEMORY_SOURCE_DEPENDENCIES, 1),
        (sovra_core::MAX_MEMORY_SOURCE_DEPENDENCIES + 1, 0),
    ] {
        let hits = store
            .search_memory_sources(
                MemorySearch {
                    text: format!("chainmarker{index}"),
                    ..Default::default()
                },
                Timestamp(1000),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), expected);
        if let Some(hit) = hits.first() {
            assert_eq!(Some(&hit.source_id), at_limit.as_ref());
        }
    }
    assert!(
        store
            .review_memory_source(&previous.unwrap())
            .await
            .unwrap()
            .episode
            .content
            .contains("chainmarker129")
    );
}

#[tokio::test]
async fn reply_dependencies_and_withdrawal_survive_real_disk_reopen() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sovra-helper-lineage-{}-{unique}",
        std::process::id()
    ));
    let config = StoreConfig::on_disk(&path);
    let store = SurrealStore::open(config.clone()).await.unwrap();
    let root = user_source(&store).await;
    let first = save_reply(&store, "first", vec![root.clone()], vec![]).await;
    let second = save_reply(&store, "second", vec![first], vec![]).await;
    drop(store);
    let store = reopen(config.clone()).await;
    assert_eq!(lookup(&store).await.len(), 2);
    store
        .stop_memory_source(&root, Timestamp(300))
        .await
        .unwrap();
    drop(store);
    let store = reopen(config).await;
    assert_eq!(lookup(&store).await, Vec::new());
    assert_eq!(
        store.episode(&second).await.unwrap().content,
        reply("second").episode.content
    );
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
