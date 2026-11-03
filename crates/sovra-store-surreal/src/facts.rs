//! Shared atomic fact writes for direct corrections and ingestion batches.

use sovra_core::{MemoryError, MemoryExtraction, NewFact, Timestamp};
use surrealdb::types::{RecordId, SurrealValue};

use super::parse_id;

pub(super) const FIELDS: &str = "id, content, entities, created_at, valid_at, invalid_at, expired_at, stopped_at, evidence, extractor, \
    (->derived_from->episode.id)[0] AS source, (->supersedes->fact.id)[0] AS replaces";

#[derive(Debug, SurrealValue)]
pub(super) struct FactWrite {
    pub(super) id: Option<RecordId>,
    pub(super) same_as: Option<RecordId>,
    pub(super) alias_key: Option<String>,
    match_valid_at: Option<i64>,
    data: NewFactRow,
    entities: Vec<EntityLink>,
    source: Option<RecordId>,
    replaces: Option<RecordId>,
}

#[derive(Debug, SurrealValue)]
struct EntityLink {
    id: RecordId,
    name: String,
}

#[derive(Debug, SurrealValue)]
struct NewFactRow {
    content: String,
    entities: Vec<String>,
    embedding: Option<Vec<f32>>,
    embed_model: Option<String>,
    created_at: i64,
    valid_at: Option<i64>,
    evidence: Option<String>,
    extractor: Option<String>,
}

pub(super) fn prepare(
    fact: NewFact,
    now: i64,
    extraction: Option<MemoryExtraction>,
) -> Result<FactWrite, MemoryError> {
    fact.validate()?;
    let source = fact
        .source
        .as_ref()
        .map(|id| parse_id(&id.0, "episode"))
        .transpose()?;
    let replaces = fact
        .replaces
        .as_ref()
        .map(|id| parse_id(&id.0, "fact"))
        .transpose()?;
    let mut seen = std::collections::HashSet::new();
    let entities = fact
        .entities
        .iter()
        .filter(|name| seen.insert((*name).clone()))
        .map(|name| EntityLink {
            id: RecordId::new("entity", format!("name:{name}")),
            name: name.clone(),
        })
        .collect();
    Ok(FactWrite {
        id: None,
        same_as: None,
        alias_key: None,
        match_valid_at: fact.valid_at.map(Timestamp::millis),
        source,
        replaces,
        entities,
        data: NewFactRow {
            content: fact.content,
            entities: fact.entities,
            embedding: fact.embedding.as_ref().map(|e| e.vector.clone()),
            embed_model: fact.embedding.map(|e| e.model),
            created_at: now,
            valid_at: Some(fact.valid_at.map_or(now, Timestamp::millis)),
            evidence: extraction.as_ref().map(|e| e.evidence.clone()),
            extractor: extraction.map(|e| e.model),
        },
    })
}

/// Runs inside the caller's transaction with `$write` bound; leaves `$created`.
/// Ingestion and human corrections must share the same graph and validity rules.
pub(super) const WRITE: &str = "
UPDATE meta:memory_facts SET revision += 1;
LET $data = $write.data;
IF $data.embed_model != NONE {
    LET $indexed = (SELECT VALUE model FROM ONLY meta:embedding);
    IF $indexed != NONE AND $indexed != $data.embed_model { THROW 'incompatible embedding model'; };
    IF $indexed = NONE { UPDATE meta:embedding SET model = $data.embed_model; };
};
LET $target = $write.id ?? type::record('fact', rand::ulid());
LET $created = (CREATE ONLY $target CONTENT $data RETURN VALUE id);
FOR $entity IN $write.entities {
    LET $existing = (SELECT VALUE id FROM entity WHERE name = $entity.name LIMIT 1)[0];
    LET $target = $existing ?? $entity.id;
    UPSERT $target SET name = $entity.name;
    RELATE $created->mentions->$target;
};
IF $write.source != NONE {
    IF !record::exists($write.source) { THROW 'memory source no longer exists'; };
    RELATE $created->derived_from->($write.source);
};
IF $write.replaces != NONE {
    LET $changed = (UPDATE $write.replaces SET invalid_at = $data.valid_at, expired_at = $data.created_at
        WHERE stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE RETURN VALUE id);
    IF array::len($changed) != 1 { THROW 'memory correction is stale or missing'; };
    RELATE $created->supersedes->($write.replaces);
};
";
