//! `SurrealDB` adapter for the [`sovra_core::MemoryStore`] port — the only crate that
//! names `SurrealDB`.
//!
//! One connection API covers every deployment via `surrealdb::engine::any`:
//! in-memory (tests), embedded on disk (the desktop default) and a remote server
//! (`ws://`) later — swapping is a [`StoreLocation`] change in the composition root,
//! and swapping the whole backend is another `sovra-store-*` crate implementing the
//! same port.
//!
//! Fact writes and their provenance/replacement edges commit together. A failed
//! correction must never leave a new belief beside an unchanged or missing source.

mod facts;
mod ingestion;
mod lifecycle;
mod profile;
mod schema;
mod semantic;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sovra_core::artifacts::Artifact;
use sovra_core::teams::TeamMember;
use sovra_core::{
    ChatThread, ConversationKind, Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId,
    MemoryError, MemoryStore, NewEpisode, NewFact, RecallQuery, RecalledFact, Timestamp, fuse_rrf,
};
use surrealdb::IndexedResults;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::method::IntoVariables;
use surrealdb::types::ToSql as _;
use surrealdb::types::{RecordId, SurrealValue};

/// Where the store lives. Local today; `Remote` keeps the door open without
/// changing any caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreLocation {
    /// Ephemeral, for tests and previews.
    InMemory,
    /// Embedded `SurrealKV` in this directory — the desktop default.
    OnDisk(PathBuf),
    /// A `SurrealDB` server endpoint (`ws://host:port`). Authentication is not wired
    /// yet; embedded engines need none and remote support lands with it.
    Remote { url: String },
}

/// Adapter configuration, built once in the composition root.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub location: StoreLocation,
    pub namespace: String,
    pub database: String,
    /// Embedder identity the HNSW index is built for. `None` means "not yet chosen":
    /// the first embedding saved pins its model persistently. Reopening cannot mix models.
    pub embedding_model: Option<String>,
    /// HNSW `DIMENSION`; vectors of any other length are rejected, not padded.
    pub embedding_dimension: usize,
}

impl StoreConfig {
    /// Nomic/BGE-class local embedders use 768; change in lockstep with the embedder.
    pub const DEFAULT_EMBEDDING_DIMENSION: usize = 768;

    pub fn new(location: StoreLocation) -> Self {
        Self {
            location,
            namespace: "sovra".to_owned(),
            database: "memory".to_owned(),
            embedding_model: None,
            embedding_dimension: Self::DEFAULT_EMBEDDING_DIMENSION,
        }
    }

    pub fn in_memory() -> Self {
        Self::new(StoreLocation::InMemory)
    }

    pub fn on_disk(dir: impl Into<PathBuf>) -> Self {
        Self::new(StoreLocation::OnDisk(dir.into()))
    }

    fn address(&self) -> String {
        match &self.location {
            StoreLocation::InMemory => "mem://".to_owned(),
            StoreLocation::OnDisk(dir) => format!("surrealkv://{}", dir.display()),
            StoreLocation::Remote { url } => url.clone(),
        }
    }
}

/// The [`MemoryStore`] implementation backed by `SurrealDB`.
#[derive(Debug, Clone)]
pub struct SurrealStore {
    db: Surreal<Any>,
    config: StoreConfig,
    /// Last fact `created_at` handed out. Recency is ordered by this millisecond stamp,
    /// so two saves in the same millisecond must not tie; shared by clones of one store.
    last_fact_ms: Arc<AtomicI64>,
}

impl SurrealStore {
    /// Connects, selects the namespace/database and applies the idempotent schema.
    pub async fn open(config: StoreConfig) -> Result<Self, MemoryError> {
        let db = surrealdb::engine::any::connect(config.address())
            .await
            .map_err(|e| unavailable(&e))?;
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .map_err(|e| unavailable(&e))?;
        let store = Self {
            db,
            config,
            last_fact_ms: Arc::new(AtomicI64::new(0)),
        };
        store.apply_schema().await?;
        Ok(store)
    }

