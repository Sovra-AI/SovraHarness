//! Memory domain: episodes, facts and the [`MemoryStore`] port.
//!
//! The model is a small bi-temporal memory graph (the shape proven by Zep's Graphiti):
//! - an **episode** is a raw, immutable record of something that happened (a user message,
//!   an agent reply, a tool call). Episodes are provenance; they are searched by text and
//!   recency, never embedded.
//! - a **fact** is a distilled statement extracted from episodes, carrying an embedding and
//!   four temporal fields: `created_at` (ingestion), `valid_at` (when it became true),
//!   `invalid_at` (when it stopped being true) and `expired_at` (when the system superseded
//!   it). Facts are never edited or deleted — supersession sets the temporal fields,
//!   preserving what the agent believed and when.
//!
//! This module is pure: no clocks (callers pass [`Timestamp`]s), no IO, no backend types.
//! Storage backends implement [`MemoryStore`]; the composition root picks one.

use async_trait::async_trait;

mod dependencies;
pub use dependencies::*;
mod admission;
pub use admission::*;
mod team_capture;
pub use team_capture::TeamMemoryCapture;
mod sources;
pub use sources::*;
mod ingestion;
pub use ingestion::*;
mod consolidation;
pub use consolidation::consolidation_context;
mod recall;
pub use recall::*;
mod citations;
pub use citations::{
    MAX_MEMORY_CITATIONS, MemoryCitationRef, MemoryCitationTarget, hide_memory_citation_tokens,
    memory_citation_refs, memory_citations, memory_source_citations,
};
mod lifecycle;
pub use lifecycle::*;
mod exploration;
pub use exploration::*;
mod graph;
pub use graph::*;
mod profile;
pub use profile::*;
mod semantic;
pub use semantic::*;

/// Public result bounds prevent an accidental unbounded memory scan or prompt.
pub const MAX_RECALL_RESULTS: usize = 200;
const MAX_FACT_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 4 * 1024;
/// Maximum original-evidence size admitted as one immutable source.
pub const MAX_EPISODE_BYTES: usize = 128 * 1024;

/// Milliseconds since the Unix epoch. Core has no clock — adapters and the runtime
/// construct these at their edges.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub const fn millis(self) -> i64 {
        self.0
    }
}

/// Opaque backend-assigned episode key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct EpisodeId(pub String);

/// Opaque backend-assigned fact key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FactId(pub String);

/// What kind of event an episode records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeKind {
    UserMessage,
    AgentMessage,
    ToolCall,
    ToolResult,
    /// A note the user asked the agent to remember verbatim.
    Note,
}

/// A vector embedding, tagged with the model that produced it so mixed-model stores
/// can refuse apples-to-oranges comparisons.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Embedding {
    pub model: String,
    pub vector: Vec<f32>,
}

impl Embedding {
    pub fn dimension(&self) -> usize {
        self.vector.len()
    }

    /// Cosine similarity is undefined for zero vectors and non-finite components.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.model.trim().is_empty()
            || self.model.len() > 512
            || self.vector.is_empty()
            || self.vector.len() > 16_384
            || self.vector.iter().any(|value| !value.is_finite())
            || !self.vector.iter().any(|value| *value != 0.0)
        {
            return Err(MemoryError::InvalidInput("invalid embedding".into()));
        }
        Ok(())
    }
}

/// An episode to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEpisode {
    pub session_id: String,
    pub kind: EpisodeKind,
    pub content: String,
    pub occurred_at: Timestamp,
}

impl NewEpisode {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.session_id.trim().is_empty()
            || self.session_id.len() > 512
            || self.content.trim().is_empty()
            || self.content.len() > MAX_EPISODE_BYTES
        {
            return Err(MemoryError::InvalidInput("invalid memory episode".into()));
        }
        Ok(())
    }
}

/// A stored episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    pub id: EpisodeId,
    pub session_id: String,
    pub kind: EpisodeKind,
    pub content: String,
    pub occurred_at: Timestamp,
}

