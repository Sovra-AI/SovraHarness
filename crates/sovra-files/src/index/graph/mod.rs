//! Reviewed knowledge records share the index's access grants and source lineage.
//! Grant removal nulls evidence links permanently, including across re-grants.

mod read;
mod write;

// Include endpoint identity evidence, not only a relationship's own citations.
// Shared by traversal and cycle checking so revoked topology is never consulted.
const GRANTED_RELATION: &str = "NOT EXISTS (
    SELECT 1 FROM index_knowledge_evidence e WHERE e.id IN (
        SELECT evidence_id FROM index_knowledge_relation_sources WHERE relation_id=r.id
        UNION SELECT evidence_id FROM index_knowledge_entity_sources WHERE entity_id=r.subject OR entity_id=r.object
    ) AND (e.root_id IS NULL OR e.file_id IS NULL OR e.version_id IS NULL)
)";

use super::derived::{bounded, decode, json, number};
use super::{LocalFileIndex, db_error, now};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    IndexError, IndexPassage,
    graph::{
        DraftEndpoint, EntityKind, FileKnowledge, GraphDraft, GraphEndpoint, GraphEntity,
        GraphEvidence, GraphOrigin, GraphPage, GraphQuery, GraphRelation, GraphSource,
        KnowledgeProposal, ProposalState, RelationKind,
    },
};

fn missing() -> IndexError {
    IndexError(
        "This knowledge record is unavailable or a supporting folder grant was removed.".into(),
    )
}

fn entity(db: &Connection, id: &str, roots: &[String]) -> Result<GraphEntity, IndexError> {
    let record: String = db
        .query_row(
            "SELECT details FROM index_knowledge_entities WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db_error(&e))?
        .ok_or_else(missing)?;
    let entity: GraphEntity = decode(&record)?;
    for id in &entity.evidence {
        evidence(db, *id, roots)?;
    }
    Ok(entity)
}

fn evidence(db: &Connection, id: u64, roots: &[String]) -> Result<GraphEvidence, IndexError> {
    let value:Option<(String,String,bool)>=db.query_row("SELECT e.snapshot,e.root_id,coalesce(f.available=1 AND f.current_version=e.version_id,0) FROM index_knowledge_evidence e LEFT JOIN index_files f ON f.id=e.file_id WHERE e.id=?1 AND e.root_id IS NOT NULL AND e.file_id IS NOT NULL AND e.version_id IS NOT NULL",[number(id)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|db_error(&e))?;
    let (snapshot, root, current) = value.ok_or_else(missing)?;
    if !roots.is_empty() && !roots.contains(&root) {
        return Err(missing());
    }
    let passage: IndexPassage = decode(&snapshot)?;
    Ok(GraphEvidence {
        source: GraphSource {
            id,
            reference: passage.reference.clone(),
            path: passage.path.clone(),
            start_line: passage.start_line,
            end_line: passage.end_line,
            current,
        },
        passage,
    })
}

fn proposal(db: &Connection, id: u64) -> Result<KnowledgeProposal, IndexError> {
    let (details, state): (String, String) = db
        .query_row(
            "SELECT details,state FROM index_knowledge_proposals WHERE id=?1",
            [number(id)?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| db_error(&e))?
        .ok_or_else(missing)?;
    let mut proposal: KnowledgeProposal = decode(&details)?;
    proposal.state = decode(&state)?;
    for evidence in evidence_ids(db, id)? {
        self::evidence(db, evidence, &[])?;
    }
    for node in &proposal.existing_entities {
        entity(db, &node.id, &[])?;
    }
    for relation in &proposal.replaced_relations {
        read::relation(db, &relation.id, &[])?;
    }
    Ok(proposal)
}

fn evidence_ids(db: &Connection, proposal: u64) -> Result<Vec<u64>, IndexError> {
    let mut query = db
        .prepare("SELECT id FROM index_knowledge_evidence WHERE proposal_id=?1 ORDER BY ordinal")
        .map_err(|e| db_error(&e))?;
    query
        .query_map([number(proposal)?], |r| r.get::<_, i64>(0))
        .map_err(|e| db_error(&e))?
        .map(|r| u64::try_from(r.map_err(|e| db_error(&e))?).map_err(|_| missing()))
        .collect()
}

impl FileKnowledge for LocalFileIndex {
    fn propose(
        &self,
        draft: &GraphDraft,
        origin: &GraphOrigin,
    ) -> Result<KnowledgeProposal, IndexError> {
        draft.validate()?;
        origin.validate()?;
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let proposal = write::propose(&tx, draft, origin)?;
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(proposal)
    }
    fn pending(&self) -> Result<Vec<KnowledgeProposal>, IndexError> {
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let mut query=tx.prepare("SELECT id FROM index_knowledge_proposals WHERE state='\"pending\"' ORDER BY id DESC LIMIT 8").map_err(|e|db_error(&e))?;
        let ids = query
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?;
        ids.into_iter()
            .map(|id| proposal(&tx, u64::try_from(id).map_err(|_| missing())?))
            .collect()
    }
    fn decide(&self, id: u64, approve: bool) -> Result<KnowledgeProposal, IndexError> {
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let result = write::decide(&tx, id, approve)?;
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(result)
    }
    fn query(&self, query: &GraphQuery) -> Result<GraphPage, IndexError> {
        query.validate()?;
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        read::query(&tx, query)
    }
    fn evidence(&self, id: u64) -> Result<GraphEvidence, IndexError> {
        evidence(&self.connect()?, id, &[])
    }
}