    /// Wall-clock milliseconds, bumped past the previous stamp so newer facts always sort first.
    fn next_fact_ms(&self) -> i64 {
        let now = now_ms();
        let previous = self
            .last_fact_ms
            .update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                now.max(last.saturating_add(1))
            });
        now.max(previous.saturating_add(1))
    }

    async fn apply_schema(&self) -> Result<(), MemoryError> {
        if self.config.embedding_dimension == 0 || self.config.embedding_dimension > 16_384 {
            return Err(MemoryError::InvalidInput(
                "unsupported embedding dimension".into(),
            ));
        }
        let tables: Option<surrealdb::types::Object> = self
            .query("INFO FOR DB;", surrealdb::types::Object::default())
            .await?
            .take("tables")
            .map_err(|e| backend(&e))?;
        let has_table = |name: &str| {
            tables
                .as_ref()
                .is_some_and(|tables| tables.contains_key(name))
        };
        let version: Option<i64> = if has_table("meta") {
            self.query(
                "SELECT VALUE version FROM ONLY meta:schema;",
                surrealdb::types::Object::default(),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?
        } else {
            None
        };
        let stored = match version {
            None | Some(1) => {
                if has_table("fact") {
                    self.legacy_embedding_index().await?
                } else {
                    None
                }
            }
            Some(2 | 3 | 4 | 5 | schema::SCHEMA_VERSION) => Some(
                self.embedding_index()
                    .await?
                    .ok_or_else(|| MemoryError::Backend("missing memory index metadata".into()))?,
            ),
            Some(other) => {
                return Err(MemoryError::Backend(format!(
                    "unsupported memory schema {other}; open it with a compatible Sovra version"
                )));
            }
        };
        let model = if let Some(index) = stored {
            if index.dimension != self.config.embedding_dimension
                || index
                    .model
                    .as_ref()
                    .zip(self.config.embedding_model.as_ref())
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(MemoryError::InvalidInput("memory index configuration changed; reindex before changing embedding models or dimensions".into()));
            }
            index.model.or_else(|| self.config.embedding_model.clone())
        } else {
            self.config.embedding_model.clone()
        };
        let withdrawal_migration = if version.is_none_or(|version| version < 5) && has_table("fact")
        {
            schema::MIGRATE_WITHDRAWALS
        } else {
            ""
        };
        let statements = format!(
            "BEGIN TRANSACTION;\n{}\n{}\nUPSERT meta:embedding SET model = $model, dimension = $dimension;\nUPSERT meta:remembering SET enabled = enabled ?? true, generation = generation ?? 0, revision = revision ?? 0;\nUPSERT meta:memory_semantic SET generation = generation ?? 0, revision = revision ?? 0;\n{withdrawal_migration}\nUPSERT meta:schema SET version = $version;\nCOMMIT TRANSACTION;",
            schema::STATIC_SCHEMA,
            schema::hnsw_index(self.config.embedding_dimension),
        );
        self.query(
            &statements,
            (
                "version",
                schema::SCHEMA_VERSION,
                "model",
                model,
                "dimension",
                self.config.embedding_dimension,
            ),
        )
        .await?;
        if let Some(setting) = self.semantic_state().await?.setting {
            let mut statements = schema::semantic_table(setting.generation);
            if let Some(dimension) = setting.dimension {
                statements.push_str(&schema::semantic_index(setting.generation, dimension));
            }
            self.query(&statements, surrealdb::types::Object::default())
                .await?;
        }
        Ok(())
    }

    async fn embedding_index(&self) -> Result<Option<EmbeddingIndexRow>, MemoryError> {
        self.query(
            "SELECT model, dimension FROM ONLY meta:embedding;",
            surrealdb::types::Object::default(),
        )
        .await?
        .take(0)
        .map_err(|e| backend(&e))
    }

    /// v1 did not persist index identity. Inspect existing vectors once; refusing a
    /// mixed legacy index is safer than silently accepting meaningless distances.
    async fn legacy_embedding_index(&self) -> Result<Option<EmbeddingIndexRow>, MemoryError> {
        let mut rows: Vec<EmbeddingIndexRow> = self.query(
            "SELECT embed_model AS model, array::len(embedding) AS dimension FROM fact WHERE embedding IS NOT NONE GROUP BY model, dimension LIMIT 2;", surrealdb::types::Object::default(),
        ).await?.take(0).map_err(|e| backend(&e))?;
        if rows.len() > 1
            || rows.first().is_some_and(|index| {
                index
                    .model
                    .as_ref()
                    .is_none_or(|model| model.trim().is_empty())
            })
        {
            return Err(MemoryError::InvalidInput("legacy memory contains incompatible embeddings; reindex before using semantic search".into()));
        }
        Ok(rows.pop())
    }

    async fn query(
        &self,
        statements: &str,
        bindings: impl IntoVariables,
    ) -> Result<IndexedResults, MemoryError> {
        let response = self
            .db
            .query(statements)
            .bind(bindings)
            .await
            .map_err(|e| backend(&e))?;
        ensure_ok(response)
    }

    async fn check_embedding(&self, embedding: &Embedding) -> Result<(), MemoryError> {
        embedding.validate()?;
        let dimension_ok = embedding.dimension() == self.config.embedding_dimension;
        let indexed = self
            .embedding_index()
            .await?
            .and_then(|index| index.model)
            .or_else(|| self.config.embedding_model.clone());
        let model_ok = indexed
            .as_ref()
            .is_none_or(|model| *model == embedding.model);
        if dimension_ok && model_ok {
            Ok(())
        } else {
            Err(MemoryError::EmbeddingMismatch {
                indexed: indexed.unwrap_or_else(|| "unspecified".to_owned()),
                dimension: self.config.embedding_dimension,
                got: format!("{} ({} dims)", embedding.model, embedding.dimension()),
            })
        }
    }

    /// Runs one recall leg and converts its rows.
    async fn fact_leg(
        &self,
        statements: &str,
        bindings: impl IntoVariables,
    ) -> Result<Vec<Fact>, MemoryError> {
        let rows: Vec<FactRow> = self
            .query(statements, bindings)
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        Ok(rows.into_iter().map(FactRow::into_fact).collect())
    }
}