/// A fact to save, with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    pub content: String,
    /// Names of entities this fact is about (e.g. "Shaun", "SovraHarness").
    pub entities: Vec<String>,
    pub embedding: Option<Embedding>,
    /// The episode this fact was extracted from, when known.
    pub source: Option<EpisodeId>,
    /// When the fact became true in the world; defaults to ingestion time when unknown.
    pub valid_at: Option<Timestamp>,
    /// A fact this one supersedes: the store invalidates it and links the two.
    pub replaces: Option<FactId>,
}

impl NewFact {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.content.trim().is_empty() || self.content.len() > MAX_FACT_BYTES {
            return Err(MemoryError::InvalidInput(
                "memory text is empty or too long".into(),
            ));
        }
        if self.entities.len() > 32
            || self
                .entities
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(MemoryError::InvalidInput("invalid memory entities".into()));
        }
        if let Some(embedding) = &self.embedding {
            embedding.validate()?;
        }
        Ok(())
    }
}

/// A stored fact with bi-temporal validity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub id: FactId,
    pub content: String,
    pub entities: Vec<String>,
    /// Immutable evidence for this statement, if the original writer supplied it.
    pub source: Option<EpisodeId>,
    /// Earlier belief replaced by this statement; retained for correction history.
    pub replaces: Option<FactId>,
    /// Model-derived interpretations retain their exact evidence and model identity.
    pub extraction: Option<MemoryExtraction>,
    /// Ingestion time (always set by the store).
    pub created_at: Timestamp,
    /// When the fact became true in the world, when known.
    pub valid_at: Option<Timestamp>,
    /// When the fact stopped being true in the world.
    pub invalid_at: Option<Timestamp>,
    /// When the system superseded it (never deleted).
    pub expired_at: Option<Timestamp>,
    /// Explicit user withdrawal; excluded from recall even for earlier dates.
    pub stopped_at: Option<Timestamp>,
}

impl Fact {
    /// A fact the agent should still believe.
    pub fn is_current(&self) -> bool {
        self.stopped_at.is_none() && self.invalid_at.is_none() && self.expired_at.is_none()
    }
}

/// What to recall. Hybrid retrieval: a vector leg (`embedding`) and/or a full-text leg
/// (`text`); the store runs each leg it can and fuses them with [`fuse_rrf`]. With neither
/// set, recall is recency-ordered.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallQuery {
    pub embedding: Option<Embedding>,
    /// Full-text (BM25) query leg.
    pub text: Option<String>,
    /// Restrict to facts mentioning this entity name, when set.
    pub entity: Option<String>,
    pub limit: usize,
    /// Include superseded facts (history view). Defaults to current facts only.
    pub include_expired: bool,
    /// World time at which a claim should apply. Use `include_expired` for history.
    pub valid_at: Option<Timestamp>,
    /// Recorded knowledge cutoff; later corrections must not change earlier beliefs.
    pub known_at: Option<Timestamp>,
}

impl RecallQuery {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.limit > MAX_RECALL_RESULTS {
            return Err(MemoryError::InvalidInput(format!(
                "request at most {MAX_RECALL_RESULTS} memories at a time"
            )));
        }
        if self
            .text
            .as_ref()
            .is_some_and(|text| text.trim().is_empty() || text.len() > MAX_QUERY_BYTES)
            || self
                .entity
                .as_ref()
                .is_some_and(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(MemoryError::InvalidInput("invalid memory search".into()));
        }
        if let Some(embedding) = &self.embedding {
            embedding.validate()?;
        }
        Ok(())
    }

    /// Recency-ordered recall of current facts.
    pub fn recent(limit: usize) -> Self {
        Self {
            embedding: None,
            text: None,
            entity: None,
            limit,
            include_expired: false,
            valid_at: None,
            known_at: None,
        }
    }

    /// Hybrid semantic + full-text recall of current facts.
    pub fn semantic(embedding: Embedding, text: impl Into<String>, limit: usize) -> Self {
        Self {
            embedding: Some(embedding),
            text: Some(text.into()),
            entity: None,
            limit,
            include_expired: false,
            valid_at: None,
            known_at: None,
        }
    }
}

