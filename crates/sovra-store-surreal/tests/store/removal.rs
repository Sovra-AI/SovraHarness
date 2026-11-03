//! Whole-history removal must close both fact and original-evidence recall paths.

use super::*;
use sovra_core::{MemoryCorrection, MemoryInput, MemoryJobState, MemoryListQuery, MemorySearch};

#[tokio::test]
async fn removing_any_version_withdraws_the_family_and_sources_but_not_other_details() {
    for selected in 0..3 {
        let store = open_store().await;
        let source = store
            .record_episode(episode(
                "parcel",
                "The parcel destination is Reykjavik. The parcel is insured.",
                1000,
            ))
            .await
            .unwrap();
        let first = store
            .save_fact(NewFact {
                source: Some(source.clone()),
                ..fact("The parcel destination is Reykjavik.", &[], None)
            })
            .await
            .unwrap();
        let other = store
            .save_fact(NewFact {
                source: Some(source.clone()),
                ..fact("The parcel is insured.", &[], None)
            })
            .await
            .unwrap();
        let second = store
            .correct_memory(MemoryCorrection {
                target: first.clone(),
                text: "The parcel destination is Tallinn.".into(),
            })
            .await
            .unwrap();
        let third = store
            .correct_memory(MemoryCorrection {
                target: second.clone(),
                text: "The parcel destination is Oslo.".into(),
            })
            .await
            .unwrap();
        let family = [first, second, third];
        store
            .remove_memory(&family[selected], Timestamp(i64::MAX - 1))
            .await
            .unwrap();
        store
            .remove_memory(&family[selected], Timestamp(i64::MAX))
            .await
            .unwrap();
        for id in &family {
            let detail = store.memory_fact(id).await.unwrap();
            assert_eq!(detail.stopped_at, Some(Timestamp(i64::MAX - 1)));
            let source = store
                .review_memory_source(&detail.source.unwrap())
                .await
                .unwrap();
            assert!(source.metadata.revoked_at.is_some());
        }
        assert!(store.memory_fact(&other).await.unwrap().is_current());
        let current = store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap();
        assert_eq!(
            current
                .facts
                .iter()
                .map(|fact| &fact.id)
                .collect::<Vec<_>>(),
            vec![&other]
        );
        let retained = store
            .list_memories(MemoryListQuery {
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(retained.facts.len(), 4);
        let evidence = store
            .search_memory_sources(
                MemorySearch {
                    text: "parcel destination".into(),
                    ..Default::default()
                },
                Timestamp(i64::MAX),
            )
            .await
            .unwrap();
        assert_eq!(evidence, Vec::new());
        let historical = store
            .recall(RecallQuery {
                include_expired: true,
                ..RecallQuery::recent(20)
            })
            .await
            .unwrap();
        assert_eq!(
            historical
                .iter()
                .map(|hit| &hit.fact.id)
                .collect::<Vec<_>>(),
            vec![&other]
        );
        assert!(
            store
                .correct_memory(MemoryCorrection {
                    target: family[2].clone(),
                    text: "The parcel destination is Paris.".into()
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn removal_revokes_repeated_sources_cancels_remaining_chunks_and_suppresses_known_aliases() {
    let store = open_store().await;
    let source = store
        .enqueue_memory(MemoryInput {
            key: "dense-residence".into(),
            episode: episode("dense-residence", &"I live in Lisbon. ".repeat(400), 1_000),
        })
        .await
        .unwrap();
    let first = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let id = store
        .complete_memory(
            &first,
            vec![candidate("User lives in Lisbon")],
            "fixture",
            Timestamp(2_001),
        )
        .await
        .unwrap()
        .remove(0);
    let mut unfinished = store.claim_memory(Timestamp(2_002)).await.unwrap().unwrap();
    assert_eq!(unfinished.episode.id, source);
    let repeated = store
        .enqueue_memory(memory_input("repeat", "repeated-residence"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_003)).await.unwrap().unwrap();
    assert_eq!(claim.episode.id, repeated);
    let mut alias = candidate("User resides in Lisbon");
    alias.same_as = Some(id.clone());
    assert_eq!(
        store
            .complete_memory(&claim, vec![alias], "fixture", Timestamp(2_004))
            .await
            .unwrap(),
        vec![id.clone()]
    );
    store.remove_memory(&id, Timestamp(2_005)).await.unwrap();
    assert_eq!(
        store.memory_job(&source).await.unwrap().state,
        MemoryJobState::Cancelled
    );
    for source in [&source, &repeated] {
        assert!(
            store
                .review_memory_source(source)
                .await
                .unwrap()
                .metadata
                .revoked_at
                .is_some()
        );
    }
    // Refreshing the fact revision cannot resurrect a revoked source lease.
    unfinished.revision = store.memory_revision().await.unwrap();
    assert!(
        store
            .complete_memory(
                &unfinished,
                vec![candidate("User lives in Lisbon")],
                "fixture",
                Timestamp(2_006)
            )
            .await
            .is_err()
    );
    let later = store
        .enqueue_memory(memory_input("later", "later-residence"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_007)).await.unwrap().unwrap();
    assert_eq!(claim.episode.id, later);
    assert_eq!(
        store
            .complete_memory(
                &claim,
                vec![candidate("User resides in Lisbon")],
                "fixture",
                Timestamp(2_008)
            )
            .await
            .unwrap(),
        vec![id]
    );
    assert!(
        store
            .review_memory_source(&later)
            .await
            .unwrap()
            .metadata
            .revoked_at
            .is_some()
    );
    assert_eq!(
        store.recall(RecallQuery::recent(20)).await.unwrap(),
        Vec::new()
    );
    assert_eq!(
        store
            .search_memory_sources(
                MemorySearch {
                    text: "Lisbon".into(),
                    ..Default::default()
                },
                Timestamp(3_000)
            )
            .await
            .unwrap(),
        Vec::new()
    );
}