#[async_trait]
impl MemoryStore for SurrealStore {
    async fn memory_semantics(&self) -> Result<sovra_core::MemorySemanticState, MemoryError> {
        self.semantic_state().await
    }
    async fn configure_memory_semantics(
        &self,
        target: Option<sovra_core::models::ModelDestination>,
    ) -> Result<sovra_core::MemorySemanticState, MemoryError> {
        self.configure_semantics(target).await
    }
    async fn memory_embedding_inputs(
        &self,
        setting: &sovra_core::MemorySemanticSetting,
        ids: &[FactId],
    ) -> Result<Vec<sovra_core::MemoryEmbeddingInput>, MemoryError> {
        self.embedding_inputs(setting, ids).await
    }
    async fn commit_memory_vectors(
        &self,
        setting: &sovra_core::MemorySemanticSetting,
        policy: sovra_core::MemoryPolicy,
        vectors: Vec<sovra_core::MemoryVector>,
    ) -> Result<(), MemoryError> {
        self.save_vectors(setting, policy, vectors).await
    }
    async fn semantic_memories(
        &self,
        setting: &sovra_core::MemorySemanticSetting,
        vector: &sovra_core::file_index::semantic::EmbeddingVector,
        search: sovra_core::MemorySearch,
        now: Timestamp,
    ) -> Result<Vec<sovra_core::MemorySemanticHit>, MemoryError> {
        self.nearest_memories(setting, vector, search, now).await
    }
    async fn health(&self) -> Result<(), MemoryError> {
        // No .bind(): the unit type is not an object-like variable set.
        let response = self
            .db
            .query("RETURN true;")
            .await
            .map_err(|e| backend(&e))?;
        ensure_ok(response).map(|_| ())
    }

    async fn memory_policy(&self) -> Result<sovra_core::MemoryPolicy, MemoryError> {
        self.read_memory_policy().await
    }
    async fn set_memory_enabled(
        &self,
        enabled: bool,
    ) -> Result<sovra_core::MemoryPolicy, MemoryError> {
        self.change_memory_policy(enabled).await
    }
    async fn enqueue_memory(
        &self,
        input: sovra_core::MemoryInput,
    ) -> Result<EpisodeId, MemoryError> {
        self.enqueue(input).await
    }
    async fn claim_memory(
        &self,
        at: Timestamp,
    ) -> Result<Option<sovra_core::MemoryClaim>, MemoryError> {
        self.claim(at).await
    }
    async fn complete_memory(
        &self,
        claim: &sovra_core::MemoryClaim,
        candidates: Vec<sovra_core::MemoryCandidate>,
        model: &str,
        at: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        self.finish(claim, candidates, model, at).await
    }
    async fn fail_memory(
        &self,
        claim: &sovra_core::MemoryClaim,
        message: &str,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        self.fail(claim, message, at).await
    }
    async fn memory_job(&self, episode: &EpisodeId) -> Result<sovra_core::MemoryJob, MemoryError> {
        self.read_job(episode).await
    }

    async fn record_episode(&self, episode: NewEpisode) -> Result<EpisodeId, MemoryError> {
        episode.validate()?;
        let row = NewEpisodeRow {
            session_id: episode.session_id,
            kind: kind_to_str(episode.kind).to_owned(),
            content: episode.content,
            occurred_at: episode.occurred_at.millis(),
        };
        let mut response = self.query(
            "BEGIN TRANSACTION;
             IF !(SELECT VALUE enabled FROM ONLY meta:remembering) { THROW 'remembering is paused'; };
             UPDATE meta:remembering SET revision += 1;
             LET $created = (CREATE ONLY episode CONTENT $data RETURN VALUE id);
             RETURN $created;
             COMMIT TRANSACTION;",
            ("data", row),
        ).await?;
        let index = response.num_statements().saturating_sub(2);
        let id: Option<RecordId> = response.take(index).map_err(|e| backend(&e))?;
        id.map(|id| EpisodeId(id.to_sql()))
            .ok_or_else(|| MemoryError::Backend("CREATE episode returned nothing".to_owned()))
    }