/// A recalled fact with its retrieval score (higher is better). For hybrid recall the
/// score is the reciprocal-rank-fusion score from [`fuse_rrf`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecalledFact {
    pub fact: Fact,
    pub score: f32,
}

/// Standard RRF damping constant; rank contributions are `1 / (K + rank)`.
pub const RRF_K: f32 = 60.0;

/// Fuses ranked result legs (vector, full-text, …) with reciprocal rank fusion — the
/// weight-free baseline used by Graphiti/Zep. Pure, so stores share one tested fusion.
///
/// Each leg is best-first. A fact's score is the sum over legs of `1 / (RRF_K + rank)`;
/// ties keep first-seen (leg order) priority.
pub fn fuse_rrf(legs: Vec<Vec<Fact>>, limit: usize) -> Vec<RecalledFact> {
    ranked_fusion(legs, limit, |fact| fact.id.clone())
        .into_iter()
        .map(|(fact, score)| RecalledFact { fact, score })
        .collect()
}

fn ranked_fusion<T, K: Clone + Eq + std::hash::Hash>(
    legs: Vec<Vec<T>>,
    limit: usize,
    key: impl Fn(&T) -> K,
) -> Vec<(T, f32)> {
    let mut order = Vec::new();
    let mut fused: std::collections::HashMap<K, (T, f32)> = std::collections::HashMap::new();
    for leg in legs {
        let mut seen = std::collections::HashSet::new();
        for (rank, item) in leg
            .into_iter()
            .filter(|item| seen.insert(key(item)))
            .enumerate()
        {
            #[allow(clippy::cast_precision_loss, reason = "ranks are tiny")]
            let contribution = 1.0 / (RRF_K + 1.0 + rank as f32);
            let id = key(&item);
            if let Some(existing) = fused.get_mut(&id) {
                existing.1 += contribution;
            } else {
                order.push(id.clone());
                fused.insert(id, (item, contribution));
            }
        }
    }
    let mut results: Vec<(T, f32)> = order
        .into_iter()
        .filter_map(|id| fused.remove(&id))
        .collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

/// Backend-neutral storage errors. Adapters map their native errors into these,
/// keeping backend types out of every caller.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("invalid memory request: {0}")]
    InvalidInput(String),
    #[error("memory store unavailable: {0}")]
    Unavailable(String),
    #[error("memory query failed: {0}")]
    Backend(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(
        "embedding model mismatch: store is indexed for `{indexed}` ({dimension} dims), got `{got}`"
    )]
    EmbeddingMismatch {
        indexed: String,
        dimension: usize,
        got: String,
    },
}

