//! Scoped original-evidence reads, independent of successful model extraction.

use super::{EpisodeRow, SurrealStore, backend, parse_id};
use sovra_core::{Episode, EpisodeId, MemoryError, MemorySearch, MemorySourceEvidence, Timestamp};
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

/// Expand immutable source versions inside the caller's fenced transaction.
/// Missing legacy metadata never grants new authorship or permission.
pub(super) const SOURCE_VERSION_FAMILY: &str = "
    LET $source_families = array::union($sources,
        (SELECT VALUE source_family FROM $sources WHERE source_family IS NOT NONE));
    LET $version_sources = array::union($sources,
        (SELECT VALUE id FROM episode WHERE source_family IN $source_families));
";

const REVIEW_FIELDS: &str =
    "id, session_id, kind, occurred_at, origin, permission_scope, source_version, (revoked_at ?? raw_withdrawn_at) AS revoked_at, source_metadata, admitted_at,
    NONE AS processing,
    array::distinct(<-derived_from<-fact.id) AS linked";

impl SurrealStore {
    pub(super) async fn source_page(
        &self,
        query: sovra_core::MemorySourceQuery,
    ) -> Result<sovra_core::MemorySourcePage, MemoryError> {
        query.validate()?;
        let fact = query
            .fact
            .as_ref()
            .map(|id| parse_id(&id.0, "fact"))
            .transpose()?;
        let before_id = query
            .before
            .as_ref()
            .map(|cursor| parse_id(&cursor.id.0, "episode"))
            .transpose()?;
        let before_time = query
            .before
            .as_ref()
            .map(|cursor| cursor.occurred_at.millis());
        let mut rows: Vec<SourceReviewRow> = self.query(&format!(
            "SELECT {REVIEW_FIELDS}, string::slice(content, 0, 600) AS content, string::len(content) > 600 AS shortened FROM episode
             WHERE ($stopped OR (revoked_at IS NONE AND raw_withdrawn_at IS NONE))
             AND ($fact IS NONE OR id IN (SELECT VALUE out FROM derived_from WHERE in = $fact)
                 OR id IN (SELECT VALUE episode FROM memory_job WHERE fact_ids CONTAINS $fact))
             AND ($search = '' OR string::contains(string::lowercase(content), $search))
             AND ($before_time IS NONE OR occurred_at < $before_time OR (occurred_at = $before_time AND id < $before_id))
             ORDER BY occurred_at DESC, id DESC LIMIT $limit;"),
            SourcePageBindings { fact, stopped: query.include_stopped, search: query.search.trim().to_lowercase(), before_time, before_id, limit: query.limit + 1 },
        ).await?.take(0).map_err(|error| backend(&error))?;
        let more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let jobs = self
            .source_jobs(rows.iter().map(|row| row.id.clone()).collect())
            .await?;
        for row in &mut rows {
            row.merge_job(jobs.get(&row.id.to_sql()));
        }
        let next = if more {
            rows.last().map(|row| sovra_core::MemorySourceCursor {
                id: EpisodeId(row.id.to_sql()),
                occurred_at: Timestamp(row.occurred_at),
            })
        } else {
            None
        };
        let sources = rows
            .into_iter()
            .map(|row| {
                Ok(sovra_core::MemorySourceSummary {
                    metadata: row.metadata()?,
                    id: EpisodeId(row.id.to_sql()),
                    occurred_at: Timestamp(row.occurred_at),
                    kind: super::kind_from_str(&row.kind)?,
                    excerpt: row.content,
                    shortened: row.shortened,
                })
            })
            .collect::<Result<_, MemoryError>>()?;
        Ok(sovra_core::MemorySourcePage { sources, next })
    }

    pub(super) async fn source_record(
        &self,
        id: &EpisodeId,
    ) -> Result<sovra_core::MemorySourceRecord, MemoryError> {
        let row: Option<SourceReviewRow> = self
            .query(
                &format!("SELECT {REVIEW_FIELDS}, content, false AS shortened FROM ONLY $source;"),
                ("source", parse_id(&id.0, "episode")?),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let mut row = row.ok_or_else(|| MemoryError::NotFound(id.0.clone()))?;
        let jobs = self.source_jobs(vec![row.id.clone()]).await?;
        row.merge_job(jobs.get(&row.id.to_sql()));
        Ok(sovra_core::MemorySourceRecord {
            metadata: row.metadata()?,
            episode: EpisodeRow {
                id: row.id,
                session_id: row.session_id,
                kind: row.kind,
                content: row.content,
                occurred_at: row.occurred_at,
            }
            .into_episode()?,
        })
    }

    async fn source_jobs(
        &self,
        ids: Vec<RecordId>,
    ) -> Result<std::collections::HashMap<String, SourceJobRow>, MemoryError> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let jobs: Vec<SourceJobRow> = self
            .query(
                "SELECT episode, state, fact_ids FROM memory_job WHERE episode IN $ids;",
                ("ids", ids),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        Ok(jobs
            .into_iter()
            .map(|row| (row.episode.to_sql(), row))
            .collect())
    }

    pub(super) async fn preceding_evidence(
        &self,
        id: &EpisodeId,
    ) -> Result<Vec<Episode>, MemoryError> {
        let mut rows: Vec<EpisodeRow> = self
            .query(
                "LET $current = (SELECT * FROM ONLY $source);
             SELECT id, session_id, kind, content, occurred_at FROM episode
             WHERE (SELECT VALUE enabled FROM ONLY meta:remembering)
             AND $current.revoked_at IS NONE AND $current.raw_withdrawn_at IS NONE
             AND session_id = $current.session_id AND occurred_at < $current.occurred_at
             AND permission_scope = 'private' AND origin = 'user_statement' AND revoked_at IS NONE AND raw_withdrawn_at IS NONE
             AND kind IN ['user_message', 'note']
             ORDER BY occurred_at DESC, id ASC LIMIT 2;",
                ("source", parse_id(&id.0, "episode")?),
            )
            .await?
            .take(1)
            .map_err(|error| backend(&error))?;
        rows.reverse();
        rows.into_iter().map(EpisodeRow::into_episode).collect()
    }

    pub(super) async fn revoke_evidence(
        &self,
        id: &EpisodeId,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        let statement = ["BEGIN TRANSACTION;
            IF !record::exists($source) { THROW 'Original source not found'; };
            UPDATE meta:memory_facts SET revision += 1;
            UPDATE meta:remembering SET revision += 1;
            LET $sources = [$source];", SOURCE_VERSION_FAMILY,
            "UPDATE $version_sources SET revoked_at = revoked_at ?? $at;
            LET $facts = array::flatten((SELECT VALUE fact_ids FROM memory_job WHERE episode IN $version_sources));
            UPDATE fact SET stopped_at = stopped_at ?? $at, invalid_at = invalid_at ?? $at, expired_at = expired_at ?? $at
                WHERE id IN $facts OR id IN (SELECT VALUE in FROM derived_from WHERE out IN $version_sources);
            UPDATE memory_job SET state = 'cancelled', token = NONE
                WHERE episode IN $version_sources AND state IN ['pending', 'processing', 'failed'];
            COMMIT TRANSACTION;"].concat();
        self.query(
            &statement,
            ("source", parse_id(&id.0, "episode")?, "at", at.millis()),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn evidence_sources(
        &self,
        query: MemorySearch,
        now: Timestamp,
    ) -> Result<Vec<MemorySourceEvidence>, MemoryError> {
        query.validate()?;
        let terms = query.terms();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut bindings = surrealdb::types::Object::default();
        let mut statements = String::new();
        for (index, term) in terms.iter().enumerate() {
            bindings.insert(format!("term{index}"), term.clone());
            statements.push_str(&search_statement(index));
        }
        bindings.insert("before", query.known_at.unwrap_or(now).millis());
        bindings.insert(
            "valid",
            query.valid_at.or(query.known_at).unwrap_or(now).millis(),
        );
        bindings.insert(
            "entity",
            query.entity.as_ref().map(|name| name.to_lowercase()),
        );
        let mut result = self.query(&statements, bindings).await?;
        let mut legs = Vec::new();
        for index in 0..terms.len() {
            let rows: Vec<SourceEvidenceRow> =
                result.take(index).map_err(|error| backend(&error))?;
            legs.push(
                rows.into_iter()
                    .map(|row| row.into_evidence(&query))
                    .collect::<Result<Vec<_>, MemoryError>>()?,
            );
        }
        Ok(sovra_core::fuse_memory_sources(legs, 12))
    }

    pub(super) async fn evidence_batch(
        &self,
        ids: &[EpisodeId],
    ) -> Result<Vec<Episode>, MemoryError> {
        if ids.len() > sovra_core::MAX_RECALL_RESULTS {
            return Err(MemoryError::InvalidInput("too many evidence reads".into()));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids
            .iter()
            .map(|id| parse_id(&id.0, "episode"))
            .collect::<Result<Vec<RecordId>, _>>()?;
        let rows: Vec<EpisodeRow> = self
            .query(
                "SELECT id, session_id, kind, content, occurred_at FROM $ids
                 WHERE (SELECT VALUE enabled FROM ONLY meta:remembering)
                 AND permission_scope = 'private' AND origin = 'user_statement'
                 AND source_version > 0 AND revoked_at IS NONE
                 AND kind IN ['user_message', 'note'];",
                ("ids", ids),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        rows.into_iter().map(EpisodeRow::into_episode).collect()
    }
}

#[derive(Debug, SurrealValue)]
struct SourceEvidenceRow {
    id: RecordId,
    session_id: String,
    kind: String,
    content: String,
    occurred_at: i64,
    source_version: u64,
    support_changed: bool,
}
impl SourceEvidenceRow {
    fn into_evidence(self, query: &MemorySearch) -> Result<MemorySourceEvidence, MemoryError> {
        let episode = EpisodeRow {
            id: self.id,
            session_id: self.session_id,
            kind: self.kind,
            content: self.content,
            occurred_at: self.occurred_at,
        }
        .into_episode()?;
        let mut evidence = MemorySourceEvidence::private_admitted(&episode, query)?;
        evidence.version = self.source_version;
        evidence.support_changed = self.support_changed;
        Ok(evidence)
    }
}

#[derive(Debug, SurrealValue)]
struct SourcePageBindings {
    fact: Option<RecordId>,
    stopped: bool,
    search: String,
    before_time: Option<i64>,
    before_id: Option<RecordId>,
    limit: usize,
}

#[derive(Debug, SurrealValue)]
struct SourceJobRow {
    episode: RecordId,
    state: String,
    fact_ids: Vec<RecordId>,
}

#[derive(Debug, SurrealValue)]
struct SourceReviewRow {
    id: RecordId,
    session_id: String,
    kind: String,
    content: String,
    occurred_at: i64,
    shortened: bool,
    source_metadata: Option<String>,
    admitted_at: Option<i64>,
    origin: Option<String>,
    permission_scope: Option<String>,
    source_version: Option<u64>,
    revoked_at: Option<i64>,
    processing: Option<String>,
    linked: Vec<RecordId>,
}

impl SourceReviewRow {
    fn merge_job(&mut self, job: Option<&SourceJobRow>) {
        if let Some(job) = job {
            self.processing = Some(job.state.clone());
            for id in &job.fact_ids {
                if !self.linked.contains(id) {
                    self.linked.push(id.clone());
                }
            }
        }
    }
    fn metadata(&self) -> Result<sovra_core::MemorySourceMetadata, MemoryError> {
        use sovra_core::{MemorySourceOrigin, MemorySourceScope};
        let origin = match self.origin.as_deref() {
            Some("user_statement") => MemorySourceOrigin::UserStatement,
            Some("assistant_output") => MemorySourceOrigin::AssistantOutput,
            Some("document_claim") => MemorySourceOrigin::DocumentClaim,
            Some("legacy_unknown") | None => MemorySourceOrigin::LegacyUnknown,
            Some(_) => return Err(MemoryError::Backend("Unknown source origin".into())),
        };
        let scope = match self.permission_scope.as_deref() {
            Some("private") => MemorySourceScope::Private,
            Some("unknown") | None => MemorySourceScope::Unknown,
            Some(_) => return Err(MemoryError::Backend("Unknown source scope".into())),
        };
        Ok(sovra_core::MemorySourceMetadata {
            attribution: self
                .source_metadata
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| {
                    MemoryError::Backend(format!("invalid source attribution: {error}"))
                })?,
            admitted_at: self.admitted_at.map(Timestamp),
            origin,
            scope,
            version: self.source_version,
            revoked_at: self.revoked_at.map(Timestamp),
            processing: self
                .processing
                .as_deref()
                .map(super::ingestion::job_state)
                .transpose()?,
            detail_count: Some(self.linked.len()),
        })
    }
}

fn search_statement(parameter: usize) -> String {
    // Direct bind parameters let the pinned engine resolve score references.
    // Rank each term separately: common-word questions must not crowd out the
    // distinguishing term before core applies the shared evidence budget.
    format!(
        "SELECT id, session_id, kind, content, occurred_at, source_version, search::score(0) AS relevance,
            IF origin = 'assistant_output' {{
                array::len((SELECT VALUE id FROM $parent.dependency_facts
                    WHERE (expired_at IS NOT NONE AND expired_at <= $before)
                    OR (expired_at IS NONE AND invalid_at IS NOT NONE AND invalid_at <= $valid))) > 0
                OR array::len((SELECT VALUE id FROM $parent.dependency_sources
                    WHERE array::len(<-derived_from<-fact[WHERE
                        (expired_at IS NOT NONE AND expired_at <= $before)
                        OR (expired_at IS NONE AND invalid_at IS NOT NONE AND invalid_at <= $valid)]) > 0)) > 0
            }} ELSE {{ false }} AS support_changed FROM episode
            WHERE (SELECT VALUE enabled FROM ONLY meta:remembering)
            AND permission_scope = 'private' AND revoked_at IS NONE AND raw_withdrawn_at IS NONE
            AND ((origin = 'user_statement' AND kind IN ['user_message', 'note'])
                OR (origin = 'assistant_output' AND kind = 'agent_message' AND dependency_complete = true
                    AND array::len(dependency_sources) = array::len((SELECT VALUE id FROM $parent.dependency_sources
                        WHERE permission_scope = 'private' AND revoked_at IS NONE AND raw_withdrawn_at IS NONE
                        AND array::len(<-derived_from<-fact[WHERE stopped_at IS NOT NONE]) = 0))
                    AND array::len(dependency_facts) = array::len((SELECT VALUE id FROM $parent.dependency_facts WHERE stopped_at IS NONE))))
            AND source_version > 0 AND content @0@ $term{parameter}
            AND occurred_at <= $before
            AND ($entity IS NONE OR string::contains(string::lowercase(content), $entity))
            AND array::len(<-derived_from<-fact[WHERE stopped_at IS NOT NONE]) = 0
            ORDER BY relevance DESC, occurred_at DESC, id ASC LIMIT 12;"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::{EpisodeKind, MemoryInput, MemoryStore, NewEpisode};

    #[tokio::test]
    async fn fact_evidence_batch_excludes_unknown_permissions_withdrawal_and_pause() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let episode = NewEpisode {
            session_id: "scope-fixture".into(),
            kind: EpisodeKind::UserMessage,
            content: "Fictional scope evidence".into(),
            occurred_at: Timestamp(1),
        };
        let legacy = store.record_episode(episode.clone()).await.unwrap();
        let admitted = store
            .enqueue_memory(MemoryInput {
                key: "admitted".into(),
                episode,
            })
            .await
            .unwrap();
        let ids = vec![legacy.clone(), admitted.clone()];
        let loaded = store.memory_episodes(&ids).await.unwrap();
        assert_eq!(
            loaded.iter().map(|episode| &episode.id).collect::<Vec<_>>(),
            vec![&admitted]
        );
        store.set_memory_enabled(false).await.unwrap();
        assert_eq!(store.memory_episodes(&ids).await.unwrap(), Vec::new());
        store.set_memory_enabled(true).await.unwrap();
        store
            .stop_memory_source(&admitted, Timestamp(2))
            .await
            .unwrap();
        assert_eq!(store.memory_episodes(&ids).await.unwrap(), Vec::new());
        assert_eq!(
            store.episode(&legacy).await.unwrap().content,
            "Fictional scope evidence"
        );
    }

    #[tokio::test]
    async fn partial_withdrawal_keeps_fact_grounding_but_full_revocation_survives_migration() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let source = store
            .enqueue_memory(MemoryInput {
                key: "partial".into(),
                episode: NewEpisode {
                    session_id: "withdrawal-fixture".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "Juniper tea. Cedar boat.".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap();
        let mut ids = Vec::new();
        for text in ["Juniper tea.", "Cedar boat."] {
            ids.push(
                store
                    .save_fact(sovra_core::NewFact {
                        content: text.into(),
                        entities: vec![],
                        embedding: None,
                        source: Some(source.clone()),
                        replaces: None,
                        valid_at: None,
                    })
                    .await
                    .unwrap(),
            );
        }
        store.remove_memory(&ids[0], Timestamp(2)).await.unwrap();
        assert_eq!(
            store
                .memory_episodes(std::slice::from_ref(&source))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .search_memory_sources(
                    MemorySearch {
                        text: "cedar".into(),
                        ..Default::default()
                    },
                    Timestamp(i64::MAX)
                )
                .await
                .unwrap(),
            Vec::new()
        );
        assert!(
            store
                .memory_fact(&ids[1])
                .await
                .unwrap()
                .stopped_at
                .is_none()
        );
        store
            .stop_memory_source(&source, Timestamp(3))
            .await
            .unwrap();
        store
            .query(
                "UPDATE meta:schema SET version = 11;",
                surrealdb::types::Object::default(),
            )
            .await
            .unwrap();
        store.apply_schema().await.unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(
            store
                .memory_episodes(std::slice::from_ref(&source))
                .await
                .unwrap(),
            Vec::new()
        );
        assert_eq!(
            store.episode(&source).await.unwrap().content,
            "Juniper tea. Cedar boat."
        );
        assert!(
            store
                .memory_fact(&ids[1])
                .await
                .unwrap()
                .stopped_at
                .is_some()
        );
    }

    #[tokio::test]
    async fn source_family_admission_and_withdrawal_use_the_version_index() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let source = store
            .enqueue_memory(MemoryInput {
                key: "family-plan".into(),
                episode: NewEpisode {
                    session_id: "family-plan".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "Fixture wording.".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap();
        let id = parse_id(&source.0, "episode").unwrap();
        for sql in [
            "SELECT id FROM episode WHERE source_family = $family AND (revoked_at IS NOT NONE OR raw_withdrawn_at IS NOT NONE) LIMIT 1 EXPLAIN FULL;",
            "SELECT VALUE id FROM episode WHERE source_family IN $families EXPLAIN FULL;",
        ] {
            let plan: Vec<surrealdb::types::Value> = store
                .query(sql, ("family", id.clone(), "families", vec![id.clone()]))
                .await
                .unwrap()
                .take(0)
                .unwrap();
            let plan = format!("{plan:?}");
            assert!(plan.contains("episode_source_family"), "{plan}");
            assert!(!plan.contains("Iterate Table"), "{plan}");
        }
    }

    #[tokio::test]
    async fn preceding_context_excludes_revoked_unknown_and_later_sources() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let mut ids = Vec::new();
        for (index, text) in ["permitted", "withdrawn", "current", "later"]
            .iter()
            .enumerate()
        {
            ids.push(
                store
                    .enqueue_memory(MemoryInput {
                        key: text.to_string(),
                        episode: NewEpisode {
                            session_id: "context".into(),
                            kind: EpisodeKind::UserMessage,
                            content: text.to_string(),
                            occurred_at: Timestamp(i64::try_from(index).unwrap() + 1),
                        },
                    })
                    .await
                    .unwrap(),
            );
        }
        store
            .record_episode(NewEpisode {
                session_id: "context".into(),
                kind: EpisodeKind::UserMessage,
                content: "not admitted".into(),
                occurred_at: Timestamp(1),
            })
            .await
            .unwrap();
        store
            .stop_memory_source(&ids[1], Timestamp(5))
            .await
            .unwrap();
        let context = store.memory_preceding_sources(&ids[2]).await.unwrap();
        assert_eq!(
            context.iter().map(|source| &source.id).collect::<Vec<_>>(),
            vec![&ids[0]]
        );
        store.set_memory_enabled(false).await.unwrap();
        assert_eq!(
            store.memory_preceding_sources(&ids[2]).await.unwrap(),
            Vec::new()
        );
    }

    #[tokio::test]
    async fn source_plan_uses_the_pinned_fulltext_index_and_migration_keeps_unknown_scope() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let source = store
            .enqueue_memory(MemoryInput {
                key: "fixture".into(),
                episode: NewEpisode {
                    session_id: "migration".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "The rehearsal code is cedarlock.".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap();
        for parameter in [0, 1] {
            let sql =
                search_statement(parameter).trim_end_matches(';').to_owned() + " EXPLAIN FULL;";
            let plan: Vec<surrealdb::types::Value> = store
                .query(
                    &sql,
                    (
                        "term0",
                        "cedarlock",
                        "term1",
                        "rehearsal",
                        "before",
                        10_i64,
                        "entity",
                        Option::<String>::None,
                    ),
                )
                .await
                .unwrap()
                .take(0)
                .unwrap();
            assert!(format!("{plan:?}").contains("episode_text"), "{plan:?}");
        }
        store.query("UPDATE episode UNSET origin, permission_scope, source_version; UPDATE memory_job UNSET cursor, session_id; UPDATE meta:schema SET version = 6;", surrealdb::types::Object::default()).await.unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.episode(&source).await.unwrap().content,
            "The rehearsal code is cedarlock."
        );
        assert_eq!(
            store
                .claim_memory(Timestamp(10))
                .await
                .unwrap()
                .unwrap()
                .cursor,
            0
        );
        assert_eq!(
            store
                .search_memory_sources(
                    MemorySearch {
                        text: "cedarlock".into(),
                        ..Default::default()
                    },
                    Timestamp(10)
                )
                .await
                .unwrap(),
            Vec::new()
        );
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use sovra_core::{EpisodeKind, MemoryInput, MemorySourceQuery, MemoryStore, NewEpisode};

    #[tokio::test]
    async fn source_library_keeps_unextracted_evidence_and_pages_without_repeats() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let mut ids = Vec::new();
        for index in 0..5 {
            ids.push(
                store
                    .enqueue_memory(MemoryInput {
                        key: format!("source-{index}"),
                        episode: NewEpisode {
                            session_id: "review".into(),
                            kind: EpisodeKind::UserMessage,
                            content: format!("{} hidden needle {index}", "茶".repeat(700)),
                            occurred_at: Timestamp(index / 2),
                        },
                    })
                    .await
                    .unwrap(),
            );
        }
        let page = store
            .list_memory_sources(MemorySourceQuery {
                search: "NEEDLE".into(),
                limit: 2,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.sources.len(), 2);
        assert_eq!(page.sources[0].excerpt, "茶".repeat(600));
        assert!(page.sources[0].shortened);
        assert_eq!(page.sources[0].metadata.detail_count, Some(0));
        assert_eq!(
            page.sources[0].metadata.processing,
            Some(sovra_core::MemoryJobState::Pending)
        );
        let mut found = page
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect::<Vec<_>>();
        let mut before = page.next;
        store
            .enqueue_memory(MemoryInput {
                key: "newer".into(),
                episode: NewEpisode {
                    session_id: "review".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "newer needle".into(),
                    occurred_at: Timestamp(100),
                },
            })
            .await
            .unwrap();
        while let Some(cursor) = before {
            let page = store
                .list_memory_sources(MemorySourceQuery {
                    search: "needle".into(),
                    before: Some(cursor),
                    limit: 2,
                    ..Default::default()
                })
                .await
                .unwrap();
            found.extend(page.sources.into_iter().map(|source| source.id));
            before = page.next;
        }
        let mut expected = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (index / 2, id.clone()))
            .collect::<Vec<_>>();
        expected.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.0.cmp(&a.1.0)));
        assert_eq!(
            found,
            expected.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
        );
        store
            .stop_memory_source(&ids[4], Timestamp(110))
            .await
            .unwrap();
        let active = store
            .list_memory_sources(MemorySourceQuery::default())
            .await
            .unwrap();
        assert!(!active.sources.iter().any(|source| source.id == ids[4]));
        let retained = store
            .list_memory_sources(MemorySourceQuery {
                include_stopped: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(retained.sources.iter().any(
            |source| source.id == ids[4] && source.metadata.revoked_at == Some(Timestamp(110))
        ));
        store.set_memory_enabled(false).await.unwrap();
        let record = store.review_memory_source(&ids[4]).await.unwrap();
        assert!(record.episode.content.ends_with("hidden needle 4"));
        assert_eq!(record.metadata.revoked_at, Some(Timestamp(110)));
        assert_eq!(
            store
                .search_memory_sources(
                    MemorySearch {
                        text: "needle".into(),
                        ..Default::default()
                    },
                    Timestamp(200)
                )
                .await
                .unwrap(),
            Vec::new()
        );
    }

    #[tokio::test]
    async fn source_status_batch_uses_the_episode_index_on_the_pinned_engine() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let id = store
            .enqueue_memory(MemoryInput {
                key: "plan".into(),
                episode: NewEpisode {
                    session_id: "plan".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "Source plan fixture".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap();
        let plan: Vec<surrealdb::types::Value> = store
            .query(
                "SELECT episode, state, fact_ids FROM memory_job WHERE episode IN $ids EXPLAIN FULL;",
                ("ids", vec![parse_id(&id.0, "episode").unwrap()]),
            )
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert!(
            format!("{plan:?}").contains("memory_job_episode"),
            "{plan:?}"
        );
    }

    #[tokio::test]
    async fn source_review_preserves_unknown_origin_and_links_exact_saved_details() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let legacy = store
            .record_episode(NewEpisode {
                session_id: "old".into(),
                kind: EpisodeKind::AgentMessage,
                content: "An old helper response".into(),
                occurred_at: Timestamp(0),
            })
            .await
            .unwrap();
        let record = store.review_memory_source(&legacy).await.unwrap();
        assert_eq!(
            record.metadata.origin,
            sovra_core::MemorySourceOrigin::LegacyUnknown
        );
        assert_eq!(
            record.metadata.scope,
            sovra_core::MemorySourceScope::Unknown
        );
        let fact = store
            .remember_note(sovra_core::MemoryNote {
                text: "Keep this exact note".into(),
            })
            .await
            .unwrap();
        let saved = store.memory_fact(&fact).await.unwrap();
        let source = saved.source.unwrap();
        let record = store.review_memory_source(&source).await.unwrap();
        assert_eq!(
            record.metadata.origin,
            sovra_core::MemorySourceOrigin::UserStatement
        );
        assert_eq!(
            record.metadata.scope,
            sovra_core::MemorySourceScope::Private
        );
        assert_eq!(record.metadata.detail_count, Some(1));
        let details = store
            .list_memories(sovra_core::MemoryListQuery {
                source: Some(source),
                include_history: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            details
                .facts
                .iter()
                .map(|fact| &fact.id)
                .collect::<Vec<_>>(),
            vec![&fact]
        );
    }
}