    async fn episode(&self, id: &EpisodeId) -> Result<Episode, MemoryError> {
        let row: Option<EpisodeRow> = self
            .query(
                "SELECT id, session_id, kind, content, occurred_at FROM ONLY $episode;",
                ("episode", parse_id(&id.0, "episode")?),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        row.ok_or_else(|| MemoryError::NotFound(id.0.clone()))?
            .into_episode()
    }

    async fn recent_episodes(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        if limit > sovra_core::MAX_RECALL_RESULTS
            || session_id.trim().is_empty()
            || session_id.len() > 512
        {
            return Err(MemoryError::InvalidInput(
                "invalid episode history request".into(),
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let rows: Vec<EpisodeRow> = self
            .query(
                "SELECT id, session_id, kind, content, occurred_at FROM episode \
                 WHERE session_id = $session_id ORDER BY occurred_at DESC LIMIT $limit;",
                (
                    "session_id",
                    session_id.to_owned(),
                    "limit",
                    i64::try_from(limit).unwrap_or(i64::MAX),
                ),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter().map(EpisodeRow::into_episode).collect()
    }

    async fn save_fact(&self, fact: NewFact) -> Result<FactId, MemoryError> {
        fact.validate()?;
        if let Some(embedding) = &fact.embedding {
            self.check_embedding(embedding).await?;
        }
        let write = facts::prepare(fact, self.next_fact_ms(), None)?;
        let mut response = self
            .query(
                &format!(
                    "BEGIN TRANSACTION; {} RETURN $created; COMMIT TRANSACTION;",
                    facts::WRITE
                ),
                ("write", write),
            )
            .await?;
        // The SDK includes BEGIN/COMMIT responses; RETURN is immediately before COMMIT.
        let result_index = response.num_statements().saturating_sub(2);
        let id: Option<RecordId> = response.take(result_index).map_err(|e| backend(&e))?;
        id.map(|id| FactId(id.to_sql()))
            .ok_or_else(|| MemoryError::Backend("CREATE fact returned nothing".to_owned()))
    }

    async fn invalidate_fact(&self, id: &FactId, at: Timestamp) -> Result<(), MemoryError> {
        let updated: Vec<RecordId> = self
            .query(
                "UPDATE $fact SET invalid_at = invalid_at ?? $at, expired_at = expired_at ?? $at RETURN VALUE id;",
                ("fact", parse_id(&id.0, "fact")?, "at", at.millis()),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        if updated.is_empty() {
            return Err(MemoryError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    async fn stop_memory(&self, id: &FactId, at: Timestamp) -> Result<(), MemoryError> {
        let updated: Vec<RecordId> = self.query(
            "UPDATE $fact SET stopped_at = stopped_at ?? $at, invalid_at = invalid_at ?? $at, expired_at = expired_at ?? $at RETURN VALUE id;",
            ("fact", parse_id(&id.0, "fact")?, "at", at.millis()),
        ).await?.take(0).map_err(|error| backend(&error))?;
        if updated.is_empty() {
            return Err(MemoryError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    async fn recall(&self, query: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        const FACT_FIELDS: &str = facts::FIELDS;
        query.validate()?;
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        // Over-fetch each leg: temporal/entity clauses post-filter the KNN operator's
        // K results, and RRF fusion needs depth to be meaningful.
        let fetch = (query.limit * 4).max(10);
        let current = if query.include_expired {
            " AND stopped_at IS NONE"
        } else {
            " AND stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE"
        };
        let entity = query
            .entity
            .as_ref()
            .map_or("", |_| " AND $entity IN entities");
        let entity_value = query.entity.clone().unwrap_or_default();
        let valid = query.valid_at.map_or("", |_| " AND (valid_at IS NONE OR valid_at <= $valid) AND (invalid_at IS NONE OR invalid_at > $valid OR ($has_known AND expired_at > $known))");
        let known = query.known_at.map_or(
            "",
            |_| " AND created_at <= $known AND (expired_at IS NONE OR expired_at > $known)",
        );
        let valid_value = query.valid_at.map_or(0, Timestamp::millis);
        let known_value = query.known_at.map_or(0, Timestamp::millis);

        let mut legs = Vec::new();
        if let Some(embedding) = &query.embedding {
            self.check_embedding(embedding).await?;
            let sql = format!(
                "SELECT {FACT_FIELDS}, vector::distance::knn() AS knn_distance FROM fact \
                 WHERE embedding <|{fetch},40|> $vector{current}{entity}{valid}{known} \
                 ORDER BY knn_distance ASC;"
            );
            legs.push(
                self.fact_leg(
                    &sql,
                    (
                        "vector",
                        embedding.vector.clone(),
                        "entity",
                        entity_value.clone(),
                        "valid",
                        valid_value,
                        "known",
                        known_value,
                        "has_known",
                        query.known_at.is_some(),
                    ),
                )
                .await?,
            );
        }
        if let Some(text) = &query.text {
            let sql = format!(
                "SELECT {FACT_FIELDS}, search::score(1) AS text_score FROM fact \
                 WHERE content @1@ $text{current}{entity}{valid}{known} \
                 ORDER BY text_score DESC LIMIT {fetch};"
            );
            legs.push(
                self.fact_leg(
                    &sql,
                    (
                        "text",
                        text.clone(),
                        "entity",
                        entity_value.clone(),
                        "valid",
                        valid_value,
                        "known",
                        known_value,
                        "has_known",
                        query.known_at.is_some(),
                    ),
                )
                .await?,
            );
        }
        if legs.is_empty() {
            let sql = format!(
                "SELECT {FACT_FIELDS} FROM fact \
                 WHERE true{current}{entity}{valid}{known} ORDER BY created_at DESC LIMIT {fetch};"
            );
            legs.push(
                self.fact_leg(
                    &sql,
                    (
                        "entity",
                        entity_value,
                        "valid",
                        valid_value,
                        "known",
                        known_value,
                        "has_known",
                        query.known_at.is_some(),
                    ),
                )
                .await?,
            );
        }
        Ok(fuse_rrf(legs, query.limit))
    }

    async fn list_memories(
        &self,
        query: sovra_core::MemoryListQuery,
    ) -> Result<sovra_core::MemoryPage, MemoryError> {
        self.memory_page(query).await
    }

    async fn memory_fact(&self, id: &FactId) -> Result<Fact, MemoryError> {
        self.exact_fact(id).await
    }

    async fn correct_memory(
        &self,
        correction: sovra_core::MemoryCorrection,
    ) -> Result<FactId, MemoryError> {
        self.user_correction(correction).await
    }

    async fn save_chat_thread(&self, thread: &ChatThread) -> Result<(), MemoryError> {
        let messages_json = serde_json::to_string(&thread.messages)
            .map_err(|e| MemoryError::Backend(format!("serialize chat messages: {e}")))?;
        let record_id = RecordId::new("chat_thread", entity_key(&thread.id));
        let row = ChatThreadRow {
            session_id: thread.id.clone(),
            title: thread.title.clone(),
            kind: match thread.kind {
                ConversationKind::Chat => "chat".to_owned(),
                ConversationKind::Scheduled => "scheduled".to_owned(),
            },
            schedule_id: thread.schedule_id.clone(),
            pinned: thread.pinned,
            created_at: thread.created_at.millis(),
            updated_at: thread.updated_at.millis(),
            last_message_preview: thread.last_message_preview.clone(),
            messages_json,
            deleted_at: thread.deleted_at.map(Timestamp::millis),
        };
        self.query(
            "UPSERT $record CONTENT $data;",
            ("record", record_id, "data", row),
        )
        .await?;
        Ok(())
    }

    async fn list_chat_threads(&self, limit: usize) -> Result<Vec<ChatThread>, MemoryError> {
        let rows: Vec<ChatThreadRow> = self
            .query(
                "SELECT session_id, title, kind, schedule_id, pinned, created_at, updated_at, \
                 last_message_preview, messages_json, deleted_at FROM chat_thread \
                 WHERE deleted_at IS NONE ORDER BY updated_at DESC LIMIT $limit;",
                ("limit", i64::try_from(limit).unwrap_or(i64::MAX)),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter().map(ChatThreadRow::into_thread).collect()
    }

    async fn delete_chat_thread(&self, session_id: &str, at: Timestamp) -> Result<(), MemoryError> {
        let record_id = RecordId::new("chat_thread", entity_key(session_id));
        self.query(
            "UPDATE $record SET deleted_at = $at;",
            ("record", record_id, "at", at.millis()),
        )
        .await?;
        Ok(())
    }

    async fn load_team(&self) -> Result<Option<Vec<TeamMember>>, MemoryError> {
        let rows: Vec<TeamRow> = self
            .query(
                "SELECT members_json FROM $record;",
                ("record", RecordId::new("team", "roster")),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter()
            .next()
            .map(|row| {
                serde_json::from_str(&row.members_json)
                    .map_err(|e| MemoryError::Backend(format!("deserialize team: {e}")))
            })
            .transpose()
    }

    async fn save_team(&self, members: &[TeamMember]) -> Result<(), MemoryError> {
        let members_json = serde_json::to_string(members)
            .map_err(|e| MemoryError::Backend(format!("serialize team: {e}")))?;
        self.query(
            "UPSERT $record CONTENT $data;",
            (
                "record",
                RecordId::new("team", "roster"),
                "data",
                TeamRow { members_json },
            ),
        )
        .await?;
        Ok(())
    }

    async fn list_artifacts(&self) -> Result<Vec<Artifact>, MemoryError> {
        let rows: Vec<ArtifactRow> = self
            .query(
                "SELECT artifact_id, updated_at, artifact_json, deleted_at FROM artifact \
                 WHERE deleted_at IS NONE ORDER BY updated_at DESC LIMIT $limit;",
                // A library far beyond any desktop user's; bounds one query's memory.
                ("limit", 10_000_i64),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter()
            .map(|row| {
                serde_json::from_str(&row.artifact_json)
                    .map_err(|e| MemoryError::Backend(format!("deserialize artifact: {e}")))
            })
            .collect()
    }

    async fn save_artifact(&self, artifact: &Artifact) -> Result<(), MemoryError> {
        let artifact_json = serde_json::to_string(artifact)
            .map_err(|e| MemoryError::Backend(format!("serialize artifact: {e}")))?;
        let row = ArtifactRow {
            artifact_id: artifact.id.clone(),
            updated_at: artifact.updated_at.millis(),
            artifact_json,
            deleted_at: None,
        };
        self.query(
            "UPSERT $record CONTENT $data;",
            (
                "record",
                RecordId::new("artifact", entity_key(&artifact.id)),
                "data",
                row,
            ),
        )
        .await?;
        Ok(())
    }

    async fn delete_artifact(&self, id: &str, at: Timestamp) -> Result<(), MemoryError> {
        self.query(
            "UPDATE $record SET deleted_at = $at;",
            (
                "record",
                RecordId::new("artifact", entity_key(id)),
                "at",
                at.millis(),
            ),
        )
        .await?;
        Ok(())
    }

    async fn load_connections(
        &self,
    ) -> Result<Option<sovra_core::connections::ConnectionsState>, MemoryError> {
        let rows: Vec<TeamRow> = self
            .query(
                "SELECT members_json FROM $record;",
                ("record", RecordId::new("team", "connections")),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter()
            .next()
            .map(|row| {
                serde_json::from_str(&row.members_json)
                    .map_err(|e| MemoryError::Backend(format!("deserialize connections: {e}")))
            })
            .transpose()
    }

    async fn save_connections(
        &self,
        state: &sovra_core::connections::ConnectionsState,
    ) -> Result<(), MemoryError> {
        let members_json = serde_json::to_string(state)
            .map_err(|e| MemoryError::Backend(format!("serialize connections: {e}")))?;
        self.query(
            "UPSERT $record CONTENT $data;",
            (
                "record",
                RecordId::new("team", "connections"),
                "data",
                TeamRow { members_json },
            ),
        )
        .await?;
        Ok(())
    }

    async fn load_auth(&self) -> Result<Option<sovra_core::auth::AuthRecords>, MemoryError> {
        let rows: Vec<TeamRow> = self
            .query(
                "SELECT members_json FROM $record;",
                ("record", RecordId::new("team", "auth")),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        rows.into_iter()
            .next()
            .map(|row| {
                serde_json::from_str(&row.members_json)
                    .map_err(|e| MemoryError::Backend(format!("deserialize auth: {e}")))
            })
            .transpose()
    }

    async fn save_auth(&self, records: &sovra_core::auth::AuthRecords) -> Result<(), MemoryError> {
        let members_json = serde_json::to_string(records)
            .map_err(|e| MemoryError::Backend(format!("serialize auth: {e}")))?;
        self.query(
            "UPSERT $record CONTENT $data;",
            (
                "record",
                RecordId::new("team", "auth"),
                "data",
                TeamRow { members_json },
            ),
        )
        .await?;
        Ok(())
    }
}

// --- rows ---------------------------------------------------------------------------

#[derive(Debug, SurrealValue)]
struct TeamRow {
    members_json: String,
}

#[derive(Debug, SurrealValue)]
struct ArtifactRow {
    artifact_id: String,
    updated_at: i64,
    artifact_json: String,
    deleted_at: Option<i64>,
}

#[derive(Debug, SurrealValue)]
struct ChatThreadRow {
    session_id: String,
    title: String,
    kind: String,
    schedule_id: Option<String>,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
    last_message_preview: String,
    messages_json: String,
    deleted_at: Option<i64>,
}

impl ChatThreadRow {
    fn into_thread(self) -> Result<ChatThread, MemoryError> {
        let messages = serde_json::from_str(&self.messages_json)
            .map_err(|e| MemoryError::Backend(format!("deserialize chat messages: {e}")))?;
        let kind = match self.kind.as_str() {
            "scheduled" => ConversationKind::Scheduled,
            _ => ConversationKind::Chat,
        };
        Ok(ChatThread {
            id: self.session_id,
            title: self.title,
            kind,
            schedule_id: self.schedule_id,
            pinned: self.pinned,
            created_at: Timestamp(self.created_at),
            updated_at: Timestamp(self.updated_at),
            last_message_preview: self.last_message_preview,
            messages,
            deleted_at: self.deleted_at.map(Timestamp),
        })
    }
}

#[derive(Debug, SurrealValue)]
struct NewEpisodeRow {
    session_id: String,
    kind: String,
    content: String,
    occurred_at: i64,
}

#[derive(Debug, SurrealValue)]
struct EpisodeRow {
    id: RecordId,
    session_id: String,
    kind: String,
    content: String,
    occurred_at: i64,
}

impl EpisodeRow {
    fn into_episode(self) -> Result<Episode, MemoryError> {
        Ok(Episode {
            id: EpisodeId(self.id.to_sql()),
            session_id: self.session_id,
            kind: kind_from_str(&self.kind)?,
            content: self.content,
            occurred_at: Timestamp(self.occurred_at),
        })
    }
}

#[derive(Debug, SurrealValue)]
struct EmbeddingIndexRow {
    model: Option<String>,
    dimension: usize,
}

#[derive(Debug, SurrealValue)]
struct FactRow {
    evidence: Option<String>,
    extractor: Option<String>,
    id: RecordId,
    content: String,
    entities: Option<Vec<String>>,
    source: Option<RecordId>,
    replaces: Option<RecordId>,
    created_at: i64,
    valid_at: Option<i64>,
    invalid_at: Option<i64>,
    expired_at: Option<i64>,
    stopped_at: Option<i64>,
}

impl FactRow {
    fn into_fact(self) -> Fact {
        Fact {
            id: FactId(self.id.to_sql()),
            content: self.content,
            entities: self.entities.unwrap_or_default(),
            source: self.source.map(|id| EpisodeId(id.to_sql())),
            replaces: self.replaces.map(|id| FactId(id.to_sql())),
            extraction: self
                .extractor
                .zip(self.evidence)
                .map(|(model, evidence)| sovra_core::MemoryExtraction { model, evidence }),
            created_at: Timestamp(self.created_at),
            valid_at: self.valid_at.map(Timestamp),
            invalid_at: self.invalid_at.map(Timestamp),
            expired_at: self.expired_at.map(Timestamp),
            stopped_at: self.stopped_at.map(Timestamp),
        }
    }
}

// --- helpers ------------------------------------------------------------------------

fn kind_to_str(kind: EpisodeKind) -> &'static str {
    match kind {
        EpisodeKind::UserMessage => "user_message",
        EpisodeKind::AgentMessage => "agent_message",
        EpisodeKind::ToolCall => "tool_call",
        EpisodeKind::ToolResult => "tool_result",
        EpisodeKind::Note => "note",
    }
}

fn kind_from_str(kind: &str) -> Result<EpisodeKind, MemoryError> {
    match kind {
        "user_message" => Ok(EpisodeKind::UserMessage),
        "agent_message" => Ok(EpisodeKind::AgentMessage),
        "tool_call" => Ok(EpisodeKind::ToolCall),
        "tool_result" => Ok(EpisodeKind::ToolResult),
        "note" => Ok(EpisodeKind::Note),
        other => Err(MemoryError::Backend(format!(
            "unknown episode kind `{other}`"
        ))),
    }
}

/// Deterministic entity record key so the same name always upserts one record.
fn entity_key(name: &str) -> String {
    let mut key: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    key.truncate(64);
    if key.is_empty() {
        key.push('_');
    }
    key
}

fn parse_id(id: &str, table: &str) -> Result<RecordId, MemoryError> {
    let record = RecordId::parse_simple(id)
        .map_err(|_| MemoryError::InvalidInput("invalid memory record id".into()))?;
    if record.table != table.into() {
        return Err(MemoryError::InvalidInput("wrong memory record type".into()));
    }
    Ok(record)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn backend(error: &surrealdb::Error) -> MemoryError {
    MemoryError::Backend(error.to_string())
}

fn unavailable(error: &surrealdb::Error) -> MemoryError {
    MemoryError::Unavailable(error.to_string())
}

/// Surfaces per-statement errors, which `.await` alone does not.
fn ensure_ok(mut response: IndexedResults) -> Result<IndexedResults, MemoryError> {
    let errors = response.take_errors();
    if let Some((index, error)) = errors.into_iter().next() {
        return Err(MemoryError::Backend(format!("statement {index}: {error}")));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn version_five_keeps_expiration_distinct_from_withdrawal_when_adding_aliases() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let id = store
            .save_fact(NewFact {
                content: "Known expiration".into(),
                entities: vec![],
                source: None,
                embedding: None,
                valid_at: Some(Timestamp(1)),
                replaces: None,
            })
            .await
            .unwrap();
        store.invalidate_fact(&id, Timestamp(2)).await.unwrap();
        store
            .db
            .query("UPDATE meta:schema SET version = 5;")
            .await
            .unwrap()
            .check()
            .unwrap();
        store.apply_schema().await.unwrap();
        let fact = store.memory_fact(&id).await.unwrap();
        assert_eq!(fact.stopped_at, None);
        assert_eq!(fact.expired_at, Some(Timestamp(2)));
        let version: Option<i64> = store
            .db
            .query("SELECT VALUE version FROM ONLY meta:schema;")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(version, Some(6));
    }

    #[tokio::test]
    async fn version_four_migrates_old_stops_without_suppressing_correction_history() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let old = store
            .save_fact(NewFact {
                content: "Old preference".into(),
                entities: vec![],
                source: None,
                embedding: None,
                valid_at: Some(Timestamp(1)),
                replaces: None,
            })
            .await
            .unwrap();
        let new = store
            .save_fact(NewFact {
                content: "New preference".into(),
                entities: vec![],
                source: None,
                embedding: None,
                valid_at: Some(Timestamp(2)),
                replaces: Some(old.clone()),
            })
            .await
            .unwrap();
        store.invalidate_fact(&new, Timestamp(3)).await.unwrap();
        store
            .db
            .query("UPDATE meta:schema SET version = 4;")
            .await
            .unwrap()
            .check()
            .unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.memory_fact(&new).await.unwrap().stopped_at,
            Some(Timestamp(3))
        );
        assert_eq!(store.memory_fact(&old).await.unwrap().stopped_at, None);
        let mut query = RecallQuery::recent(10);
        query.include_expired = true;
        assert_eq!(
            store
                .recall(query)
                .await
                .unwrap()
                .iter()
                .map(|hit| hit.fact.id.clone())
                .collect::<Vec<_>>(),
            vec![old]
        );
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.memory_fact(&new).await.unwrap().stopped_at,
            Some(Timestamp(3))
        );
    }

    #[tokio::test]
    async fn version_three_adds_semantic_memory_without_consent_or_policy_changes() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let id = store
            .save_fact(NewFact {
                content: "Existing saved detail".into(),
                entities: vec![],
                embedding: None,
                source: None,
                valid_at: None,
                replaces: None,
            })
            .await
            .unwrap();
        store.set_memory_enabled(false).await.unwrap();
        let policy = store.memory_policy().await.unwrap();
        store
            .db
            .query("UPDATE meta:schema SET version = 3; DELETE meta:memory_semantic;")
            .await
            .unwrap()
            .check()
            .unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(store.memory_policy().await.unwrap(), policy);
        assert_eq!(
            store.memory_semantics().await.unwrap(),
            sovra_core::MemorySemanticState::default()
        );
        assert_eq!(
            store.memory_fact(&id).await.unwrap().content,
            "Existing saved detail"
        );
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.memory_semantics().await.unwrap(),
            sovra_core::MemorySemanticState::default()
        );
    }

    #[tokio::test]
    async fn version_two_adds_ingestion_without_changing_existing_evidence() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let source = store
            .record_episode(NewEpisode {
                session_id: "legacy".into(),
                kind: EpisodeKind::UserMessage,
                content: "Original evidence".into(),
                occurred_at: Timestamp(1),
            })
            .await
            .unwrap();
        store.db.query("UPDATE meta:schema SET version = 2; DELETE meta:remembering; REMOVE TABLE memory_job;")
            .await.unwrap().check().unwrap();
        store.apply_schema().await.unwrap();
        assert_eq!(
            store.episode(&source).await.unwrap().content,
            "Original evidence"
        );
        assert!(store.memory_policy().await.unwrap().enabled);
        assert!(store.claim_memory(Timestamp(2)).await.unwrap().is_none());
        store.set_memory_enabled(false).await.unwrap();
        store.apply_schema().await.unwrap();
        assert!(!store.memory_policy().await.unwrap().enabled);
    }

    #[tokio::test]
    async fn failed_write_rolls_back_entities_edges_and_embedding_pin() {
        let mut config = StoreConfig::in_memory();
        config.embedding_dimension = 4;
        let store = SurrealStore::open(config).await.unwrap();
        let result = store
            .save_fact(NewFact {
                content: "must be rolled back".into(),
                entities: vec!["Maya".into()],
                embedding: Some(Embedding {
                    model: "first-model".into(),
                    vector: vec![1.0; 4],
                }),
                source: Some(EpisodeId("episode:missing".into())),
                valid_at: None,
                replaces: None,
            })
            .await;
        assert!(result.is_err());
        for table in ["fact", "entity", "mentions", "derived_from", "supersedes"] {
            let count: Option<i64> = store
                .db
                .query(format!("SELECT count() AS count FROM {table} GROUP ALL;"))
                .await
                .unwrap()
                .check()
                .unwrap()
                .take("count")
                .unwrap();
            assert_eq!(count.unwrap_or(0), 0, "partial rows in {table}");
        }
        assert!(
            store
                .embedding_index()
                .await
                .unwrap()
                .unwrap()
                .model
                .is_none()
        );
    }

    #[tokio::test]
    async fn migration_rejects_mixed_legacy_models_without_changing_schema() {
        let mut config = StoreConfig::in_memory();
        config.embedding_dimension = 4;
        let store = SurrealStore::open(config).await.unwrap();
        store.db.query(
            "UPDATE meta:schema SET version = 1; DELETE meta:embedding;
             CREATE fact:a SET content = 'one', embedding = [1,0,0,0], embed_model = 'a', created_at = 1;
             CREATE fact:b SET content = 'two', embedding = [0,1,0,0], embed_model = 'b', created_at = 2;"
        ).await.unwrap().check().unwrap();
        assert!(matches!(
            store.apply_schema().await,
            Err(MemoryError::InvalidInput(_))
        ));
        let version: Option<i64> = store
            .db
            .query("SELECT VALUE version FROM ONLY meta:schema;")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(version, Some(1));
        assert_eq!(
            store.recall(RecallQuery::recent(10)).await.unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn entity_edges_preserve_distinct_names_and_legacy_links() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        store
            .db
            .query("CREATE entity:maya SET name = 'Maya';")
            .await
            .unwrap()
            .check()
            .unwrap();
        let mut expected = vec!["A-B", "A B", "Māya", "李", "王", "Maya"];
        for name in &expected {
            store
                .save_fact(NewFact {
                    content: format!("about {name}"),
                    entities: vec![(*name).into()],
                    source: None,
                    embedding: None,
                    valid_at: None,
                    replaces: None,
                })
                .await
                .unwrap();
        }
        let rows: Vec<Vec<String>> = store
            .db
            .query("SELECT VALUE ->mentions->entity.name FROM fact;")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        let mut actual: Vec<String> = rows.into_iter().flatten().collect();
        actual.sort();
        expected.sort_unstable();
        assert_eq!(actual, expected);
        let names: Vec<String> = store
            .db
            .query("SELECT VALUE name FROM entity;")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(
            names.len(),
            expected.len(),
            "legacy names must be reused, not duplicated"
        );
    }

    #[tokio::test]
    async fn schema_reapplication_rejects_incompatible_indexes_and_newer_versions() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        let mut changed = store.clone();
        changed.config.embedding_dimension = 4;
        assert!(
            changed.apply_schema().await.is_err(),
            "never rebuild an existing index at another dimension"
        );
        store
            .db
            .query("UPDATE meta:schema SET version = 999;")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(
            store.apply_schema().await.is_err(),
            "never downgrade a newer schema"
        );
        let version: Option<i64> = store
            .db
            .query("SELECT VALUE version FROM ONLY meta:schema;")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(version, Some(999));
    }

    #[tokio::test]
    async fn legacy_index_adopts_its_existing_model_without_rewriting_facts() {
        let store = SurrealStore::open(StoreConfig::in_memory()).await.unwrap();
        store
            .db
            .query("UPDATE meta:schema SET version = 1; DELETE meta:embedding;")
            .await
            .unwrap()
            .check()
            .unwrap();
        store.db.query("CREATE fact:legacy SET content = 'legacy', embedding = $vector, embed_model = 'old-model', created_at = 1;")
            .bind(("vector", vec![1.0_f32; 768])).await.unwrap().check().unwrap();
        store.apply_schema().await.unwrap();
        let model: Option<String> = store
            .db
            .query("SELECT VALUE model FROM ONLY meta:embedding;")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(model.as_deref(), Some("old-model"));
        assert_eq!(
            store.recall(RecallQuery::recent(1)).await.unwrap()[0]
                .fact
                .content,
            "legacy"
        );
    }

    #[test]
    fn entity_keys_are_deterministic_and_safe() {
        assert_eq!(entity_key("Shaun Barney"), "shaun_barney");
        assert_eq!(entity_key("Shaun Barney"), entity_key("Shaun Barney"));
        assert_eq!(entity_key("??"), "__");
        assert_eq!(entity_key(""), "_");
    }

    #[test]
    fn episode_kinds_round_trip() {
        for kind in [
            EpisodeKind::UserMessage,
            EpisodeKind::AgentMessage,
            EpisodeKind::ToolCall,
            EpisodeKind::ToolResult,
            EpisodeKind::Note,
        ] {
            assert_eq!(kind_from_str(kind_to_str(kind)).unwrap(), kind);
        }
    }

    #[test]
    fn addresses_map_to_engine_schemes() {
        assert_eq!(StoreConfig::in_memory().address(), "mem://");
        assert_eq!(
            StoreConfig::on_disk("/tmp/db").address(),
            "surrealkv:///tmp/db"
        );
        let remote = StoreConfig::new(StoreLocation::Remote {
            url: "ws://localhost:8000".into(),
        });
        assert_eq!(remote.address(), "ws://localhost:8000");
    }
}
