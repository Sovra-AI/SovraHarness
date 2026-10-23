//! `SurrealDB` adapter for the [`sovra_core::MemoryStore`] port — the only crate that
//! names `SurrealDB`.
//!
//! One connection API covers every deployment via `surrealdb::engine::any`:
//! in-memory (tests), embedded on disk (the desktop default) and a remote server
//! (`ws://`) later — swapping is a [`StoreLocation`] change in the composition root,
//! and swapping the whole backend is another `sovra-store-*` crate implementing the
//! same port.
//!
//! Writes that touch several records (fact + entity + edges) run as sequential
//! statements, not one transaction: this is a single-process local store and the
//! partial-failure window is acceptable for v1. Revisit with `db.begin()` if a
//! remote multi-writer deployment ever ships.

mod schema;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sovra_core::{
    Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryError, MemoryStore, NewEpisode,
    NewFact, RecallQuery, RecalledFact, Timestamp, fuse_rrf,
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
    /// the first embedding saved pins nothing, but dimension is still enforced.
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
        let store = Self { db, config };
        store.apply_schema().await?;
        Ok(store)
    }

    async fn apply_schema(&self) -> Result<(), MemoryError> {
        let statements = format!(
            "{}\n{}\nUPSERT meta:schema SET version = $version;",
            schema::STATIC_SCHEMA,
            schema::hnsw_index(self.config.embedding_dimension),
        );
        self.query(&statements, ("version", schema::SCHEMA_VERSION))
            .await?;
        Ok(())
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

    fn check_embedding(&self, embedding: &Embedding) -> Result<(), MemoryError> {
        let dimension_ok = embedding.dimension() == self.config.embedding_dimension;
        let model_ok = self
            .config
            .embedding_model
            .as_ref()
            .is_none_or(|indexed| *indexed == embedding.model);
        if dimension_ok && model_ok {
            Ok(())
        } else {
            Err(MemoryError::EmbeddingMismatch {
                indexed: self
                    .config
                    .embedding_model
                    .clone()
                    .unwrap_or_else(|| "unspecified".to_owned()),
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
    async fn health(&self) -> Result<(), MemoryError> {
        // No .bind(): the unit type is not an object-like variable set.
        let response = self
            .db
            .query("RETURN true;")
            .await
            .map_err(|e| backend(&e))?;
        ensure_ok(response).map(|_| ())
    }

    async fn record_episode(&self, episode: NewEpisode) -> Result<EpisodeId, MemoryError> {
        let row = NewEpisodeRow {
            session_id: episode.session_id,
            kind: kind_to_str(episode.kind).to_owned(),
            content: episode.content,
            occurred_at: episode.occurred_at.millis(),
        };
        let id: Option<RecordId> = self
            .query(
                "CREATE ONLY episode CONTENT $data RETURN VALUE id;",
                ("data", row),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        id.map(|id| EpisodeId(id.to_sql()))
            .ok_or_else(|| MemoryError::Backend("CREATE episode returned nothing".to_owned()))
    }

    async fn recent_episodes(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
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
        if let Some(embedding) = &fact.embedding {
            self.check_embedding(embedding)?;
        }
        let now = now_ms();
        let row = NewFactRow {
            content: fact.content,
            entities: fact.entities.clone(),
            embedding: fact.embedding.as_ref().map(|e| e.vector.clone()),
            embed_model: fact.embedding.map(|e| e.model),
            created_at: now,
            valid_at: fact.valid_at.map(Timestamp::millis),
        };
        let id: Option<RecordId> = self
            .query(
                "CREATE ONLY fact CONTENT $data RETURN VALUE id;",
                ("data", row),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        let fact_id =
            id.ok_or_else(|| MemoryError::Backend("CREATE fact returned nothing".to_owned()))?;

        for name in &fact.entities {
            // Deterministic record id, built in Rust and bound directly: the same
            // entity name always upserts the same record.
            let entity = RecordId::new("entity", entity_key(name));
            self.query(
                "UPSERT $entity SET name = $name; RELATE $fact->mentions->$entity;",
                (
                    "entity",
                    entity,
                    "name",
                    name.clone(),
                    "fact",
                    fact_id.clone(),
                ),
            )
            .await?;
        }
        if let Some(source) = &fact.source {
            self.query(
                "RELATE $fact->derived_from->$episode;",
                ("fact", fact_id.clone(), "episode", parse_id(&source.0)?),
            )
            .await?;
        }
        if let Some(replaced) = &fact.replaces {
            self.query(
                "UPDATE $old SET invalid_at = $at, expired_at = $at; \
                 RELATE $fact->supersedes->$old;",
                (
                    "old",
                    parse_id(&replaced.0)?,
                    "at",
                    now,
                    "fact",
                    fact_id.clone(),
                ),
            )
            .await?;
        }
        Ok(FactId(fact_id.to_sql()))
    }

    async fn invalidate_fact(&self, id: &FactId, at: Timestamp) -> Result<(), MemoryError> {
        let updated: Vec<RecordId> = self
            .query(
                "UPDATE $fact SET invalid_at = $at, expired_at = $at RETURN VALUE id;",
                ("fact", parse_id(&id.0)?, "at", at.millis()),
            )
            .await?
            .take(0)
            .map_err(|e| backend(&e))?;
        if updated.is_empty() {
            return Err(MemoryError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    async fn recall(&self, query: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        const FACT_FIELDS: &str =
            "id, content, entities, created_at, valid_at, invalid_at, expired_at";
        // Over-fetch each leg: temporal/entity clauses post-filter the KNN operator's
        // K results, and RRF fusion needs depth to be meaningful.
        let fetch = (query.limit * 4).max(10);
        let current = if query.include_expired {
            ""
        } else {
            " AND invalid_at IS NONE AND expired_at IS NONE"
        };
        let entity = query
            .entity
            .as_ref()
            .map_or("", |_| " AND $entity IN entities");
        let entity_value = query.entity.clone().unwrap_or_default();

        let mut legs = Vec::new();
        if let Some(embedding) = &query.embedding {
            self.check_embedding(embedding)?;
            let sql = format!(
                "SELECT {FACT_FIELDS}, vector::distance::knn() AS knn_distance FROM fact \
                 WHERE embedding <|{fetch},40|> $vector{current}{entity} \
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
                    ),
                )
                .await?,
            );
        }
        if let Some(text) = &query.text {
            let sql = format!(
                "SELECT {FACT_FIELDS}, search::score(1) AS text_score FROM fact \
                 WHERE content @1@ $text{current}{entity} \
                 ORDER BY text_score DESC LIMIT {fetch};"
            );
            legs.push(
                self.fact_leg(&sql, ("text", text.clone(), "entity", entity_value.clone()))
                    .await?,
            );
        }
        if legs.is_empty() {
            let sql = format!(
                "SELECT {FACT_FIELDS} FROM fact \
                 WHERE true{current}{entity} ORDER BY created_at DESC LIMIT {fetch};"
            );
            legs.push(self.fact_leg(&sql, ("entity", entity_value)).await?);
        }
        Ok(fuse_rrf(legs, query.limit))
    }
}

// --- rows ---------------------------------------------------------------------------

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
struct NewFactRow {
    content: String,
    entities: Vec<String>,
    embedding: Option<Vec<f32>>,
    embed_model: Option<String>,
    created_at: i64,
    valid_at: Option<i64>,
}

#[derive(Debug, SurrealValue)]
struct FactRow {
    id: RecordId,
    content: String,
    entities: Option<Vec<String>>,
    created_at: i64,
    valid_at: Option<i64>,
    invalid_at: Option<i64>,
    expired_at: Option<i64>,
}

impl FactRow {
    fn into_fact(self) -> Fact {
        Fact {
            id: FactId(self.id.to_sql()),
            content: self.content,
            entities: self.entities.unwrap_or_default(),
            created_at: Timestamp(self.created_at),
            valid_at: self.valid_at.map(Timestamp),
            invalid_at: self.invalid_at.map(Timestamp),
            expired_at: self.expired_at.map(Timestamp),
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

fn parse_id(id: &str) -> Result<RecordId, MemoryError> {
    RecordId::parse_simple(id)
        .map_err(|e| MemoryError::Backend(format!("bad record id `{id}`: {e}")))
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
