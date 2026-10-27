//! Versioned, idempotent schema. Applied on every open.
//!
//! Every statement uses `OVERWRITE` so re-applying converges on the current definition
//! (`IF NOT EXISTS` silently keeps a stale definition — never use it here). Tables are
//! SCHEMALESS because memory records evolve as we iterate; constraints we rely on
//! (uniqueness, indexes) are explicit. Data-shape changes bump `SCHEMA_VERSION` and get a
//! migration arm in [`super::SurrealStore::open`].

/// Bump when stored data (not just indexes) needs migrating.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// Statements that don't depend on configuration.
pub(crate) const STATIC_SCHEMA: &str = "
DEFINE TABLE OVERWRITE episode SCHEMALESS;
DEFINE INDEX OVERWRITE episode_session ON episode FIELDS session_id, occurred_at;

DEFINE TABLE OVERWRITE chat_thread SCHEMALESS;
DEFINE INDEX OVERWRITE chat_thread_session ON chat_thread FIELDS session_id UNIQUE;
DEFINE INDEX OVERWRITE chat_thread_updated ON chat_thread FIELDS updated_at;

DEFINE TABLE OVERWRITE team SCHEMALESS;

DEFINE TABLE OVERWRITE artifact SCHEMALESS;
DEFINE INDEX OVERWRITE artifact_updated ON artifact FIELDS updated_at;

DEFINE TABLE OVERWRITE fact SCHEMALESS;
DEFINE INDEX OVERWRITE fact_created ON fact FIELDS created_at;

DEFINE TABLE OVERWRITE entity SCHEMALESS;
DEFINE INDEX OVERWRITE entity_name ON entity FIELDS name UNIQUE;

DEFINE TABLE OVERWRITE mentions TYPE RELATION IN fact OUT entity;
DEFINE TABLE OVERWRITE derived_from TYPE RELATION IN fact OUT episode;
DEFINE TABLE OVERWRITE supersedes TYPE RELATION IN fact OUT fact;

DEFINE ANALYZER OVERWRITE memory_text TOKENIZERS blank,class FILTERS lowercase,snowball(english);
DEFINE INDEX OVERWRITE fact_text ON fact FIELDS content FULLTEXT ANALYZER memory_text BM25;
DEFINE INDEX OVERWRITE episode_text ON episode FIELDS content FULLTEXT ANALYZER memory_text BM25;
";

/// The HNSW index depends on the configured embedding dimension, so it is rendered.
/// Facts without an embedding simply aren't in the index.
pub(crate) fn hnsw_index(dimension: usize) -> String {
    format!(
        "DEFINE INDEX OVERWRITE fact_embedding ON fact FIELDS embedding \
         HNSW DIMENSION {dimension} DIST COSINE EFC 150 M 12;"
    )
}
