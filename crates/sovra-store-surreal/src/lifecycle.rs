//! Keyset browsing and atomic corrections with an immutable user-authored source.

use sovra_core::{
    Fact, FactId, MemoryCorrection, MemoryCursor, MemoryError, MemoryListQuery, MemoryNote,
    MemoryPage, NewFact, Timestamp,
};
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

#[derive(Debug, SurrealValue)]
struct EpisodeChatRow {
    id: RecordId,
    session_id: String,
}

use super::{FactRow, NewEpisodeRow, SurrealStore, backend, facts, parse_id};

impl SurrealStore {
    pub(super) async fn connected_memories(
        &self,
        id: &FactId,
    ) -> Result<sovra_core::MemoryNeighborhood, MemoryError> {
        let anchor = self.exact_fact(id).await?;
        let subjects = anchor
            .entities
            .iter()
            .filter(|name| !name.eq_ignore_ascii_case("user"))
            .cloned()
            .collect::<Vec<_>>();
        let mut result = self.query(&format!(
            "SELECT {} FROM fact WHERE id = $previous OR (->supersedes->fact.id) CONTAINS $anchor ORDER BY created_at DESC, id ASC LIMIT 3;
             SELECT {} FROM fact WHERE id != $anchor AND stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE
             AND (($source IS NOT NONE AND id IN (SELECT VALUE in FROM derived_from WHERE out = $source))
                 OR id IN (SELECT VALUE in FROM mentions WHERE out IN (SELECT VALUE id FROM entity WHERE name IN $subjects)))
             ORDER BY created_at DESC, id ASC LIMIT 25;", facts::FIELDS, facts::FIELDS),
            ("anchor", parse_id(&id.0, "fact")?, "previous", anchor.replaces.as_ref().map(|id| parse_id(&id.0, "fact")).transpose()?,
             "source", anchor.source.as_ref().map(|id| parse_id(&id.0, "episode")).transpose()?, "subjects", subjects),
        ).await?;
        let versions: Vec<FactRow> = result.take(0).map_err(|error| backend(&error))?;
        let related: Vec<FactRow> = result.take(1).map_err(|error| backend(&error))?;
        let truncated = versions.len() == 3 || related.len() == 25;
        let mut neighborhood = sovra_core::memory_neighborhood(
            &anchor,
            versions
                .into_iter()
                .chain(related)
                .map(FactRow::into_fact)
                .collect(),
        );
        neighborhood.has_more |= truncated;
        Ok(neighborhood)
    }

    pub(super) async fn memory_page(
        &self,
        query: MemoryListQuery,
    ) -> Result<MemoryPage, MemoryError> {
        query.validate()?;
        let before_id = query
            .before
            .as_ref()
            .map(|cursor| parse_id(&cursor.id.0, "fact"))
            .transpose()?;
        let before_time = query
            .before
            .as_ref()
            .map(|cursor| cursor.created_at.millis());
        let mut found = self
            .fact_leg(
                &format!(
                    "SELECT {} FROM fact WHERE \
                ($history OR (stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE)) \
                AND ($search = '' OR string::contains(string::lowercase(content), $search)) \
                AND ($subject IS NONE OR entities CONTAINS $subject) \
                AND ($source IS NONE OR id IN (SELECT VALUE in FROM derived_from WHERE out = $source) \
                    OR id IN array::flatten((SELECT VALUE fact_ids FROM memory_job WHERE episode = $source))) \
                AND ($before_time = NONE OR created_at < $before_time \
                    OR (created_at = $before_time AND id < $before_id)) \
                ORDER BY created_at DESC, id DESC LIMIT $limit;",
                    facts::FIELDS
                ),
                PageBindings {
                    history: query.include_history,
                    search: query.search.trim().to_lowercase(),
                    subject: query.subject,
                    source: query.source.as_ref().map(|id| parse_id(&id.0, "episode")).transpose()?,
                    before_time,
                    before_id,
                    limit: query.limit + 1,
                },
            )
            .await?;
        let more = found.len() > query.limit;
        found.truncate(query.limit);
        let next = if more {
            found.last().map(|fact| MemoryCursor {
                created_at: fact.created_at,
                id: fact.id.clone(),
            })
        } else {
            None
        };
        let chats = self.fact_chats(&found).await?;
        Ok(MemoryPage {
            facts: found,
            next,
            overview: Some(self.memory_overview().await?),
            chats,
        })
    }

    /// Attaches the chat name stored on each fact's original episode, when one exists.
    async fn fact_chats(
        &self,
        facts: &[Fact],
    ) -> Result<Vec<sovra_core::MemoryFactChat>, MemoryError> {
        let mut ids = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for fact in facts {
            if let Some(source) = &fact.source
                && seen.insert(source.0.clone())
            {
                ids.push(parse_id(&source.0, "episode")?);
            }
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<EpisodeChatRow> = self
            .query(
                "SELECT id, session_id FROM episode WHERE id IN $ids;",
                ("ids", ids),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let sessions = rows
            .into_iter()
            .map(|row| (row.id.to_sql(), row.session_id))
            .collect::<std::collections::HashMap<_, _>>();
        Ok(facts
            .iter()
            .filter_map(|fact| {
                let source = fact.source.as_ref()?;
                let chat = sessions.get(&source.0)?.trim();
                if chat.is_empty() {
                    return None;
                }
                Some(sovra_core::MemoryFactChat {
                    fact: fact.id.clone(),
                    chat: chat.to_owned(),
                })
            })
            .collect())
    }

    pub(super) async fn exact_fact(&self, id: &FactId) -> Result<Fact, MemoryError> {
        let row: Option<FactRow> = self
            .query(
                &format!("SELECT {} FROM ONLY $fact;", facts::FIELDS),
                ("fact", parse_id(&id.0, "fact")?),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        row.map(FactRow::into_fact)
            .ok_or_else(|| MemoryError::NotFound(id.0.clone()))
    }

    pub(super) async fn user_correction(
        &self,
        correction: MemoryCorrection,
    ) -> Result<FactId, MemoryError> {
        correction.validate()?;
        self.user_note(correction.text, Some(correction.target))
            .await
    }

    pub(super) async fn save_note(&self, note: MemoryNote) -> Result<FactId, MemoryError> {
        note.validate()?;
        self.user_note(note.text, None).await
    }

    async fn user_note(
        &self,
        text: String,
        replaces: Option<FactId>,
    ) -> Result<FactId, MemoryError> {
        let at = self.next_fact_ms();
        let text = text.trim().to_owned();
        let note = NewEpisodeRow {
            session_id: "memory-review".into(),
            kind: "note".into(),
            content: text.clone(),
            occurred_at: at,
        };
        let write = facts::prepare(
            NewFact {
                content: text,
                // Old entities and embeddings describe the replaced text. A
                // manual edit must not inherit those unverified interpretations.
                entities: Vec::new(),
                embedding: None,
                source: None,
                valid_at: Some(Timestamp(at)),
                replaces,
            },
            at,
            None,
        )?;
        // The shared fact writer checks the target inside the transaction, so a
        // competing edit/stop also rolls back this source note. Paused automatic
        // remembering does not prohibit an explicit correction by the user.
        let mut result = self
            .query(
                &format!(
                    "BEGIN TRANSACTION; \
                LET $source = (CREATE ONLY episode CONTENT $note RETURN VALUE id); \
                UPDATE $source SET origin = 'user_statement', permission_scope = 'private', source_version = 1; \
                {} RELATE $created->derived_from->$source; \
                RETURN $created; COMMIT TRANSACTION;",
                    facts::WRITE
                ),
                ("note", note, "write", write),
            )
            .await?;
        let index = result.num_statements().saturating_sub(2);
        let id: Option<RecordId> = result.take(index).map_err(|error| backend(&error))?;
        id.map(|id| FactId(id.to_sql()))
            .ok_or_else(|| MemoryError::Backend("Saving returned no memory.".into()))
    }
}

#[derive(Debug, SurrealValue)]
struct PageBindings {
    subject: Option<String>,
    source: Option<RecordId>,
    history: bool,
    search: String,
    before_time: Option<i64>,
    before_id: Option<RecordId>,
    limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::MemoryStore as _;

    #[tokio::test]
    async fn connected_review_uses_real_subject_source_and_correction_links() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let source = store
            .record_episode(sovra_core::NewEpisode {
                session_id: "connections".into(),
                kind: sovra_core::EpisodeKind::UserMessage,
                content: "Mira works on Cedar. Cedar meets on Tuesday.".into(),
                occurred_at: Timestamp(1),
            })
            .await
            .unwrap();
        let mut ids = Vec::new();
        for (text, entities, original) in [
            (
                "Mira works on Cedar",
                vec!["Mira", "Cedar"],
                Some(source.clone()),
            ),
            ("Cedar meets on Tuesday", vec!["Cedar"], Some(source)),
            ("Mira prefers email", vec!["Mira"], None),
            ("An unrelated personal preference", vec!["user"], None),
        ] {
            ids.push(
                store
                    .save_fact(NewFact {
                        content: text.into(),
                        entities: entities.into_iter().map(str::to_owned).collect(),
                        source: original,
                        embedding: None,
                        replaces: None,
                        valid_at: None,
                    })
                    .await
                    .unwrap(),
            );
        }
        let connected = store.memory_connections(&ids[0]).await.unwrap();
        assert_eq!(connected.connections.len(), 2);
        let filtered = store
            .list_memories(MemoryListQuery {
                subject: Some("Mira".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(filtered.facts.len(), 2);
        assert!(
            filtered
                .facts
                .iter()
                .all(|fact| fact.entities.contains(&"Mira".into()))
        );
        assert!(
            connected
                .connections
                .iter()
                .any(|link| link.fact.id == ids[1]
                    && link.reason == sovra_core::MemoryConnectionReason::SameSource)
        );
        assert!(
            connected
                .connections
                .iter()
                .any(|link| link.fact.id == ids[2] && link.reason.label() == "Also mentions Mira")
        );
        store.stop_memory(&ids[2], Timestamp(5)).await.unwrap();
        let updated = store
            .correct_memory(MemoryCorrection {
                target: ids[0].clone(),
                text: "Mira now works on Birch".into(),
            })
            .await
            .unwrap();
        let connected = store.memory_connections(&ids[0]).await.unwrap();
        assert_eq!(connected.connections[0].fact.id, updated);
        assert_eq!(
            connected.connections[0].reason,
            sovra_core::MemoryConnectionReason::NewerVersion
        );
        assert!(
            !connected
                .connections
                .iter()
                .any(|link| link.fact.id == ids[2])
        );
        let back = store.memory_connections(&updated).await.unwrap();
        assert_eq!(back.connections[0].fact.id, ids[0]);
        assert_eq!(
            back.connections[0].reason,
            sovra_core::MemoryConnectionReason::PreviousVersion
        );
    }

    #[tokio::test]
    async fn keyset_pages_disambiguate_equal_creation_times() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let mut expected = Vec::new();
        for text in ["First", "Second", "Third"] {
            expected.push(
                store
                    .save_fact(NewFact {
                        content: text.into(),
                        entities: vec![],
                        embedding: None,
                        source: None,
                        valid_at: None,
                        replaces: None,
                    })
                    .await
                    .unwrap(),
            );
        }
        store
            .query("UPDATE fact SET created_at = $at;", ("at", 42_i64))
            .await
            .unwrap();
        expected.sort_by(|a, b| b.0.cmp(&a.0));
        let mut query = MemoryListQuery {
            limit: 1,
            ..Default::default()
        };
        let mut found = Vec::new();
        loop {
            let page = store.list_memories(query.clone()).await.unwrap();
            found.extend(page.facts.into_iter().map(|fact| fact.id));
            query.before = page.next;
            if query.before.is_none() {
                break;
            }
            assert!(found.len() <= 3, "cursor must make forward progress");
        }
        assert_eq!(found, expected);
    }
}
