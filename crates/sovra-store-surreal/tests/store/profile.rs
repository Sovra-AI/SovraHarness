//! Capture summaries use persisted jobs and remain independent of search pages.

use super::*;
use sovra_core::{MemoryCorrection, MemoryJobState, MemoryListQuery, MemoryOverview};

#[tokio::test]
async fn explicit_notes_are_atomic_sourced_and_available_while_capture_is_paused() {
    let store = open_store().await;
    store.set_memory_enabled(false).await.unwrap();
    let id = store
        .remember_note(sovra_core::MemoryNote {
            text: " I prefer short answers ".into(),
        })
        .await
        .unwrap();
    let fact = store.memory_fact(&id).await.unwrap();
    assert_eq!(fact.content, "I prefer short answers");
    assert!(fact.is_current());
    assert!(fact.extraction.is_none());
    let source = store.episode(fact.source.as_ref().unwrap()).await.unwrap();
    assert_eq!(source.kind, EpisodeKind::Note);
    assert_eq!(source.content, fact.content);
    let mut context = sovra_core::MemoryContext::default();
    assert!(context.push(&fact, &source));
    assert!(!context.memories[0].interpretation);
    assert!(!store.memory_policy().await.unwrap().enabled);
    assert!(
        store
            .remember_note(sovra_core::MemoryNote { text: " ".into() })
            .await
            .is_err()
    );
    assert_eq!(
        store
            .recent_episodes("memory-review", 50)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(overview(&store).await.current, 1);
}

#[tokio::test]
async fn retry_requeues_failures_with_fresh_leases_even_beyond_old_capacity() {
    let store = open_store().await;
    let failed = store
        .enqueue_memory(memory_input("failed", "m1"))
        .await
        .unwrap();
    let mut now = Timestamp(2_000);
    let mut old_claim = None;
    for _ in 0..sovra_core::MAX_MEMORY_ATTEMPTS {
        let claim = store.claim_memory(now).await.unwrap().unwrap();
        store
            .fail_memory(&claim, "Cannot read this message.", now)
            .await
            .unwrap();
        now = Timestamp(now.millis() + sovra_core::MEMORY_RETRY_MILLIS);
        old_claim = Some(claim);
    }
    let completed = store
        .enqueue_memory(memory_input("complete", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(now).await.unwrap().unwrap();
    store
        .complete_memory(&claim, vec![], "fixture", now)
        .await
        .unwrap();
    let cancelled = store
        .enqueue_memory(memory_input("cancelled", "m1"))
        .await
        .unwrap();
    store.set_memory_enabled(false).await.unwrap();
    assert!(store.retry_failed_memories().await.is_err());
    assert_eq!(
        store.memory_job(&failed).await.unwrap().state,
        MemoryJobState::Failed
    );
    store.set_memory_enabled(true).await.unwrap();
    for index in 0..sovra_core::MAX_PENDING_MEMORIES {
        store
            .enqueue_memory(memory_input("queued", &format!("m{index}")))
            .await
            .unwrap();
    }
    assert_eq!(store.retry_failed_memories().await.unwrap(), 1);
    assert_eq!(store.retry_failed_memories().await.unwrap(), 0);
    let job = store.memory_job(&failed).await.unwrap();
    assert_eq!(job.state, MemoryJobState::Pending);
    assert_eq!(job.attempts, 0);
    assert!(job.error.is_none());
    let retry = store.claim_memory(now).await.unwrap().unwrap();
    assert_eq!(retry.episode.id, failed);
    assert_eq!(retry.attempt, 1);
    assert_ne!(retry.token, old_claim.as_ref().unwrap().token);
    assert!(
        store
            .complete_memory(old_claim.as_ref().unwrap(), vec![], "fixture", now)
            .await
            .is_err()
    );
    store
        .complete_memory(
            &retry,
            vec![candidate("User lives in Lisbon")],
            "fixture",
            now,
        )
        .await
        .unwrap();
    assert_eq!(
        store.memory_job(&completed).await.unwrap().state,
        MemoryJobState::Complete
    );
    assert_eq!(
        store.memory_job(&cancelled).await.unwrap().state,
        MemoryJobState::Cancelled
    );
}

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

#[tokio::test]
async fn subjects_count_all_current_details_with_stable_order_and_a_bound() {
    let store = open_store().await;
    store
        .save_fact(fact("Mira manages Cedar", &["Mira", "Cedar", "user"], None))
        .await
        .unwrap();
    let second = store
        .save_fact(fact("Mira keeps notes", &["Mira"], None))
        .await
        .unwrap();
    let summary = overview(&store).await;
    assert_eq!(
        summary.subjects,
        vec![
            sovra_core::MemorySubject {
                name: "Mira".into(),
                details: 2
            },
            sovra_core::MemorySubject {
                name: "Cedar".into(),
                details: 1
            },
        ]
    );
    assert!(!summary.more_subjects);
    store.stop_memory(&second, Timestamp(9)).await.unwrap();
    let summary = overview(&store).await;
    assert_eq!(
        summary
            .subjects
            .iter()
            .map(|s| (s.name.as_str(), s.details))
            .collect::<Vec<_>>(),
        vec![("Cedar", 1), ("Mira", 1)]
    );
    for index in 0..14 {
        store
            .save_fact(fact(
                "another subject",
                &[&format!("Topic {index:02}")],
                None,
            ))
            .await
            .unwrap();
    }
    let summary = overview(&store).await;
    assert_eq!(summary.subjects.len(), 12);
    assert!(summary.more_subjects);
    assert_eq!(summary.subjects[0].name, "Cedar");
}

#[tokio::test]
async fn subject_directory_pages_filters_and_excludes_old_or_stopped_details() {
    use sovra_core::MemorySubjectQuery;
    let store = open_store().await;
    for name in ["Alder", "Birch", "Cedar", "Dogwood", "École", "茶"] {
        store
            .save_fact(fact(name, &[name, "user"], None))
            .await
            .unwrap();
    }
    let old = store
        .save_fact(fact("old", &["Old subject"], None))
        .await
        .unwrap();
    store.invalidate_fact(&old, Timestamp(100)).await.unwrap();
    let stopped = store
        .save_fact(fact("stopped", &["Stopped subject"], None))
        .await
        .unwrap();
    store.stop_memory(&stopped, Timestamp(100)).await.unwrap();
    let mut query = MemorySubjectQuery {
        limit: 2,
        ..Default::default()
    };
    let first = store.list_memory_subjects(query.clone()).await.unwrap();
    assert_eq!(
        first
            .subjects
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alder", "Birch"]
    );
    query.after = first.next;
    // Inserting before the cursor must not shift or repeat subsequent pages.
    store
        .save_fact(fact("new", &["Aspen"], None))
        .await
        .unwrap();
    let mut names = Vec::new();
    loop {
        let page = store.list_memory_subjects(query.clone()).await.unwrap();
        names.extend(page.subjects.into_iter().map(|s| s.name));
        query.after = page.next;
        if query.after.is_none() {
            break;
        }
    }
    assert_eq!(names, vec!["Cedar", "Dogwood", "École", "茶"]);
    let filtered = store
        .list_memory_subjects(MemorySubjectQuery {
            search: "ÉCO".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(filtered.subjects[0].name, "École");
    assert_eq!(filtered.subjects.len(), 1);
    assert!(filtered.next.is_none());
    let none = store
        .list_memory_subjects(MemorySubjectQuery {
            search: "missing".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(none.subjects, Vec::<sovra_core::MemorySubject>::new());
}
