//! Immutable, bounded dependency closures keep helper paraphrases revocable.

/// Runs inside source admission. Existing helper closures are immutable, so one
/// bounded expansion includes all ancestors without recursive retrieval queries.
/// Missing or unknown inputs never become trusted later just because they appear.
pub(super) const RESOLVE: &str = "
LET $seed_sources = array::union($requested_sources,
    array::flatten((SELECT VALUE ->derived_from->episode.id FROM $requested_facts)));
LET $dependency_sources = array::union($seed_sources,
    array::flatten((SELECT VALUE dependency_sources ?? [] FROM $seed_sources)));
LET $dependency_facts = array::union($requested_facts,
    array::flatten((SELECT VALUE dependency_facts ?? [] FROM $seed_sources)));
LET $dependency_complete = $dependencies_supplied
    AND array::len($dependency_sources) + array::len($dependency_facts) <= $dependency_limit
    AND $episode NOT IN $dependency_sources
    AND array::len((SELECT VALUE id FROM $dependency_sources
        WHERE permission_scope = 'private' AND source_version > 0
        AND (origin = 'user_statement' OR (origin = 'assistant_output' AND dependency_complete = true))))
        = array::len($dependency_sources)
    AND array::len((SELECT VALUE id FROM $dependency_facts)) = array::len($dependency_facts);
";

#[cfg(test)]
mod tests {
    use crate::{StoreConfig, SurrealStore};
    use sovra_core::{
        EpisodeKind, MemoryInput, MemorySearch, MemorySourceAdmission, MemorySourceAuthor,
        MemoryStore, NewEpisode, NewFact, Timestamp,
    };

    #[tokio::test]
    async fn version_ten_preserves_chat_json_and_defaults_missing_source_links_to_unknown() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let mut thread = sovra_core::ChatThread::new("legacy-chat", Timestamp(100));
        let mut message = sovra_core::ChatMessage::user("user", "Original wording", "");
        message.source = Some(sovra_core::EpisodeId("episode:original".into()));
        message.occurred_at = Some(Timestamp(100));
        thread.messages.push(message);
        store.save_chat_thread(&thread).await.unwrap();
        let mut legacy = serde_json::to_value(&thread.messages).unwrap();
        legacy[0].as_object_mut().unwrap().remove("source");
        legacy[0].as_object_mut().unwrap().remove("occurred_at");
        let legacy = serde_json::to_string(&legacy).unwrap();
        store.query("UPDATE chat_thread SET messages_json = $json; UPDATE meta:schema SET version = 10;", ("json", legacy.clone())).await.unwrap();
        store.apply_schema().await.unwrap();
        store.apply_schema().await.unwrap();
        let restored = store.list_chat_threads(10).await.unwrap();
        assert_eq!(restored[0].messages[0].text, "Original wording");
        assert_eq!(restored[0].messages[0].source, None);
        assert_eq!(restored[0].messages[0].occurred_at, None);
        let raw: Vec<String> = store
            .query(
                "SELECT VALUE messages_json FROM chat_thread;",
                surrealdb::types::Object::default(),
            )
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(raw, vec![legacy]);
        store.save_chat_thread(&thread).await.unwrap();
        assert_eq!(store.list_chat_threads(10).await.unwrap(), vec![thread]);
    }

    #[tokio::test]
    async fn version_nine_migration_preserves_unknown_dependencies_and_fact_provenance() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let mut source = MemorySourceAdmission::user(MemoryInput {
            key: "legacy-helper".into(),
            episode: NewEpisode {
                session_id: "migration".into(),
                kind: EpisodeKind::UserMessage,
                content: "A legacy juniper suggestion.".into(),
                occurred_at: Timestamp(1),
            },
        });
        let user = store
            .admit_memory_source(source.clone())
            .await
            .unwrap()
            .source_id;
        let fact = store
            .save_fact(NewFact {
                content: "User discussed juniper.".into(),
                entities: vec![],
                embedding: None,
                source: Some(user.clone()),
                valid_at: None,
                replaces: None,
            })
            .await
            .unwrap();
        source.key = "helper".into();
        source.author = MemorySourceAuthor::Assistant { speaker: None };
        source.episode.kind = EpisodeKind::AgentMessage;
        let helper = store.admit_memory_source(source).await.unwrap().source_id;
        store.query("UPDATE episode UNSET dependency_complete, dependency_sources, dependency_facts, dependency_manifest; UPDATE meta:schema SET version = 9;", surrealdb::types::Object::default()).await.unwrap();
        store.apply_schema().await.unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.memory_fact(&fact).await.unwrap().source,
            Some(user.clone())
        );
        assert_eq!(
            store.episode(&helper).await.unwrap().content,
            "A legacy juniper suggestion."
        );
        let hits = store
            .search_memory_sources(
                MemorySearch {
                    text: "juniper".into(),
                    ..Default::default()
                },
                Timestamp(i64::MAX),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, user);
        let complete: Vec<bool> = store
            .query(
                "SELECT VALUE dependency_complete FROM episode;",
                surrealdb::types::Object::default(),
            )
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(complete, vec![false, false]);
    }
}
