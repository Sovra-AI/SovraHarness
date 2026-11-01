//! Capture summaries use persisted jobs and remain independent of search pages.

use super::*;
use sovra_core::{MemoryCorrection, MemoryJobState, MemoryListQuery, MemoryOverview};

async fn overview(store: &SurrealStore) -> MemoryOverview {
    store
        .list_memories(MemoryListQuery {
            search: "no matching facts".into(),
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap()
        .overview
        .expect("the embedded store reports capture totals")
}

#[tokio::test]
async fn overview_tracks_saved_details_history_and_jobs_without_search_filtering() {
    let store = open_store().await;
    assert_eq!(overview(&store).await, MemoryOverview::default());
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let pending = overview(&store).await;
    assert_eq!(pending.waiting, 1);
    assert_eq!(pending.recent_activity[0].episode_id, id);
    assert_eq!(pending.recent_activity[0].session_id, "s1");
    assert_eq!(pending.recent_activity[0].state, MemoryJobState::Pending);
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let working = overview(&store).await;
    assert_eq!((working.waiting, working.working), (0, 1));
    let ids = store
        .complete_memory(
            &claim,
            vec![candidate("User lives in Lisbon")],
            "test-model",
            Timestamp(2_010),
        )
        .await
        .unwrap();
    let saved = overview(&store).await;
    assert_eq!((saved.current, saved.checked, saved.working), (1, 1, 0));
    assert_eq!(saved.recent_activity[0].saved, 1);
    assert_eq!(
        saved.last_saved_at,
        Some(store.memory_fact(&ids[0]).await.unwrap().created_at)
    );
    let replacement = store
        .correct_memory(MemoryCorrection {
            target: ids[0].clone(),
            text: "I live in Porto".into(),
        })
        .await
        .unwrap();
    let corrected = overview(&store).await;
    assert_eq!(
        (corrected.current, corrected.history, corrected.stopped),
        (1, 1, 0)
    );
    store
        .stop_memory(&replacement, Timestamp(i64::MAX - 1))
        .await
        .unwrap();
    let stopped = overview(&store).await;
    assert_eq!(
        (stopped.current, stopped.history, stopped.stopped),
        (0, 1, 1)
    );
}

#[tokio::test]
async fn overview_keeps_failure_and_empty_check_receipts_and_bounds_recent_activity() {
    let store = open_store().await;
    let failed = store
        .enqueue_memory(memory_input("failed", "m1"))
        .await
        .unwrap();
    let mut now = Timestamp(2_000);
    for _ in 0..sovra_core::MAX_MEMORY_ATTEMPTS {
        let claim = store.claim_memory(now).await.unwrap().unwrap();
        store
            .fail_memory(&claim, "Choose a working AI in Models.", now)
            .await
            .unwrap();
        now = Timestamp(now.millis() + sovra_core::MEMORY_RETRY_MILLIS);
    }
    let failure = overview(&store).await;
    assert_eq!((failure.failed, failure.waiting), (1, 0));
    assert_eq!(failure.recent_activity[0].episode_id, failed);
    assert_eq!(
        failure.recent_activity[0].error.as_deref(),
        Some("Choose a working AI in Models.")
    );
    store
        .enqueue_memory(memory_input("empty", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(now).await.unwrap().unwrap();
    store
        .complete_memory(&claim, vec![], "test-model", now)
        .await
        .unwrap();
    let checked = overview(&store).await;
    assert_eq!((checked.checked, checked.current), (1, 0));
    assert!(
        checked
            .recent_activity
            .iter()
            .any(|activity| { activity.state == MemoryJobState::Complete && activity.saved == 0 })
    );
    for index in 0..10 {
        store
            .enqueue_memory(memory_input("waiting", &format!("m{index}")))
            .await
            .unwrap();
    }
    let waiting = overview(&store).await;
    assert_eq!(waiting.waiting, 10);
    assert_eq!(waiting.recent_activity.len(), 8);
    assert!(
        waiting
            .recent_activity
            .windows(2)
            .all(|pair| pair[0].created_at >= pair[1].created_at)
    );
    store.set_memory_enabled(false).await.unwrap();
    let paused = overview(&store).await;
    assert_eq!(
        (
            paused.waiting,
            paused.cancelled,
            paused.failed,
            paused.checked
        ),
        (0, 10, 1, 1)
    );
}
