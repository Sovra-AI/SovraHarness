//! Keyset browsing and atomic corrections with an immutable user-authored source.

use sovra_core::{
    Fact, FactId, MEMORY_REVIEW_SESSION, MemoryCorrection, MemoryCursor, MemoryError,
    MemoryListQuery, MemoryNote, MemoryPage, MemorySameDetail, NewFact, Timestamp,
};
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

#[derive(Debug, SurrealValue)]
struct EpisodeChatRow {
    id: RecordId,
    session_id: String,
}

#[derive(Debug, SurrealValue)]
struct ThreadTitleRow {
    session_id: String,
    title: String,
}

#[derive(Debug, SurrealValue)]
struct RepeatLink {
    newer: RecordId,
    older: RecordId,
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
        let mut session_by_episode = std::collections::HashMap::new();
        let mut sessions = Vec::new();
        for row in rows {
            let session = row.session_id.trim();
            // A note typed on this screen is stored on a review episode. That
            // bucket is not a conversation, so it must not become a chat node.
            if session.is_empty() || session == MEMORY_REVIEW_SESSION {
                continue;
            }
            let session = session.to_owned();
            if !sessions.contains(&session) {
                sessions.push(session.clone());
            }
            session_by_episode.insert(row.id.to_sql(), session);
        }
        if sessions.is_empty() {
            return Ok(Vec::new());
        }
        let titles: Vec<ThreadTitleRow> = self
            .query(
                "SELECT session_id, title FROM chat_thread WHERE session_id IN $sessions;",
                ("sessions", sessions),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let titles = titles
            .into_iter()
            .map(|row| (row.session_id, row.title))
            .collect::<std::collections::HashMap<_, _>>();
        Ok(facts
            .iter()
            .filter_map(|fact| {
                let source = fact.source.as_ref()?;
                let session = session_by_episode.get(&source.0)?;
                let title = titles.get(session)?.trim();
                if title.is_empty() {
                    return None;
                }
                Some(sovra_core::MemoryFactChat {
                    fact: fact.id.clone(),
                    session: session.clone(),
                    title: title.to_owned(),
                })
            })
            .collect())
    }