/// The storage port. Implementations live in adapter crates (`sovra-store-*`);
/// everything else holds an `Arc<dyn MemoryStore>` and never names a backend.
///
/// The trait is async but runtime-agnostic: implementations choose their executor,
/// and the interface stays pure.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Atomically admits explicitly attributed evidence and any required processing.
    /// Adapters must preserve identity/version and never infer permission from content.
    async fn admit_memory_source(
        &self,
        _source: MemorySourceAdmission,
    ) -> Result<MemorySourceReceipt, MemoryError> {
        Err(MemoryError::Backend(
            "Attributed source admission is unavailable.".into(),
        ))
    }
    /// Explicit destination consent is separate from folder embedding permissions.
    async fn memory_semantics(&self) -> Result<MemorySemanticState, MemoryError> {
        Ok(MemorySemanticState::default())
    }
    async fn configure_memory_semantics(
        &self,
        _target: Option<crate::models::ModelDestination>,
    ) -> Result<MemorySemanticState, MemoryError> {
        Err(MemoryError::Backend(
            "Semantic memory is unavailable.".into(),
        ))
    }
    /// Checks only the supplied bounded page; callers own the pagination cursor.
    async fn memory_embedding_inputs(
        &self,
        _setting: &MemorySemanticSetting,
        _ids: &[FactId],
    ) -> Result<Vec<MemoryEmbeddingInput>, MemoryError> {
        Err(MemoryError::Backend(
            "Semantic memory is unavailable.".into(),
        ))
    }
    /// The entire batch must match the active index and remembering policy generation.
    async fn commit_memory_vectors(
        &self,
        _setting: &MemorySemanticSetting,
        _policy: MemoryPolicy,
        _vectors: Vec<MemoryVector>,
    ) -> Result<(), MemoryError> {
        Err(MemoryError::Backend(
            "Semantic memory is unavailable.".into(),
        ))
    }
    async fn semantic_memories(
        &self,
        _setting: &MemorySemanticSetting,
        _vector: &crate::file_index::semantic::EmbeddingVector,
        _search: MemorySearch,
        _now: Timestamp,
    ) -> Result<Vec<MemorySemanticHit>, MemoryError> {
        Err(MemoryError::Backend(
            "Semantic memory is unavailable.".into(),
        ))
    }

    /// Revision of factual content, independent of queue bookkeeping.
    async fn memory_revision(&self) -> Result<u64, MemoryError> {
        Ok(0)
    }

    /// Bounded review neighborhood from real subject/source/version links.
    /// This is user-requested review, so explicit version links may include history.
    async fn memory_connections(&self, _id: &FactId) -> Result<MemoryNeighborhood, MemoryError> {
        Ok(MemoryNeighborhood::default())
    }

    /// Bounded original evidence, independent of extracted facts. Unknown scopes
    /// and revoked sources must be excluded by the adapter before returning.
    async fn search_memory_sources(
        &self,
        _query: MemorySearch,
        _now: Timestamp,
    ) -> Result<Vec<MemorySourceEvidence>, MemoryError> {
        Ok(Vec::new())
    }

    /// Explicitly revoke an original source and cancel its pending processing.
    /// Retain its content for review; derived facts must no longer enter recall.
    async fn stop_memory_source(&self, _id: &EpisodeId, _at: Timestamp) -> Result<(), MemoryError> {
        Err(MemoryError::Backend(
            "Stopping original evidence is unavailable.".into(),
        ))
    }

    /// Preceding admitted sources in the same conversation, oldest first. The
    /// adapter must exclude withdrawn and unauthorized evidence before returning.
    async fn memory_preceding_sources(&self, _id: &EpisodeId) -> Result<Vec<Episode>, MemoryError> {
        Ok(Vec::new())
    }

    /// Batch fact provenance for recall. Return only explicitly permitted user
    /// evidence under current pause/withdrawal policy; omit unknown grants.
    /// Raw `episode` reads are for review and cannot substitute for this check.
    async fn memory_episodes(&self, ids: &[EpisodeId]) -> Result<Vec<Episode>, MemoryError> {
        if ids.len() > MAX_RECALL_RESULTS {
            return Err(MemoryError::InvalidInput("too many evidence reads".into()));
        }
        Err(MemoryError::Backend(
            "Scoped evidence loading is unavailable.".into(),
        ))
    }

    /// Cheap liveness/readiness probe.
    async fn health(&self) -> Result<(), MemoryError>;

    async fn memory_policy(&self) -> Result<MemoryPolicy, MemoryError>;
    /// Disabling cancels queued/leased work; re-enabling never revives it.
    async fn set_memory_enabled(&self, enabled: bool) -> Result<MemoryPolicy, MemoryError>;
    /// Atomically records evidence and its job; repeating a key must preserve the input.
    async fn enqueue_memory(&self, input: MemoryInput) -> Result<EpisodeId, MemoryError>;
    /// Claims at most one due job, recovering abandoned leases with bounded attempts.
    async fn claim_memory(&self, at: Timestamp) -> Result<Option<MemoryClaim>, MemoryError>;
    /// Persist grounded candidates before consolidation; a reclaimed lease resumes
    /// with this result instead of calling the extractor again.
    async fn save_memory_extraction(
        &self,
        claim: &MemoryClaim,
        extracted: &MemoryExtractedChunk,
        at: Timestamp,
    ) -> Result<(), MemoryError>;
    /// Saves the whole validated batch and completes its job in one transaction.
    async fn complete_memory(
        &self,
        claim: &MemoryClaim,
        candidates: Vec<MemoryCandidate>,
        model: &str,
        at: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError>;
    /// Retries with backoff, then retains a visible failed state after the attempt bound.
    async fn fail_memory(
        &self,
        claim: &MemoryClaim,
        message: &str,
        at: Timestamp,
    ) -> Result<(), MemoryError>;
    async fn memory_job(&self, episode: &EpisodeId) -> Result<MemoryJob, MemoryError>;

    /// Explicitly retries a bounded page of failed captures. Never
    /// revives cancelled work or repeats completed captures; paused policy rejects it.
    async fn retry_failed_memories(&self) -> Result<usize, MemoryError> {
        Err(MemoryError::Backend(
            "Retrying saved messages is unavailable.".into(),
        ))
    }

    /// An explicit user note and its fact commit together, even while automatic
    /// remembering is paused. It remains separate from model-generated claims.
    async fn remember_note(&self, _note: MemoryNote) -> Result<FactId, MemoryError> {
        Err(MemoryError::Backend(
            "Saving a personal detail is unavailable.".into(),
        ))
    }

    /// Appends an immutable episode.
    async fn record_episode(&self, episode: NewEpisode) -> Result<EpisodeId, MemoryError>;

    /// Read the immutable evidence referenced by a fact; absent IDs return `NotFound`.
    async fn episode(&self, id: &EpisodeId) -> Result<Episode, MemoryError>;

    /// Most recent episodes for a session, newest first.
    async fn recent_episodes(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError>;

    /// Saves a fact, links it to its entities and source episode, and — when `replaces`
    /// is set — invalidates the replaced fact and links the two.
    async fn save_fact(&self, fact: NewFact) -> Result<FactId, MemoryError>;

    /// Marks a fact no longer believed as of `at` (sets `invalid_at`/`expired_at`).
    /// Never deletes.
    async fn invalidate_fact(&self, id: &FactId, at: Timestamp) -> Result<(), MemoryError>;

    /// Retain the record for review, but revoke its use in current and historical recall.
    async fn stop_memory(&self, _id: &FactId, _at: Timestamp) -> Result<(), MemoryError> {
        Err(MemoryError::Backend(
            "Stopping saved details is unavailable.".into(),
        ))
    }

    /// Withdraw the complete correction history containing this detail, including
    /// its original and repeated sources, atomically. Retain review history and
    /// unrelated facts; never silently fall back to withdrawing only one version.
    async fn remove_memory(&self, _id: &FactId, _at: Timestamp) -> Result<(), MemoryError> {
        Err(MemoryError::Backend(
            "Removing a detail and its versions is unavailable.".into(),
        ))
    }

    /// Retrieves facts by semantic similarity, entity, and/or recency per [`RecallQuery`].
    async fn recall(&self, query: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError>;

    /// Bounded local review of retained evidence, independent of model recall.
    async fn list_memory_sources(
        &self,
        _query: MemorySourceQuery,
    ) -> Result<MemorySourcePage, MemoryError> {
        Err(MemoryError::Backend(
            "Source browsing is unavailable in this memory store.".into(),
        ))
    }

    /// Explicit local review, distinct from permission-checked model retrieval.
    async fn review_memory_source(
        &self,
        id: &EpisodeId,
    ) -> Result<MemorySourceRecord, MemoryError> {
        Ok(MemorySourceRecord {
            episode: self.episode(id).await?,
            metadata: MemorySourceMetadata::default(),
        })
    }

    /// Bounded alphabetical discovery of subjects in current saved details.
    async fn list_memory_subjects(
        &self,
        _query: MemorySubjectQuery,
    ) -> Result<MemorySubjectPage, MemoryError> {
        Err(MemoryError::Backend(
            "Subject browsing is unavailable in this memory store.".into(),
        ))
    }

    /// Bounded current/history browsing, independent of automatic remembering policy.
    async fn list_memories(&self, _query: MemoryListQuery) -> Result<MemoryPage, MemoryError> {
        Err(MemoryError::Backend(
            "Memory browsing is unavailable.".into(),
        ))
    }

    /// Exact fact lookup for source inspection and reviewing a retained version.
    async fn memory_fact(&self, _id: &FactId) -> Result<Fact, MemoryError> {
        Err(MemoryError::Backend(
            "Memory inspection is unavailable.".into(),
        ))
    }

    /// Explicit user action, permitted while automatic remembering is paused.
    /// The source note, replacement, and invalidation must commit together.
    async fn correct_memory(&self, _correction: MemoryCorrection) -> Result<FactId, MemoryError> {
        Err(MemoryError::Backend(
            "Memory correction is unavailable.".into(),
        ))
    }

    /// Links current details as earlier and later versions without hiding either.
    /// A second call for the same pair does not add another link.
    async fn keep_same_details(&self, _links: &[MemorySameDetail]) -> Result<(), MemoryError> {
        Err(MemoryError::Backend(
            "Linking repeated details is unavailable.".into(),
        ))
    }

    /// Upserts a conversation thread (including its messages and metadata) into the store.
    async fn save_chat_thread(
        &self,
        _thread: &crate::chat_history::ChatThread,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Lists non-deleted conversation threads ordered by `updated_at` descending.
    async fn list_chat_threads(
        &self,
        _limit: usize,
    ) -> Result<Vec<crate::chat_history::ChatThread>, MemoryError> {
        Ok(Vec::new())
    }

    /// Soft-deletes a conversation thread as of `at` (sets `deleted_at`; never hard-deletes).
    async fn delete_chat_thread(
        &self,
        _session_id: &str,
        _at: Timestamp,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// The saved team roster; `None` before the user's team was first saved.
    async fn load_team(&self) -> Result<Option<Vec<crate::teams::TeamMember>>, MemoryError> {
        Ok(None)
    }

    /// Replaces the saved team roster.
    async fn save_team(&self, _members: &[crate::teams::TeamMember]) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Non-deleted artifacts, newest first.
    async fn list_artifacts(&self) -> Result<Vec<crate::artifacts::Artifact>, MemoryError> {
        Ok(Vec::new())
    }

    /// Upserts an artifact (its versions travel with it).
    async fn save_artifact(
        &self,
        _artifact: &crate::artifacts::Artifact,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Soft-deletes an artifact as of `at`.
    async fn delete_artifact(&self, _id: &str, _at: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Loads persisted personal channel connections (such as `WhatsApp` Personal companion state).
    async fn load_connections(
        &self,
    ) -> Result<Option<crate::connections::ConnectionsState>, MemoryError> {
        Ok(None)
    }

    /// Saves personal channel connections state.
    async fn save_connections(
        &self,
        _state: &crate::connections::ConnectionsState,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    /// Loads sign-in accounts and the saved session. Password hashes are PHC
    /// strings (Argon2id); refresh tokens never pass through this port — they
    /// stay in the OS keychain behind the providers' credential store.
    async fn load_auth(&self) -> Result<Option<crate::auth::AuthRecords>, MemoryError> {
        Ok(None)
    }

    /// Saves sign-in accounts and the active session.
    async fn save_auth(&self, _records: &crate::auth::AuthRecords) -> Result<(), MemoryError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str) -> Fact {
        Fact {
            id: FactId(id.into()),
            content: format!("fact {id}"),
            entities: vec![],
            source: None,
            replaces: None,
            extraction: None,
            created_at: Timestamp(1_000),
            valid_at: None,
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        }
    }

    #[test]
    fn recall_query_constructors_set_sane_defaults() {
        let recent = RecallQuery::recent(5);
        assert_eq!(recent.limit, 5);
        assert!(recent.embedding.is_none() && recent.text.is_none());
        assert!(!recent.include_expired);

        let semantic = RecallQuery::semantic(
            Embedding {
                model: "test".into(),
                vector: vec![0.0; 3],
            },
            "query",
            7,
        );
        assert_eq!(
            semantic.embedding.as_ref().map(Embedding::dimension),
            Some(3)
        );
        assert_eq!(semantic.text.as_deref(), Some("query"));
        assert_eq!(semantic.limit, 7);
    }

    #[test]
    fn facts_track_bitemporal_validity() {
        assert!(fact("a").is_current());
        let invalidated = Fact {
            invalid_at: Some(Timestamp(2_000)),
            ..fact("a")
        };
        assert!(!invalidated.is_current());
        let expired = Fact {
            expired_at: Some(Timestamp(2_000)),
            ..fact("a")
        };
        assert!(!expired.is_current());
    }

    #[test]
    fn rrf_prefers_facts_ranked_in_both_legs() {
        // "b" is mid-ranked in both legs; "a" and "c" top one leg each.
        let vector_leg = vec![fact("a"), fact("b")];
        let text_leg = vec![fact("c"), fact("b")];
        let fused = fuse_rrf(vec![vector_leg, text_leg], 10);
        assert_eq!(
            fused[0].fact.id,
            FactId("b".into()),
            "two mid ranks beat one top rank"
        );
        assert_eq!(fused.len(), 3);
        let scores: Vec<f32> = fused.iter().map(|f| f.score).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "sorted best-first");
    }

    #[test]
    fn rrf_respects_limit_and_handles_empty_legs() {
        assert_eq!(fuse_rrf(vec![], 5), vec![]);
        assert_eq!(fuse_rrf(vec![vec![]], 5), vec![]);
        let fused = fuse_rrf(vec![vec![fact("a"), fact("b"), fact("c")]], 2);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].fact.id, FactId("a".into()));
    }

    #[test]
    fn rrf_uses_one_based_ranks_and_ignores_duplicate_hits_in_one_leg() {
        let unique = fuse_rrf(vec![vec![fact("a"), fact("b")]], 10);
        let repeated = fuse_rrf(vec![vec![fact("a"), fact("a"), fact("b")]], 10);
        assert_eq!(
            unique, repeated,
            "one retrieval path cannot vote twice for a fact"
        );
        assert!((unique[0].score - 1.0 / 61.0).abs() < f32::EPSILON);
    }

    #[test]
    fn memory_requests_reject_unbounded_or_invalid_input() {
        for vector in [vec![], vec![0.0; 3], vec![f32::NAN], vec![f32::INFINITY]] {
            assert!(
                Embedding {
                    model: "test".into(),
                    vector
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Embedding {
                model: "test".into(),
                vector: vec![0.0, 1.0]
            }
            .validate()
            .is_ok()
        );
        assert!(RecallQuery::recent(usize::MAX).validate().is_err());
        assert!(RecallQuery::recent(MAX_RECALL_RESULTS).validate().is_ok());
        assert!(RecallQuery::recent(0).validate().is_ok());
        let mut query = RecallQuery::recent(10);
        query.text = Some(" ".into());
        assert!(query.validate().is_err());
        query.text = Some("Lisbon".into());
        assert!(query.validate().is_ok());
        query.entity = Some("x".repeat(257));
        assert!(query.validate().is_err());

        let mut candidate = NewFact {
            content: "Maya lives in Lisbon".into(),
            entities: vec!["Maya".into()],
            embedding: None,
            source: None,
            valid_at: None,
            replaces: None,
        };
        assert!(candidate.validate().is_ok());
        candidate.content = "x".repeat(MAX_FACT_BYTES + 1);
        assert!(candidate.validate().is_err());
        candidate.content = "Maya lives in Lisbon".into();
        candidate.entities = vec![String::new()];
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn episodes_require_bounded_content_and_a_session() {
        let mut episode = NewEpisode {
            session_id: "chat-1".into(),
            kind: EpisodeKind::UserMessage,
            content: "I live in Lisbon".into(),
            occurred_at: Timestamp(1),
        };
        assert!(episode.validate().is_ok());
        episode.content = "x".repeat(MAX_EPISODE_BYTES + 1);
        assert!(episode.validate().is_err());
        episode.content = " ".into();
        assert!(episode.validate().is_err());
        episode.content = "hello".into();
        episode.session_id = "x".repeat(513);
        assert!(episode.validate().is_err());
        episode.session_id.clear();
        assert!(episode.validate().is_err());
    }
}
