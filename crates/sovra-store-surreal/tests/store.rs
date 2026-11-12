//! Integration tests against a real embedded engine (`mem://`), exercising the port
//! exactly as the runtime will.

use sovra_core::{
    Embedding, EpisodeKind, MemoryError, MemoryStore, NewEpisode, NewFact, RecallQuery, Timestamp,
};
use sovra_store_surreal::{StoreConfig, SurrealStore};

const DIM: usize = 4;

#[cfg(test)]
#[path = "store/semantic.rs"]
mod semantic;

#[cfg(test)]
#[path = "store/consolidation.rs"]
mod consolidation;

#[cfg(test)]
#[path = "store/growth.rs"]
mod growth;

#[path = "store/retrieval_evaluation.rs"]
mod retrieval_evaluation;

#[cfg(test)]
#[path = "store/profile.rs"]
mod profile;

#[path = "store/removal.rs"]
mod removal;

#[cfg(test)]
#[path = "store/reply_dependencies.rs"]
mod reply_dependencies;

#[tokio::test]
async fn memory_browsing_pages_without_duplicates_and_filters_before_limiting() {
    use sovra_core::MemoryListQuery;
    let store = open_store().await;
    let mut ids = Vec::new();
    for i in 0..7 {
        ids.push(
            store
                .save_fact(fact(&format!("Tea preference {i}"), &[], None))
                .await
                .unwrap(),
        );
    }
    store.invalidate_fact(&ids[3], Timestamp(10)).await.unwrap();
    let mut query = MemoryListQuery {
        search: "TEA".into(),
        limit: 2,
        ..Default::default()
    };
    let first = store.list_memories(query.clone()).await.unwrap();
    assert_eq!(first.facts.len(), 2);
    assert_eq!(first.facts[0].id, ids[6]);
    assert!(first.next.is_some());
    // Newer writes cannot shift the keyset boundary and repeat the previous page.
    store
        .save_fact(fact("Tea added after page one", &[], None))
        .await
        .unwrap();
    let mut found = first.facts;
    query.before = first.next;
    loop {
        let page = store.list_memories(query.clone()).await.unwrap();
        found.extend(page.facts);
        query.before = page.next;
        if query.before.is_none() {
            break;
        }
    }
    let expected: Vec<_> = ids
        .iter()
        .rev()
        .filter(|id| *id != &ids[3])
        .cloned()
        .collect();
    assert_eq!(
        found.into_iter().map(|fact| fact.id).collect::<Vec<_>>(),
        expected
    );
    let history = store
        .list_memories(MemoryListQuery {
            include_history: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(history.facts.len(), 8);
    assert!(history.next.is_none());
    assert_eq!(
        store
            .list_memories(MemoryListQuery {
                search: "coffee".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .facts,
        Vec::<sovra_core::Fact>::new()
    );
    assert!(
        store
            .list_memories(MemoryListQuery {
                limit: 51,
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        store
            .memory_fact(&sovra_core::FactId("episode:wrong-table".into()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_memory_corrections_keep_sources_and_roll_back_stale_edits_while_paused() {
    use sovra_core::{MemoryCorrection, MemoryListQuery};
    let store = open_store().await;
    let original = store
        .save_fact(fact("I prefer black tea", &["Tea"], None))
        .await
        .unwrap();
    store.set_memory_enabled(false).await.unwrap();
    let edit = MemoryCorrection {
        target: original.clone(),
        text: "I prefer jasmine tea".into(),
    };
    let corrected = store.correct_memory(edit.clone()).await.unwrap();
    let current = store.memory_fact(&corrected).await.unwrap();
    assert!(current.is_current());
    assert_eq!(current.replaces, Some(original.clone()));
    assert_eq!(current.entities, Vec::<String>::new());
    assert!(current.extraction.is_none());
    let source = store
        .episode(current.source.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(source.kind, EpisodeKind::Note);
    assert_eq!(source.content, "I prefer jasmine tea");
    let mut context = sovra_core::MemoryContext::default();
    assert!(
        context.push(&current, &source),
        "an explicit correction is supported recall evidence"
    );
    assert!(!context.memories[0].interpretation);
    assert!(!store.memory_fact(&original).await.unwrap().is_current());
    assert!(store.correct_memory(edit).await.is_err());
    assert_eq!(
        store
            .recent_episodes("memory-review", 50)
            .await
            .unwrap()
            .len(),
        1,
        "failed edits must not orphan source notes"
    );
    store
        .invalidate_fact(&corrected, Timestamp(current.created_at.0 + 1))
        .await
        .unwrap();
    assert!(
        store
            .correct_memory(MemoryCorrection {
                target: corrected,
                text: "Do not revive this".into()
            })
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap()
            .facts,
        Vec::<sovra_core::Fact>::new()
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
        2
    );
    assert!(!store.memory_policy().await.unwrap().enabled);
    assert_eq!(
        store
            .recent_episodes("memory-review", 50)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn auth_records_roundtrip_with_session_and_phc_hash() {
    let store = open_store().await;
    assert!(store.load_auth().await.unwrap().is_none());

    let account = sovra_core::auth::AuthAccount {
        id: "account-1-42".into(),
        email: "barney@sovra.ai".into(),
        full_name: "Barney the Jedi".into(),
        provider: sovra_core::auth::AuthProvider::Password,
        password_phc: Some("$argon2id$v=19$m=19456,t=2,p=1$salt$hash".into()),
        created_at: Timestamp(42),
    };
    let records = sovra_core::auth::AuthRecords {
        accounts: vec![account.clone()],
        session: Some(sovra_core::auth::AuthSession {
            account_id: account.id.clone(),
            email: account.email.clone(),
            full_name: account.full_name.clone(),
            provider: account.provider,
            signed_in_at: Timestamp(43),
        }),
    };
    store.save_auth(&records).await.unwrap();
    let loaded = store.load_auth().await.unwrap().expect("records saved");
    assert_eq!(loaded, records);

    // Signing out persists the cleared session without losing the account.
    let signed_out = sovra_core::auth::AuthRecords {
        session: None,
        ..records
    };
    store.save_auth(&signed_out).await.unwrap();
    let loaded = store.load_auth().await.unwrap().expect("records saved");
    assert_eq!(loaded.accounts.len(), 1);
    assert!(loaded.session.is_none());
}

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
async fn failed_fact_write_leaves_no_partial_fact() {
    let store = open_store().await;
    let mut candidate = fact("must not survive", &["Maya"], None);
    candidate.source = Some(sovra_core::EpisodeId("episode:missing".into()));
    assert!(store.save_fact(candidate).await.is_err());
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn correction_requires_a_current_existing_fact() {
    let store = open_store().await;
    let original = store
        .save_fact(fact("Maya lives in Rome", &[], None))
        .await
        .unwrap();
    let mut correction = fact("Maya lives in Lisbon", &[], None);
    correction.replaces = Some(original.clone());
    store.save_fact(correction.clone()).await.unwrap();
    assert!(
        store.save_fact(correction).await.is_err(),
        "stale correction must be rejected"
    );
    let mut missing = fact("orphan correction", &[], None);
    missing.replaces = Some(sovra_core::FactId("fact:missing".into()));
    assert!(store.save_fact(missing).await.is_err());
    let mut query = RecallQuery::recent(10);
    query.include_expired = true;
    assert_eq!(store.recall(query).await.unwrap().len(), 2);
}

#[tokio::test]
async fn concurrent_corrections_cannot_create_two_current_replacements() {
    let store = open_store().await;
    let old = store
        .save_fact(fact("Maya lives in Rome", &[], None))
        .await
        .unwrap();
    let mut first = fact("Maya lives in Lisbon", &[], None);
    first.replaces = Some(old.clone());
    let mut second = fact("Maya lives in Paris", &[], None);
    second.replaces = Some(old);
    let (a, b) = tokio::join!(store.save_fact(first), store.save_fact(second));
    assert_ne!(a.is_ok(), b.is_ok(), "exactly one correction may commit");
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap().len(),
        1
    );
    let mut query = RecallQuery::recent(10);
    query.include_expired = true;
    assert_eq!(
        store.recall(query).await.unwrap().len(),
        2,
        "loser's partial fact rolled back"
    );
}

#[tokio::test]
async fn correction_keeps_world_time_separate_from_recording_time() {
    let store = open_store().await;
    let original = store
        .save_fact(fact("Maya lives in Rome", &[], None))
        .await
        .unwrap();
    let mut correction = fact("Maya lives in Lisbon", &[], None);
    correction.valid_at = Some(Timestamp(2_000));
    correction.replaces = Some(original.clone());
    store.save_fact(correction).await.unwrap();
    let mut query = RecallQuery::recent(10);
    query.include_expired = true;
    let facts = store.recall(query).await.unwrap();
    let old = &facts.iter().find(|f| f.fact.id == original).unwrap().fact;
    assert_eq!(old.invalid_at, Some(Timestamp(2_000)));
    assert!(old.expired_at.unwrap() > Timestamp(2_000));
}

#[tokio::test]
async fn invalidating_twice_preserves_the_first_history_boundary() {
    let store = open_store().await;
    let id = store
        .save_fact(fact("earlier belief", &[], None))
        .await
        .unwrap();
    store.invalidate_fact(&id, Timestamp(2_000)).await.unwrap();
    store.invalidate_fact(&id, Timestamp(3_000)).await.unwrap();
    let mut query = RecallQuery::recent(10);
    query.include_expired = true;
    let old = store.recall(query).await.unwrap().remove(0).fact;
    assert_eq!(old.invalid_at, Some(Timestamp(2_000)));
    assert_eq!(old.expired_at, Some(Timestamp(2_000)));
}

#[tokio::test]
async fn record_ids_cannot_target_another_table() {
    let store = open_store().await;
    let id = store
        .record_episode(episode("s1", "source", 1_000))
        .await
        .unwrap();
    assert!(
        store
            .invalidate_fact(&sovra_core::FactId(id.0), Timestamp(2_000))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oversized_recall_does_not_overflow_or_scan_without_a_bound() {
    let store = open_store().await;
    assert!(store.recall(RecallQuery::recent(usize::MAX)).await.is_err());
    assert_eq!(
        store.recall(RecallQuery::recent(0)).await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn zero_length_and_non_finite_vectors_are_rejected() {
    let store = open_store().await;
    for vector in [[0.0; DIM], [f32::NAN; DIM], [f32::INFINITY; DIM]] {
        assert!(
            store
                .save_fact(fact("invalid vector", &[], Some(vector)))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn the_first_embedding_pins_the_model_for_writes_and_queries() {
    let store = open_store().await;
    store
        .save_fact(fact("first", &[], Some([1.0, 0.0, 0.0, 0.0])))
        .await
        .unwrap();
    let mut other = fact("mixed model", &[], Some([1.0, 0.0, 0.0, 0.0]));
    other.embedding.as_mut().unwrap().model = "incompatible".into();
    assert!(store.save_fact(other).await.is_err());
    let mut query = RecallQuery::recent(10);
    query.embedding = Some(Embedding {
        model: "incompatible".into(),
        vector: vec![1.0; DIM],
    });
    assert!(store.recall(query).await.is_err());
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap().len(),
        1
    );
}

/// Embedded engines release their file handles asynchronously when the client drops.
async fn reopen(config: StoreConfig) -> SurrealStore {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match SurrealStore::open(config.clone()).await {
                Ok(store) => return store,
                Err(MemoryError::Unavailable(_)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("reopen: {error}"),
            }
        }
    })
    .await
    .expect("embedded engine releases its directory")
}

#[tokio::test]
async fn disk_reopen_preserves_history_sources_and_embedding_identity() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sovra-memory-{}-{unique}", std::process::id()));
    let mut config = StoreConfig::on_disk(&dir);
    config.embedding_dimension = DIM;
    let store = SurrealStore::open(config.clone()).await.unwrap();
    let source = store
        .record_episode(episode("s1", "I live in Rome", 1))
        .await
        .unwrap();
    let mut original = fact("Maya lives in Rome", &["Maya"], Some([1.0, 0.0, 0.0, 0.0]));
    original.source = Some(source.clone());
    let old = store.save_fact(original).await.unwrap();
    let mut correction = fact("Maya lives in Lisbon", &["Maya"], None);
    correction.replaces = Some(old.clone());
    correction.source = Some(source.clone());
    let new = store.save_fact(correction).await.unwrap();
    store.invalidate_fact(&new, Timestamp(3)).await.unwrap();
    let job_id = store
        .enqueue_memory(memory_input("recover", "m1"))
        .await
        .unwrap();
    let lease = store.claim_memory(Timestamp(4_000)).await.unwrap().unwrap();
    let mut thread = sovra_core::ChatThread::new("restored-chat", Timestamp(1));
    thread.messages = vec![
        sovra_core::ChatMessage::user("user", "Our garden app uses Rust", ""),
        sovra_core::ChatMessage::assistant("reply", "We chose an offline design", "", vec![]),
    ];
    store.save_chat_thread(&thread).await.unwrap();
    drop(store);

    let store = reopen(config.clone()).await;
    let saved_thread = store.list_chat_threads(1).await.unwrap().remove(0);
    assert_eq!(saved_thread.id, "restored-chat");
    let context = sovra_core::ConversationContext::from_messages(&saved_thread.messages);
    assert_eq!(context.messages()[0].role, sovra_core::MessageRole::User);
    assert_eq!(context.messages()[1].text, "We chose an offline design");
    assert_eq!(
        store.memory_job(&job_id).await.unwrap().state,
        sovra_core::MemoryJobState::Processing
    );
    let recovered = store
        .claim_memory(lease.lease_until)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.attempt, 2);
    store
        .complete_memory(&recovered, Vec::new(), "test-model", lease.lease_until)
        .await
        .unwrap();
    store.set_memory_enabled(false).await.unwrap();
    assert_eq!(store.recent_episodes("s1", 10).await.unwrap()[0].id, source);
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
    let mut history = RecallQuery::recent(10);
    history.include_expired = true;
    let saved = store.recall(history).await.unwrap();
    assert_eq!(saved.len(), 2);
    assert!(
        saved
            .iter()
            .all(|hit| hit.fact.source == Some(source.clone()))
    );
    assert_eq!(
        saved
            .iter()
            .find(|hit| hit.fact.id == new)
            .unwrap()
            .fact
            .replaces,
        Some(old.clone())
    );
    assert!(
        saved
            .iter()
            .any(|hit| hit.fact.id == old && !hit.fact.is_current())
    );
    assert!(
        saved
            .iter()
            .any(|hit| hit.fact.id == new && hit.fact.invalid_at == Some(Timestamp(3)))
    );
    let mut wrong = embedding([1.0, 0.0, 0.0, 0.0]);
    wrong.model = "different-model".into();
    assert!(matches!(
        store.recall(RecallQuery::semantic(wrong, "Maya", 10)).await,
        Err(MemoryError::EmbeddingMismatch { .. })
    ));
    let editable = store
        .save_fact(fact("I drink black tea", &[], None))
        .await
        .unwrap();
    let reviewed = store
        .correct_memory(sovra_core::MemoryCorrection {
            target: editable.clone(),
            text: "I drink jasmine tea".into(),
        })
        .await
        .unwrap();
    store
        .invalidate_fact(&reviewed, Timestamp(i64::MAX))
        .await
        .unwrap();
    drop(store);
    let mut isolated_config = config.clone();
    isolated_config.database = "other_workspace".into();
    let isolated = reopen(isolated_config).await;
    assert!(matches!(
        isolated.episode(&source).await,
        Err(MemoryError::NotFound(_))
    ));
    assert!(matches!(
        isolated.memory_job(&job_id).await,
        Err(MemoryError::NotFound(_))
    ));
    assert_eq!(
        isolated.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
    drop(isolated);
    let store = reopen(config.clone()).await;
    store.health().await.unwrap();
    let reviewed_fact = store.memory_fact(&reviewed).await.unwrap();
    assert!(!reviewed_fact.is_current());
    assert_eq!(reviewed_fact.replaces, Some(editable));
    assert_eq!(
        store
            .episode(reviewed_fact.source.as_ref().unwrap())
            .await
            .unwrap()
            .content,
        "I drink jasmine tea"
    );
    let reviewed_history = store
        .list_memories(sovra_core::MemoryListQuery {
            search: "tea".into(),
            include_history: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(reviewed_history.facts.len(), 2);
    assert_eq!(
        store
            .list_memories(sovra_core::MemoryListQuery::default())
            .await
            .unwrap()
            .facts,
        Vec::<sovra_core::Fact>::new()
    );
    assert!(!store.memory_policy().await.unwrap().enabled);
    assert_eq!(
        store.memory_job(&job_id).await.unwrap().state,
        sovra_core::MemoryJobState::Complete
    );
    assert!(
        store
            .enqueue_memory(memory_input("recover", "new"))
            .await
            .is_err()
    );
    let note = store
        .remember_note(sovra_core::MemoryNote {
            text: "I prefer short answers".into(),
        })
        .await
        .unwrap();
    let overview = store
        .list_memories(sovra_core::MemoryListQuery::default())
        .await
        .unwrap()
        .overview;
    drop(store);
    let store = reopen(config).await;
    let saved = store.memory_fact(&note).await.unwrap();
    assert!(saved.is_current());
    assert_eq!(saved.content, "I prefer short answers");
    assert_eq!(
        store
            .episode(saved.source.as_ref().unwrap())
            .await
            .unwrap()
            .kind,
        EpisodeKind::Note
    );
    assert_eq!(
        store
            .list_memories(sovra_core::MemoryListQuery::default())
            .await
            .unwrap()
            .overview,
        overview
    );
    drop(store);
    // Windows also needs the engine's asynchronous shutdown to release its handles.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("remove the test database after the engine releases its handles");
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
    assert_eq!(store.episode(&recent[0].id).await.unwrap(), recent[0]);
    assert!(matches!(
        store
            .episode(&sovra_core::EpisodeId("episode:absent".into()))
            .await,
        Err(MemoryError::NotFound(_))
    ));
    assert!(matches!(
        store
            .episode(&sovra_core::EpisodeId("fact:absent".into()))
            .await,
        Err(MemoryError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn episode_writes_and_reads_are_bounded() {
    let store = open_store().await;
    assert!(
        store
            .record_episode(episode("", "no owner", 1))
            .await
            .is_err()
    );
    assert!(store.record_episode(episode("s1", " ", 1)).await.is_err());
    assert!(
        store
            .record_episode(episode("s1", &"x".repeat(128 * 1024 + 1), 1))
            .await
            .is_err()
    );
    assert!(store.recent_episodes("s1", usize::MAX).await.is_err());
    assert!(store.recent_episodes("", 10).await.is_err());
    assert_eq!(store.recent_episodes("s1", 0).await.unwrap(), Vec::new());
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
            vector: vec![1.0; DIM + 1],
        }),
        ..fact("bad", &[], None)
    };
    match store.save_fact(bad).await {
        Err(MemoryError::EmbeddingMismatch { dimension, .. }) => assert_eq!(dimension, DIM),
        other => panic!("expected EmbeddingMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn chat_threads_save_list_and_soft_delete() {
    let store = open_store().await;
    let mut thread1 = sovra_core::ChatThread::new("chat-1", Timestamp(1_000));
    thread1.title = "First conversation".into();
    thread1
        .messages
        .push(sovra_core::ChatMessage::user("m1", "Hello", "Just now"));
    thread1.refresh_summary(Timestamp(2_000));

    let mut thread2 = sovra_core::ChatThread::new_scheduled(
        "sched-run-1",
        "sched-morning-brief",
        "Executive Morning Brief",
        Timestamp(3_000),
    );
    thread2.messages.push(sovra_core::ChatMessage::assistant(
        "m2",
        "Morning brief ready.",
        "Just now",
        vec![],
    ));
    thread2.messages[0].model = Some(sovra_core::ResponseModel {
        model_id: "provider/model".into(),
        name: "Friendly model".into(),
        provider: "OpenRouter".into(),
    });
    thread2.refresh_summary(Timestamp(4_000));

    store
        .save_chat_thread(&thread1)
        .await
        .expect("save thread1");
    store
        .save_chat_thread(&thread2)
        .await
        .expect("save thread2");

    let listed = store.list_chat_threads(10).await.expect("list threads");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "sched-run-1");
    assert_eq!(listed[0].messages, thread2.messages);
    assert_eq!(listed[0].kind, sovra_core::ConversationKind::Scheduled);
    assert_eq!(listed[1].id, "chat-1");
    assert_eq!(listed[1].messages.len(), 1);

    store
        .delete_chat_thread("chat-1", Timestamp(5_000))
        .await
        .expect("soft delete");
    let after_delete = store
        .list_chat_threads(10)
        .await
        .expect("list after delete");
    assert_eq!(after_delete.len(), 1);
    assert_eq!(after_delete[0].id, "sched-run-1");
}

#[test]
fn older_chat_messages_load_without_inventing_a_model() {
    let message: sovra_core::ChatMessage = serde_json::from_value(serde_json::json!({
        "id":"old", "role":"assistant", "author":"SOVRA", "timestamp":"Yesterday",
        "text":"Earlier reply", "widgets":[],
    }))
    .unwrap();
    assert!(message.model.is_none());
}

#[tokio::test]
async fn team_roster_and_artifacts_survive_and_soft_delete() {
    use sovra_core::artifacts::{Artifact, ArtifactDraft, ArtifactKind, ArtifactSource};
    let store = open_store().await;
    assert_eq!(store.load_team().await.expect("empty team"), None);
    let mut team = sovra_core::teams::default_team();
    team[1].name = "Morgana".into();
    store.save_team(&team).await.expect("save team");
    assert_eq!(store.load_team().await.expect("load team"), Some(team));

    let draft = |title: &str| ArtifactDraft {
        title: title.into(),
        kind: ArtifactKind::Code,
        language: "python".into(),
        content: "print('hi')".into(),
        source: ArtifactSource::default(),
    };
    let mut first = Artifact::from_draft("art-1".into(), draft("First"), Timestamp(1));
    let second = Artifact::from_draft("art-2".into(), draft("Second"), Timestamp(2));
    store.save_artifact(&first).await.expect("save first");
    store.save_artifact(&second).await.expect("save second");
    first.edit("First, edited", "print('bye')", Timestamp(3));
    store.save_artifact(&first).await.expect("update first");
    let listed = store.list_artifacts().await.expect("list artifacts");
    assert_eq!(listed, [first.clone(), second]);
    assert_eq!(listed[0].versions.len(), 1);
    store
        .delete_artifact("art-2", Timestamp(4))
        .await
        .expect("delete");
    assert_eq!(store.list_artifacts().await.expect("list"), [first]);
}

fn memory_input(session: &str, key: &str) -> sovra_core::MemoryInput {
    sovra_core::MemoryInput {
        key: key.into(),
        episode: episode(session, "I live in Lisbon and I prefer tea", 1_000),
    }
}

fn candidate(content: &str) -> sovra_core::MemoryCandidate {
    sovra_core::MemoryCandidate {
        content: content.into(),
        evidence: "I live in Lisbon".into(),
        entities: vec!["Lisbon".into()],
        valid_at: None,
        replaces: None,
        same_as: None,
    }
}

#[tokio::test]
async fn memory_inputs_are_immutable_idempotent_and_scoped_to_the_conversation() {
    let store = open_store().await;
    let input = memory_input("s1", "message-1");
    let id = store.enqueue_memory(input.clone()).await.unwrap();
    assert_eq!(store.enqueue_memory(input.clone()).await.unwrap(), id);
    assert_eq!(store.recent_episodes("s1", 10).await.unwrap().len(), 1);
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Pending
    );
    let mut changed = input.clone();
    changed.episode.content = "different evidence".into();
    assert!(store.enqueue_memory(changed).await.is_err());
    let mut other = input.clone();
    other.episode.session_id = "s2".into();
    assert_ne!(store.enqueue_memory(other).await.unwrap(), id);
    let mut generated = input;
    generated.key = "generated".into();
    generated.episode.kind = EpisodeKind::ToolResult;
    assert!(store.enqueue_memory(generated).await.is_err());
    assert_eq!(store.recent_episodes("s1", 10).await.unwrap().len(), 1);
    assert!(matches!(
        store
            .memory_job(&sovra_core::EpisodeId("episode:missing".into()))
            .await,
        Err(MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn ingestion_batch_is_atomic_idempotent_and_preserves_evidence() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    assert_eq!(claim.episode.id, id);
    assert_eq!(claim.attempt, 1);
    assert!(
        store
            .claim_memory(Timestamp(2_001))
            .await
            .unwrap()
            .is_none()
    );
    let candidates = vec![candidate("User lives in Lisbon")];
    let revision = store.memory_revision().await.unwrap();
    let ids = store
        .complete_memory(&claim, candidates.clone(), "test-model", Timestamp(2_010))
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    assert!(
        store.memory_revision().await.unwrap() > revision,
        "saving facts must still invalidate older recall results"
    );
    assert_eq!(
        store
            .complete_memory(&claim, candidates, "test-model", Timestamp(2_020))
            .await
            .unwrap(),
        ids
    );
    assert!(
        store
            .complete_memory(
                &claim,
                vec![candidate("different output")],
                "test-model",
                Timestamp(2_020)
            )
            .await
            .is_err()
    );
    let job = store.memory_job(&id).await.unwrap();
    assert_eq!(job.state, sovra_core::MemoryJobState::Complete);
    assert_eq!(job.fact_ids, ids);
    let fact = store
        .recall(RecallQuery::recent(1))
        .await
        .unwrap()
        .remove(0)
        .fact;
    assert_eq!(fact.source, Some(id));
    assert_eq!(
        fact.extraction,
        Some(sovra_core::MemoryExtraction {
            model: "test-model".into(),
            evidence: "I live in Lisbon".into(),
        })
    );
}

#[tokio::test]
async fn ingestion_rejects_fabricated_evidence_and_rolls_back_the_entire_batch() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let mut fabricated = candidate("User lives in Rome");
    fabricated.evidence = "I live in Rome".into();
    let mut forged = claim.clone();
    forged.episode.content = fabricated.evidence.clone();
    assert!(
        store
            .complete_memory(&forged, vec![fabricated], "test-model", Timestamp(2_010))
            .await
            .is_err()
    );
    let mut stale = candidate("User prefers tea");
    stale.evidence = "I prefer tea".into();
    stale.replaces = Some(sovra_core::FactId("fact:missing".into()));
    assert!(
        store
            .complete_memory(
                &claim,
                vec![candidate("User lives in Lisbon"), stale],
                "test-model",
                Timestamp(2_010)
            )
            .await
            .is_err()
    );
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
    let job = store.memory_job(&id).await.unwrap();
    assert_eq!(job.state, sovra_core::MemoryJobState::Processing);
    assert_eq!(job.fact_ids, Vec::new());
    store
        .complete_memory(
            &claim,
            vec![candidate("User lives in Lisbon")],
            "test-model",
            Timestamp(2_020),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_leases_recover_and_old_workers_cannot_write() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let first = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let second = store
        .claim_memory(first.lease_until)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.token, second.token);
    assert_eq!(second.attempt, 2);
    assert!(
        store
            .complete_memory(
                &first,
                vec![candidate("old worker")],
                "test-model",
                first.lease_until
            )
            .await
            .is_err()
    );
    let third = store
        .claim_memory(second.lease_until)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third.attempt, sovra_core::MAX_MEMORY_ATTEMPTS);
    assert!(
        store
            .claim_memory(third.lease_until)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Failed
    );
}

#[tokio::test]
async fn failed_extraction_backs_off_then_stops_with_an_inspectable_error() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let mut now = Timestamp(2_000);
    for attempt in 1..=sovra_core::MAX_MEMORY_ATTEMPTS {
        let claim = store.claim_memory(now).await.unwrap().unwrap();
        assert_eq!(claim.attempt, attempt);
        store
            .fail_memory(&claim, "The model returned incomplete data.", now)
            .await
            .unwrap();
        assert!(store.claim_memory(now).await.unwrap().is_none());
        now = Timestamp(now.millis() + sovra_core::MEMORY_RETRY_MILLIS);
    }
    assert!(store.claim_memory(now).await.unwrap().is_none());
    let job = store.memory_job(&id).await.unwrap();
    assert_eq!(job.state, sovra_core::MemoryJobState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("The model returned incomplete data.")
    );
}

#[tokio::test]
async fn pausing_memory_revokes_pending_and_inflight_work_even_after_resuming() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let pending = store
        .enqueue_memory(memory_input("s2", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let before = store.memory_policy().await.unwrap();
    assert!(before.enabled);
    let paused = store.set_memory_enabled(false).await.unwrap();
    assert!(!paused.enabled);
    assert!(paused.generation > before.generation);
    assert_eq!(store.set_memory_enabled(false).await.unwrap(), paused);
    assert!(
        store
            .enqueue_memory(memory_input("s3", "m1"))
            .await
            .is_err()
    );
    assert!(
        store
            .claim_memory(Timestamp(2_001))
            .await
            .unwrap()
            .is_none()
    );
    store.set_memory_enabled(true).await.unwrap();
    assert!(
        store
            .complete_memory(
                &claim,
                vec![candidate("too late")],
                "test-model",
                Timestamp(2_010)
            )
            .await
            .is_err()
    );
    assert!(
        store
            .fail_memory(&claim, "late failure", Timestamp(2_010))
            .await
            .is_err()
    );
    for id in [&id, &pending] {
        assert_eq!(
            store.memory_job(id).await.unwrap().state,
            sovra_core::MemoryJobState::Cancelled
        );
    }
    assert_eq!(
        store
            .enqueue_memory(memory_input("s1", "m1"))
            .await
            .unwrap(),
        id
    );
    assert!(
        store
            .claim_memory(Timestamp(2_020))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn repeated_evidence_does_not_duplicate_or_reactivate_a_stopped_memory() {
    let store = open_store().await;
    store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let ids = store
        .complete_memory(
            &claim,
            vec![candidate("User lives in Lisbon")],
            "test-model",
            Timestamp(2_010),
        )
        .await
        .unwrap();
    store
        .invalidate_fact(&ids[0], Timestamp(3_000))
        .await
        .unwrap();
    store
        .enqueue_memory(memory_input("s2", "m1"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(4_000)).await.unwrap().unwrap();
    assert_eq!(
        store
            .complete_memory(
                &claim,
                vec![candidate("User lives in Lisbon")],
                "test-model",
                Timestamp(4_010)
            )
            .await
            .unwrap(),
        ids
    );
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap(),
        Vec::new()
    );
    let mut history = RecallQuery::recent(10);
    history.include_expired = true;
    assert_eq!(store.recall(history).await.unwrap().len(), 1);
}

#[tokio::test]
async fn durable_queue_keeps_evidence_beyond_the_old_capacity() {
    let store = open_store().await;
    for index in 0..=sovra_core::MAX_PENDING_MEMORIES {
        store
            .enqueue_memory(memory_input("s1", &format!("m{index}")))
            .await
            .unwrap();
    }
    let id = store
        .enqueue_memory(memory_input("overflow", "m1"))
        .await
        .unwrap();
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Pending
    );
    assert_eq!(
        store.recent_episodes("overflow", 10).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn competing_workers_cannot_claim_the_same_input() {
    let store = open_store().await;
    store
        .enqueue_memory(memory_input("s1", "m1"))
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        store.claim_memory(Timestamp(2_000)),
        store.claim_memory(Timestamp(2_000))
    );
    let claims: Vec<_> = [first, second]
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect();
    assert_eq!(claims.len(), 1);
    assert!(
        store
            .claim_memory(Timestamp(2_001))
            .await
            .unwrap()
            .is_none()
    );
    store
        .complete_memory(&claims[0], Vec::new(), "test-model", Timestamp(2_010))
        .await
        .unwrap();
}

#[tokio::test]
async fn duplicate_candidates_keep_one_fact_and_each_jobs_extraction_evidence() {
    let store = open_store().await;
    for (session, at) in [("s1", 2_000), ("s2", 3_000)] {
        let id = store
            .enqueue_memory(memory_input(session, "m1"))
            .await
            .unwrap();
        let claim = store.claim_memory(Timestamp(at)).await.unwrap().unwrap();
        let candidates = vec![
            candidate("User lives in Lisbon"),
            candidate("User lives in Lisbon"),
        ];
        let ids = store
            .complete_memory(&claim, candidates.clone(), "test-model", Timestamp(at + 1))
            .await
            .unwrap();
        assert_eq!(ids.len(), 1);
        let job = store.memory_job(&id).await.unwrap();
        assert_eq!(job.extraction_model.as_deref(), Some("test-model"));
        assert_eq!(job.candidates, candidates);
    }
    assert_eq!(
        store.recall(RecallQuery::recent(10)).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn pausing_rejects_late_raw_episode_writes_as_well_as_extraction() {
    let store = open_store().await;
    store.set_memory_enabled(false).await.unwrap();
    let mut reply = episode("s1", "late reply", 1);
    reply.kind = EpisodeKind::AgentMessage;
    assert!(store.record_episode(reply).await.is_err());
    assert_eq!(store.recent_episodes("s1", 10).await.unwrap(), Vec::new());
    store.set_memory_enabled(true).await.unwrap();
    store
        .record_episode(episode("s1", "new evidence", 2))
        .await
        .unwrap();
    assert_eq!(store.recent_episodes("s1", 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn recall_applies_valid_and_known_times_before_limiting() {
    let store = open_store().await;
    let mut original = fact("Maya prefers tea", &["Maya"], None);
    original.valid_at = Some(Timestamp(100));
    let old_id = store.save_fact(original).await.unwrap();
    let old = store
        .recall(RecallQuery::recent(1))
        .await
        .unwrap()
        .remove(0)
        .fact;
    let mut replacement = fact("Maya prefers coffee", &["Maya"], None);
    replacement.valid_at = Some(Timestamp(200));
    replacement.replaces = Some(old_id.clone());
    let new_id = store.save_fact(replacement).await.unwrap();
    let mut query = RecallQuery::recent(1);
    query.include_expired = true;
    query.valid_at = Some(Timestamp(150));
    query.text = Some("prefers".into());
    query.entity = Some("Maya".into());
    let found = store.recall(query.clone()).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].fact.id, old_id);
    query.valid_at = Some(Timestamp(250));
    assert_eq!(
        store.recall(query.clone()).await.unwrap()[0].fact.id,
        new_id
    );
    query.known_at = Some(old.created_at);
    query.valid_at = Some(Timestamp(250));
    assert_eq!(
        store.recall(query.clone()).await.unwrap()[0].fact.id,
        old_id,
        "a later correction must not rewrite what was known earlier"
    );
    query.valid_at = Some(Timestamp(150));
    assert_eq!(
        store.recall(query.clone()).await.unwrap()[0].fact.id,
        old_id
    );
    query.known_at = Some(Timestamp(old.created_at.millis() - 1));
    assert_eq!(store.recall(query.clone()).await.unwrap(), Vec::new());
    query.known_at = None;
    query.valid_at = Some(Timestamp(50));
    assert_eq!(store.recall(query).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn long_sources_resume_chunks_and_reject_stale_fact_revisions() {
    let store = open_store().await;
    let mut input = memory_input("chunks", "dense");
    input.episode.content = "I like tea. ".repeat(400);
    let id = store.enqueue_memory(input).await.unwrap();
    let mut first = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    assert_eq!(first.cursor, 0);
    assert!(first.chunk().unwrap().content.len() <= sovra_core::MEMORY_CHUNK_BYTES);
    store
        .remember_note(sovra_core::MemoryNote {
            text: "A manual correction arrived".into(),
        })
        .await
        .unwrap();
    assert!(
        store
            .complete_memory(&first, vec![], "fixture", Timestamp(2_001))
            .await
            .is_err()
    );
    first.revision = store.memory_revision().await.unwrap();
    store
        .complete_memory(&first, vec![], "fixture", Timestamp(2_002))
        .await
        .unwrap();
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Pending
    );
    let second = store.claim_memory(Timestamp(2_003)).await.unwrap().unwrap();
    assert_eq!(second.cursor, first.chunk().unwrap().content.len());
    assert_eq!(second.attempt, 1);
    assert_eq!(second.episode.id, id);
}

#[tokio::test]
async fn sources_in_one_conversation_are_claimed_in_order() {
    let store = open_store().await;
    for (session, key) in [
        ("ordered", "one"),
        ("ordered", "two"),
        ("independent", "three"),
    ] {
        store
            .enqueue_memory(memory_input(session, key))
            .await
            .unwrap();
    }
    let first = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let second = store.claim_memory(Timestamp(2_001)).await.unwrap().unwrap();
    assert_ne!(first.episode.session_id, second.episode.session_id);
    assert!(
        store
            .claim_memory(Timestamp(2_002))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn attributed_admission_keeps_assistant_evidence_out_of_personal_extraction() {
    use sovra_core::{
        MemorySourceAdmission, MemorySourceAuthor, MemorySourceOrigin, MemorySourceProvenance,
        MemorySourceScope,
    };
    let store = open_store().await;
    let mut source = MemorySourceAdmission {
        dependencies: None,
        expected_generation: None,
        key: "assistant-reply-1".into(),
        version: 1,
        author: MemorySourceAuthor::Assistant {
            speaker: Some("Planning helper".into()),
        },
        scope: MemorySourceScope::Private,
        provenance: MemorySourceProvenance::Conversation,
        episode: NewEpisode {
            session_id: "attributed-chat".into(),
            kind: EpisodeKind::AgentMessage,
            content: "I suggest cedar tea.".into(),
            occurred_at: Timestamp(1),
        },
    };
    let first = store.admit_memory_source(source.clone()).await.unwrap();
    assert!(!first.has_processing_job);
    assert_eq!(
        store.admit_memory_source(source.clone()).await.unwrap(),
        first
    );
    let record = store.review_memory_source(&first.source_id).await.unwrap();
    assert_eq!(record.metadata.origin, MemorySourceOrigin::AssistantOutput);
    assert_eq!(record.metadata.scope, MemorySourceScope::Private);
    assert_eq!(record.metadata.version, Some(1));
    assert_eq!(record.metadata.processing, None);
    assert!(record.metadata.admitted_at.is_some());
    let attribution = record.metadata.attribution.unwrap();
    assert_eq!(attribution.author, source.author);
    assert_eq!(attribution.provenance, source.provenance);
    assert_eq!(attribution.producer_key, source.key);
    assert!(store.claim_memory(Timestamp(10)).await.unwrap().is_none());
    assert!(
        store
            .search_memory_sources(
                sovra_core::MemorySearch {
                    text: "cedar".into(),
                    ..Default::default()
                },
                Timestamp(10)
            )
            .await
            .unwrap()
            .is_empty(),
        "assistant recall requires dependency-aware withdrawal first"
    );
    source.episode.content = "I suggest mint tea.".into();
    assert!(store.admit_memory_source(source.clone()).await.is_err());
    source.version = 2;
    let second = store.admit_memory_source(source.clone()).await.unwrap();
    assert_ne!(first.source_id, second.source_id);
    assert_eq!(
        store.episode(&first.source_id).await.unwrap().content,
        "I suggest cedar tea."
    );
    assert_eq!(
        store
            .review_memory_source(&second.source_id)
            .await
            .unwrap()
            .metadata
            .version,
        Some(2)
    );
    store
        .stop_memory_source(&second.source_id, Timestamp(11))
        .await
        .unwrap();
    assert_eq!(
        store.admit_memory_source(source.clone()).await.unwrap(),
        second
    );
    assert_eq!(
        store
            .review_memory_source(&second.source_id)
            .await
            .unwrap()
            .metadata
            .revoked_at,
        Some(Timestamp(11))
    );
    assert_eq!(
        store
            .review_memory_source(&first.source_id)
            .await
            .unwrap()
            .metadata
            .revoked_at,
        Some(Timestamp(11))
    );
    source.version = 3;
    assert!(store.admit_memory_source(source.clone()).await.is_err());
    store.set_memory_enabled(false).await.unwrap();
    source.key = "another-reply".into();
    assert!(store.admit_memory_source(source).await.is_err());
}

#[tokio::test]
async fn distinctive_source_term_keeps_a_place_in_the_provider_evidence_budget() {
    let store = open_store().await;
    let mut original = None;
    for index in 0..1_000 {
        let mut input = memory_input("distinctive-ranking", &format!("code-{index}"));
        input.episode.content = format!("My rehearsal code is fixturecode{index}.");
        input.episode.occurred_at = Timestamp(1);
        let id = store.enqueue_memory(input).await.unwrap();
        if index == 100 {
            original = Some(id);
        }
    }
    for (index, code) in ["101", "102", "103", "104", "100"].into_iter().enumerate() {
        let mut input = memory_input("distinctive-ranking", &format!("question-{index}"));
        input.episode.content = format!("Recall fixturecode{code}");
        input.episode.occurred_at = Timestamp(i64::try_from(index).unwrap() + 2);
        store.enqueue_memory(input).await.unwrap();
    }
    let sources = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "Recall fixturecode100".into(),
                ..Default::default()
            },
            Timestamp(100),
        )
        .await
        .unwrap();
    let mut context = sovra_core::MemoryContext::default();
    for source in sources {
        context.push_source(source);
    }
    assert!(
        context
            .sources
            .iter()
            .any(|source| Some(&source.source_id) == original.as_ref()),
        "a distinguishing term must retain evidence even when common-word questions are shorter"
    );
}

#[tokio::test]
async fn relevant_original_evidence_survives_newer_questions_with_a_common_search_word() {
    let store = open_store().await;
    let mut input = memory_input("source-ranking", "answer");
    input.episode.content = "My rehearsal code is fixturecode100.".into();
    input.episode.occurred_at = Timestamp(1);
    let original = store.enqueue_memory(input).await.unwrap();
    for index in 0..40 {
        let mut input = memory_input("source-ranking", &format!("question-{index}"));
        input.episode.content = format!("Recall fixturecode{index}");
        input.episode.occurred_at = Timestamp(index + 2);
        store.enqueue_memory(input).await.unwrap();
    }
    let sources = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "Recall fixturecode100".into(),
                ..Default::default()
            },
            Timestamp(100),
        )
        .await
        .unwrap();
    assert_eq!(
        sources.first().map(|source| &source.source_id),
        Some(&original)
    );
    assert!(sources.len() <= 12);
    store
        .stop_memory_source(&original, Timestamp(101))
        .await
        .unwrap();
    let sources = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "Recall fixturecode100".into(),
                ..Default::default()
            },
            Timestamp(102),
        )
        .await
        .unwrap();
    assert!(sources.iter().all(|source| source.source_id != original));
}

#[tokio::test]
async fn original_evidence_is_retrievable_without_facts_but_pause_and_withdrawal_win() {
    use sovra_core::{MemorySearch, MemorySourceOrigin};
    let store = open_store().await;
    let mut input = memory_input("source-only", "omitted");
    input.episode.content = "The rehearsal code is cedar and the room is upstairs.".into();
    let id = store.enqueue_memory(input).await.unwrap();
    let query = MemorySearch {
        text: "cedar".into(),
        ..Default::default()
    };
    let sources = store
        .search_memory_sources(query.clone(), Timestamp(9_000))
        .await
        .unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_id, id);
    assert_eq!(sources[0].origin, MemorySourceOrigin::UserStatement);
    assert!(sources[0].quote.contains("cedar"));
    assert_eq!(
        store
            .memory_episodes(std::slice::from_ref(&id))
            .await
            .unwrap()
            .len(),
        1
    );
    store.set_memory_enabled(false).await.unwrap();
    assert_eq!(
        store
            .search_memory_sources(query.clone(), Timestamp(9_000))
            .await
            .unwrap(),
        Vec::new()
    );
    store.set_memory_enabled(true).await.unwrap();
    let mut detail = fact("The rehearsal code is cedar", &[], None);
    detail.source = Some(id);
    let detail = store.save_fact(detail).await.unwrap();
    store.stop_memory(&detail, Timestamp(9_001)).await.unwrap();
    assert_eq!(
        store
            .search_memory_sources(query, Timestamp(9_002))
            .await
            .unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn chunk_progress_and_unextracted_evidence_survive_disk_reopen() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sovra-source-reopen-{}-{unique}",
        std::process::id()
    ));
    let config = StoreConfig::on_disk(&path);
    let store = SurrealStore::open(config.clone()).await.unwrap();
    let mut input = memory_input("restart-source", "one");
    input.episode.content = format!(
        "{} The omitted code is cedarlock.",
        "A retained detail. ".repeat(300)
    );
    let id = store.enqueue_memory(input).await.unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    let end = claim.chunk().unwrap().content.len();
    store
        .complete_memory(&claim, vec![], "fixture", Timestamp(2_001))
        .await
        .unwrap();
    let second = store.claim_memory(Timestamp(2_002)).await.unwrap().unwrap();
    let extracted = sovra_core::MemoryExtractedChunk {
        candidates: vec![],
        model: "fixture".into(),
    };
    store
        .save_memory_extraction(&second, &extracted, Timestamp(2_003))
        .await
        .unwrap();
    drop(store);
    let store = reopen(config).await;
    let next = store
        .claim_memory(Timestamp(200_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.cursor, end);
    assert_eq!(next.extracted, Some(extracted));
    let sources = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "cedarlock".into(),
                ..Default::default()
            },
            Timestamp(9_000),
        )
        .await
        .unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_id, id);
    assert!(sources[0].quote.contains("cedarlock"));
    assert_eq!(store.memory_job(&id).await.unwrap().fact_ids, Vec::new());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn revoking_original_evidence_cancels_its_lease_and_preserves_review_content() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("withdraw-source", "one"))
        .await
        .unwrap();
    let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
    store
        .stop_memory_source(&id, Timestamp(2_001))
        .await
        .unwrap();
    store
        .stop_memory_source(&id, Timestamp(2_002))
        .await
        .unwrap();
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Cancelled
    );
    assert!(
        store
            .complete_memory(&claim, vec![], "fixture", Timestamp(2_003))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .search_memory_sources(
                sovra_core::MemorySearch {
                    text: "Lisbon".into(),
                    ..Default::default()
                },
                Timestamp(9_000)
            )
            .await
            .unwrap(),
        Vec::new()
    );
    assert_eq!(
        store.episode(&id).await.unwrap().content,
        claim.episode.content
    );
}

#[tokio::test]
async fn withdrawn_failed_evidence_cannot_be_retried() {
    let store = open_store().await;
    let id = store
        .enqueue_memory(memory_input("failed-withdrawal", "one"))
        .await
        .unwrap();
    for attempt in 0..sovra_core::MAX_MEMORY_ATTEMPTS {
        let now = Timestamp(2_000 + i64::from(attempt) * 20_000);
        let claim = store.claim_memory(now).await.unwrap().unwrap();
        store
            .fail_memory(&claim, "fixture failure", now)
            .await
            .unwrap();
    }
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Failed
    );
    store
        .stop_memory_source(&id, Timestamp(90_000))
        .await
        .unwrap();
    assert_eq!(store.retry_failed_memories().await.unwrap(), 0);
    assert_eq!(
        store.memory_job(&id).await.unwrap().state,
        sovra_core::MemoryJobState::Cancelled
    );
}

#[tokio::test]
async fn immutable_source_versions_keep_exact_citations_and_withdraw_together() {
    let store = open_store().await;
    let mut source =
        sovra_core::MemorySourceAdmission::user(memory_input("version-chat", "message"));
    source.episode.content = "My rehearsal code is cedarlock.".into();
    let first = store.admit_memory_source(source.clone()).await.unwrap();
    source.version = 2;
    source.episode.content = "My rehearsal code is mintlock.".into();
    let second = store.admit_memory_source(source).await.unwrap();
    let hits = store
        .search_memory_sources(
            sovra_core::MemorySearch {
                text: "mintlock".into(),
                ..Default::default()
            },
            Timestamp(9_000),
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].version, 2);
    assert_eq!(hits[0].source_id, second.source_id);
    assert_eq!(hits[0].citation, format!("source:{}", second.source_id.0));
    store
        .stop_memory_source(&first.source_id, Timestamp(10_000))
        .await
        .unwrap();
    for receipt in [first, second] {
        assert_eq!(
            store.memory_job(&receipt.source_id).await.unwrap().state,
            sovra_core::MemoryJobState::Cancelled
        );
        assert_eq!(
            store
                .review_memory_source(&receipt.source_id)
                .await
                .unwrap()
                .metadata
                .revoked_at,
            Some(Timestamp(10_000))
        );
    }
    assert!(
        store
            .claim_memory(Timestamp(11_000))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn source_capture_cannot_cross_pause_resume_even_when_admission_runs_later() {
    let store = open_store().await;
    let policy = store.memory_policy().await.unwrap();
    let mut source = sovra_core::MemorySourceAdmission::user(sovra_core::MemoryInput {
        key: "late-source".into(),
        episode: NewEpisode {
            session_id: "capture-test".into(),
            kind: EpisodeKind::UserMessage,
            content: "Fictional source captured before a pause.".into(),
            occurred_at: Timestamp(1),
        },
    });
    source.expected_generation = Some(policy.generation);
    store.set_memory_enabled(false).await.unwrap();
    let resumed = store.set_memory_enabled(true).await.unwrap();
    assert!(store.admit_memory_source(source.clone()).await.is_err());
    let page = store
        .list_memory_sources(sovra_core::MemorySourceQuery::default())
        .await
        .unwrap();
    assert_eq!(page.sources, Vec::new());
    source.expected_generation = Some(resumed.generation);
    let receipt = store.admit_memory_source(source.clone()).await.unwrap();
    assert_eq!(
        store.admit_memory_source(source.clone()).await.unwrap(),
        receipt
    );
    store.set_memory_enabled(false).await.unwrap();
    assert!(store.admit_memory_source(source).await.is_err());
}