    /// Records an earlier-version link without hiding either detail.
    ///
    /// `supersedes` is reserved for corrections: writing one also expires the
    /// older fact, and removal walks that edge. A repeat uses `repeats`, which
    /// the page reads as `replaces` only when no correction edge exists.
    pub(super) async fn link_repeated_details(
        &self,
        links: &[MemorySameDetail],
    ) -> Result<(), MemoryError> {
        if links.is_empty() {
            return Ok(());
        }
        if links.len() > sovra_core::MAX_MEMORY_PAGE {
            return Err(MemoryError::InvalidInput(
                "Too many repeated details to link at once.".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for link in links {
            link.validate()?;
            if !seen.insert((link.newer.0.clone(), link.older.0.clone())) {
                continue;
            }
            rows.push(RepeatLink {
                newer: parse_id(&link.newer.0, "fact")?,
                older: parse_id(&link.older.0, "fact")?,
            });
        }
        self.query(
            "BEGIN TRANSACTION;
             FOR $link IN $links {
                 IF !record::exists($link.newer) OR !record::exists($link.older) {
                     THROW 'saved detail no longer exists';
                 };
                 LET $forward = (SELECT VALUE id FROM repeats WHERE in = $link.newer AND out = $link.older LIMIT 1);
                 LET $reverse = (SELECT VALUE id FROM repeats WHERE in = $link.older AND out = $link.newer LIMIT 1);
                 IF array::len($forward) = 0 AND array::len($reverse) = 0 {
                     RELATE ($link.newer)->repeats->($link.older);
                 };
             };
             COMMIT TRANSACTION;",
            ("links", rows),
        )
        .await?;
        Ok(())
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
            session_id: MEMORY_REVIEW_SESSION.into(),
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

    #[derive(Debug, SurrealValue)]
    struct TotalRow {
        count: i64,
    }

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

    #[tokio::test]
    async fn listed_memories_use_chat_titles_and_skip_the_review_bucket() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let mut first = sovra_core::ChatThread::new("chat-school-a", Timestamp(1));
        first.title = "School pickup".into();
        let mut second = sovra_core::ChatThread::new("chat-school-b", Timestamp(2));
        second.title = "School pickup".into();
        store.save_chat_thread(&first).await.unwrap();
        store.save_chat_thread(&second).await.unwrap();
        let episode_a = store
            .record_episode(sovra_core::NewEpisode {
                session_id: "chat-school-a".into(),
                kind: sovra_core::EpisodeKind::UserMessage,
                content: "Sam likes mint tea at school.".into(),
                occurred_at: Timestamp(3),
            })
            .await
            .unwrap();
        let episode_b = store
            .record_episode(sovra_core::NewEpisode {
                session_id: "chat-school-b".into(),
                kind: sovra_core::EpisodeKind::UserMessage,
                content: "Sam keeps mint tea at home.".into(),
                occurred_at: Timestamp(4),
            })
            .await
            .unwrap();
        let fact_a = store
            .save_fact(NewFact {
                content: "Sam likes mint tea at school.".into(),
                entities: vec!["Sam".into()],
                embedding: None,
                source: Some(episode_a),
                valid_at: None,
                replaces: None,
            })
            .await
            .unwrap();
        let fact_b = store
            .save_fact(NewFact {
                content: "Sam keeps mint tea at home.".into(),
                entities: vec!["Sam".into()],
                embedding: None,
                source: Some(episode_b),
                valid_at: None,
                replaces: None,
            })
            .await
            .unwrap();
        let note = store
            .remember_note(MemoryNote {
                text: "I typed this on the memory screen.".into(),
            })
            .await
            .unwrap();
        let page = store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap();
        assert!(page.facts.iter().any(|fact| fact.id == note));
        assert!(page.facts.iter().any(|fact| fact.id == fact_a));
        assert!(page.facts.iter().any(|fact| fact.id == fact_b));
        assert_eq!(page.chats.len(), 2);
        let mut titles = page
            .chats
            .iter()
            .map(|chat| chat.title.as_str())
            .collect::<Vec<_>>();
        titles.sort_unstable();
        assert_eq!(titles, ["School pickup", "School pickup"]);
        let mut sessions = page
            .chats
            .iter()
            .map(|chat| chat.session.as_str())
            .collect::<Vec<_>>();
        sessions.sort_unstable();
        assert_eq!(sessions, ["chat-school-a", "chat-school-b"]);
        assert_eq!(
            page.chats
                .iter()
                .filter(|chat| chat.session == MEMORY_REVIEW_SESSION
                    || chat.title == MEMORY_REVIEW_SESSION
                    || chat.fact == note)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn keeping_the_same_wording_links_both_details_without_hiding_either() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let older = store
            .remember_note(MemoryNote {
                text: "Sam likes mint tea.".into(),
            })
            .await
            .unwrap();
        let newer = store
            .remember_note(MemoryNote {
                text: "sam   likes mint tea!".into(),
            })
            .await
            .unwrap();
        let close = store
            .remember_note(MemoryNote {
                text: "Sam enjoys mint tea.".into(),
            })
            .await
            .unwrap();
        let links = [
            MemorySameDetail {
                newer: newer.clone(),
                older: older.clone(),
            },
            MemorySameDetail {
                newer: close.clone(),
                older: older.clone(),
            },
        ];
        store.keep_same_details(&links).await.unwrap();
        store.keep_same_details(&links).await.unwrap();
        let page = store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap();
        assert_eq!(page.facts.len(), 3);
        for fact in &page.facts {
            assert_eq!(fact.invalid_at, None);
            assert_eq!(fact.expired_at, None);
            assert_eq!(fact.stopped_at, None);
        }
        assert_eq!(
            page.facts
                .iter()
                .find(|fact| fact.id == newer)
                .and_then(|fact| fact.replaces.clone()),
            Some(older.clone())
        );
        assert_eq!(
            page.facts
                .iter()
                .find(|fact| fact.id == close)
                .and_then(|fact| fact.replaces.clone()),
            Some(older.clone())
        );
        let report = sovra_core::check_memory_duplicates(&page.facts);
        assert_eq!(
            report
                .near
                .iter()
                .filter(|pair| pair.newer == close || pair.older == close)
                .count(),
            0
        );
        let records = page
            .facts
            .iter()
            .cloned()
            .map(|fact| sovra_core::MemoryGraphRecord {
                fact,
                session: None,
                title: None,
            })
            .collect::<Vec<_>>();
        let graph = sovra_core::memory_graph(&records, &[]);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, sovra_core::MemoryGraphEdgeKind::EarlierVersion))
        );
        let totals: Vec<TotalRow> = store
            .query(
                "SELECT count() AS count FROM repeats GROUP ALL;",
                surrealdb::types::Object::default(),
            )
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(totals.first().map_or(0, |row| row.count), 2);
        store.remove_memory(&newer, Timestamp(9_000)).await.unwrap();
        let after = store
            .list_memories(MemoryListQuery::default())
            .await
            .unwrap();
        assert!(after.facts.iter().any(|fact| fact.id == older));
        assert!(after.facts.iter().any(|fact| fact.id == close));
        assert!(after.facts.iter().all(|fact| fact.id != newer));
    }
}
