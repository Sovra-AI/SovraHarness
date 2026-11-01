//! Keyset browsing and atomic corrections with an immutable user-authored source.

use sovra_core::{
    Fact, FactId, MemoryCorrection, MemoryCursor, MemoryError, MemoryListQuery, MemoryNote,
    MemoryPage, NewFact, Timestamp,
};
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

use super::{FactRow, NewEpisodeRow, SurrealStore, backend, facts, parse_id};

impl SurrealStore {
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
                AND ($before_time = NONE OR created_at < $before_time \
                    OR (created_at = $before_time AND id < $before_id)) \
                ORDER BY created_at DESC, id DESC LIMIT $limit;",
                    facts::FIELDS
                ),
                PageBindings {
                    history: query.include_history,
                    search: query.search.trim().to_lowercase(),
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
        Ok(MemoryPage {
            facts: found,
            next,
            overview: Some(self.memory_overview().await?),
        })
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
