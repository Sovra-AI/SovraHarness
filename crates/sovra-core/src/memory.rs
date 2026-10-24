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
}

/// An episode to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEpisode {
    pub session_id: String,
    pub kind: EpisodeKind,
    pub content: String,
    pub occurred_at: Timestamp,
}

/// A stored episode.
#[derive(Debug, Clone, PartialEq)]
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

/// A stored fact with bi-temporal validity.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub id: FactId,
    pub content: String,
    pub entities: Vec<String>,
    /// Ingestion time (always set by the store).
    pub created_at: Timestamp,
    /// When the fact became true in the world, when known.
    pub valid_at: Option<Timestamp>,
    /// When the fact stopped being true in the world.
    pub invalid_at: Option<Timestamp>,
    /// When the system superseded it (never deleted).
    pub expired_at: Option<Timestamp>,
}

impl Fact {
    /// A fact the agent should still believe.
    pub fn is_current(&self) -> bool {
        self.invalid_at.is_none() && self.expired_at.is_none()
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
}

impl RecallQuery {
    /// Recency-ordered recall of current facts.
    pub fn recent(limit: usize) -> Self {
        Self {
            embedding: None,
            text: None,
            entity: None,
            limit,
            include_expired: false,
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
    let mut order: Vec<FactId> = Vec::new();
    let mut fused: std::collections::HashMap<FactId, RecalledFact> =
        std::collections::HashMap::new();
    for leg in legs {
        for (rank, fact) in leg.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "ranks are tiny")]
            let contribution = 1.0 / (RRF_K + rank as f32);
            if let Some(existing) = fused.get_mut(&fact.id) {
                existing.score += contribution;
            } else {
                order.push(fact.id.clone());
                fused.insert(
                    fact.id.clone(),
                    RecalledFact {
                        fact,
                        score: contribution,
                    },
                );
            }
        }
    }
    let mut results: Vec<RecalledFact> = order
        .into_iter()
        .filter_map(|id| fused.remove(&id))
        .collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

/// Backend-neutral storage errors. Adapters map their native errors into these,
/// keeping backend types out of every caller.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
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
    /// Cheap liveness/readiness probe.
    async fn health(&self) -> Result<(), MemoryError>;

    /// Appends an immutable episode.
    async fn record_episode(&self, episode: NewEpisode) -> Result<EpisodeId, MemoryError>;

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

    /// Retrieves facts by semantic similarity, entity, and/or recency per [`RecallQuery`].
    async fn recall(&self, query: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError>;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str) -> Fact {
        Fact {
            id: FactId(id.into()),
            content: format!("fact {id}"),
            entities: vec![],
            created_at: Timestamp(1_000),
            valid_at: None,
            invalid_at: None,
            expired_at: None,
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
}
