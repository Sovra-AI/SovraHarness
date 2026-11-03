//! Versioned, idempotent schema. Applied on every open.
//!
//! Every statement uses `OVERWRITE` so re-applying converges on the current definition
//! (`IF NOT EXISTS` silently keeps a stale definition — never use it here). Tables are
//! SCHEMALESS because memory records evolve as we iterate; constraints we rely on
//! (uniqueness, indexes) are explicit. Data-shape changes bump `SCHEMA_VERSION` and get a
//! migration arm in [`super::SurrealStore::open`].

/// Bump when stored data (not just indexes) needs migrating.
pub(crate) const SCHEMA_VERSION: i64 = 12;

/// Statements that don't depend on configuration.
pub(crate) const STATIC_SCHEMA: &str = "
DEFINE TABLE OVERWRITE memory_job SCHEMALESS;
DEFINE INDEX OVERWRITE memory_job_due ON memory_job FIELDS state, retry_at;
DEFINE INDEX OVERWRITE memory_job_episode ON memory_job FIELDS episode;
DEFINE INDEX OVERWRITE memory_job_conversation ON memory_job FIELDS session_id, state, created_at;
DEFINE INDEX OVERWRITE memory_job_lease ON memory_job FIELDS state, lease_until;

DEFINE TABLE OVERWRITE episode SCHEMALESS;
DEFINE INDEX OVERWRITE episode_session ON episode FIELDS session_id, occurred_at;
DEFINE INDEX OVERWRITE episode_source_family ON episode FIELDS source_family, source_version;

DEFINE TABLE OVERWRITE chat_thread SCHEMALESS;
DEFINE INDEX OVERWRITE chat_thread_session ON chat_thread FIELDS session_id UNIQUE;
DEFINE INDEX OVERWRITE chat_thread_updated ON chat_thread FIELDS updated_at;

DEFINE TABLE OVERWRITE team SCHEMALESS;

DEFINE TABLE OVERWRITE artifact SCHEMALESS;
DEFINE INDEX OVERWRITE artifact_updated ON artifact FIELDS updated_at;

DEFINE TABLE OVERWRITE fact SCHEMALESS;
DEFINE INDEX OVERWRITE fact_created ON fact FIELDS created_at;
DEFINE INDEX OVERWRITE fact_content ON fact FIELDS content;

DEFINE TABLE OVERWRITE memory_alias SCHEMALESS;
DEFINE INDEX OVERWRITE memory_alias_content ON memory_alias FIELDS content;

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

pub(crate) fn semantic_table_name(generation: u64) -> String {
    format!("memory_vectors_{generation}")
}

pub(crate) fn semantic_table(generation: u64) -> String {
    let table = semantic_table_name(generation);
    format!(
        "DEFINE TABLE OVERWRITE {table} SCHEMALESS; DEFINE INDEX OVERWRITE memory_vector_fact ON {table} FIELDS fact UNIQUE;"
    )
}

pub(crate) fn semantic_index(generation: u64, dimension: usize) -> String {
    let table = semantic_table_name(generation);
    format!(
        "DEFINE INDEX OVERWRITE memory_vector_knn ON {table} FIELDS vector HNSW DIMENSION {dimension} DIST COSINE EFC 150 M 12;"
    )
}

/// Before v5 an explicit stop was an expiration without a successor. Preserve
/// those withdrawals conservatively; correction history still has its successor.
pub(crate) const MIGRATE_WITHDRAWALS: &str = "UPDATE fact SET stopped_at = expired_at WHERE expired_at IS NOT NONE AND stopped_at IS NONE AND array::len(<-supersedes<-fact) = 0;";

/// Existing IDs, completed receipts and provenance remain unchanged. Old pending
/// jobs start at the beginning; the origin is explicitly unknown for old evidence.
pub(crate) const MIGRATE_SOURCES: &str = "
UPDATE memory_job SET cursor = 0, session_id = episode.session_id WHERE cursor IS NONE;
UPDATE episode SET origin = 'legacy_unknown', permission_scope = 'unknown', source_version = 1 WHERE origin IS NONE;
";

/// Existing record IDs are known identities; authorship and admission time remain unknown.
pub(crate) const MIGRATE_SOURCE_IDENTITIES: &str =
    "UPDATE episode SET source_family = source_family ?? id;";

/// Earlier helper replies have no complete record of their model inputs.
pub(crate) const MIGRATE_REPLY_DEPENDENCIES: &str =
    "UPDATE episode SET dependency_complete = false WHERE dependency_complete IS NONE;";
