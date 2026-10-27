use super::*;
use sovra_core::file_index::{EvidenceBasis, EvidenceTier};
use std::collections::BTreeMap;

pub(super) fn propose(
    db: &Connection,
    draft: &GraphDraft,
    origin: &GraphOrigin,
) -> Result<KnowledgeProposal, IndexError> {
    let fingerprint = super::super::digest(json(&(draft, origin))?.as_bytes());
    if let Some(id) = db
        .query_row(
            "SELECT id FROM index_knowledge_proposals WHERE fingerprint=?1",
            [&fingerprint],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| db_error(&e))?
    {
        return proposal(db, u64::try_from(id).map_err(|_| missing())?);
    }
    let (pending, total): (i64, i64) = db
        .query_row(
            "SELECT coalesce(sum(state='\"pending\"'),0),count(*) FROM index_knowledge_proposals",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| db_error(&e))?;
    if pending >= 8 || total >= 10_000 {
        return Err(IndexError("Review or reject waiting proposals before creating more. Knowledge retains at most 10,000 proposal records.".into()));
    }
    let mut sources = vec![];
    for reference in &draft.evidence {
        sources.push(db.query_row(&format!("{} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4 AND f.available=1 AND f.current_version=v.id",super::super::PASSAGES),params![reference.root_id,reference.file_id,reference.version_id,reference.ordinal],super::super::passage).optional().map_err(|e|db_error(&e))?.ok_or_else(||IndexError("A cited passage changed or is no longer granted. Search current sources before proposing knowledge.".into()))?);
    }
    let mut existing = BTreeMap::new();
    for relation in &draft.relations {
        for endpoint in [&relation.subject, &relation.object] {
            if let DraftEndpoint::Entity(id) = endpoint {
                existing.insert(id.clone(), entity(db, id, &[])?);
            }
        }
    }
    let replaced_relations = draft
        .relations
        .iter()
        .filter_map(|r| r.replaces.as_ref())
        .map(|id| read::relation(db, id, &[]))
        .collect::<Result<Vec<_>, _>>()?;
    if replaced_relations.iter().any(|r| r.expired_at.is_some()) {
        return Err(IndexError(
            "A relationship to replace was already superseded. Query the current knowledge first."
                .into(),
        ));
    }
    let mut required_roots: std::collections::BTreeSet<String> = sources
        .iter()
        .map(|p| p.reference.root_id.clone())
        .collect();
    let mut dependencies = vec![];
    for node in existing.values() {
        dependencies.extend_from_slice(&node.evidence);
    }
    for relation in &replaced_relations {
        dependencies.extend_from_slice(&relation.evidence);
        for endpoint in [&relation.subject, &relation.object] {
            if let GraphEndpoint::Entity(id) = endpoint {
                let node = entity(db, id, &[])?;
                dependencies.extend_from_slice(&node.evidence);
                existing.entry(id.clone()).or_insert(node);
            }
        }
    }
    for id in dependencies {
        required_roots.insert(evidence(db, id, &[])?.source.reference.root_id);
    }
    let created_at = now();
    let mut result = KnowledgeProposal {
        id: 0,
        state: ProposalState::Pending,
        created_at,
        reviewed_at: None,
        origin: origin.clone(),
        draft: draft.clone(),
        sources,
        existing_entities: existing.into_values().collect(),
        replaced_relations,
        required_roots: required_roots.into_iter().collect(),
    };
    validate_kinds(&result)?;
    db.execute("INSERT INTO index_knowledge_proposals(fingerprint,state,created,details) VALUES(?1,?2,?3,'')",params![fingerprint,json(&result.state)?,number(created_at)?]).map_err(|e|db_error(&e))?;
    result.id = u64::try_from(db.last_insert_rowid()).map_err(|_| missing())?;
    db.execute(
        "UPDATE index_knowledge_proposals SET details=?2 WHERE id=?1",
        params![number(result.id)?, json(&result)?],
    )
    .map_err(|e| db_error(&e))?;
    for (ordinal, passage) in result.sources.iter().enumerate() {
        let reference = &passage.reference;
        db.execute("INSERT INTO index_knowledge_evidence(proposal_id,ordinal,root_id,file_id,version_id,snapshot) VALUES(?1,?2,?3,?4,?5,?6)",params![number(result.id)?,u32::try_from(ordinal).expect("bounded evidence ordinal"),reference.root_id,reference.file_id,reference.version_id,json(passage)?]).map_err(|e|db_error(&e))?;
    }
    Ok(result)
}

fn kind(
    proposal: &KnowledgeProposal,
    endpoint: &DraftEndpoint,
) -> Result<Option<EntityKind>, IndexError> {
    match endpoint {
        DraftEndpoint::New(id) => proposal
            .draft
            .entities
            .iter()
            .find(|e| e.local_id == *id)
            .map(|e| Some(e.kind))
            .ok_or_else(missing),
        DraftEndpoint::Entity(id) => proposal
            .existing_entities
            .iter()
            .find(|e| e.id == *id)
            .map(|e| Some(e.kind))
            .ok_or_else(missing),
        DraftEndpoint::Source(_) => Ok(None),
    }
}
fn validate_kinds(proposal: &KnowledgeProposal) -> Result<(), IndexError> {
    for relation in &proposal.draft.relations {
        if !relation.relation.accepts(
            kind(proposal, &relation.subject)?,
            kind(proposal, &relation.object)?,
        ) {
            return Err(IndexError("A relationship uses incompatible entity types. Parts, projects and source representations have distinct roles.".into()));
        }
    }
    Ok(())
}

pub(super) fn decide(
    db: &Connection,
    id: u64,
    approve: bool,
) -> Result<KnowledgeProposal, IndexError> {
    let mut proposed = proposal(db, id)?;
    if proposed.state != ProposalState::Pending {
        return Err(IndexError("This proposal was already reviewed.".into()));
    }
    let next = if approve {
        ProposalState::Accepted
    } else {
        ProposalState::Rejected
    };
    if approve {
        proposed.draft.validate()?;
        validate_kinds(&proposed)?;
        let evidence_ids = evidence_ids(db, id)?;
        for id in &evidence_ids {
            if !evidence(db, *id, &[])?.source.current {
                return Err(IndexError(
                    "A source changed after this preview. Ask for a new proposal before saving."
                        .into(),
                ));
            }
        }
        accept(db, &proposed, &evidence_ids)?;
    }
    proposed.state = next;
    proposed.reviewed_at = Some(now());
    db.execute(
        "UPDATE index_knowledge_proposals SET state=?2,details=?3 WHERE id=?1",
        params![number(id)?, json(&next)?, json(&proposed)?],
    )
    .map_err(|e| db_error(&e))?;
    Ok(proposed)
}

fn accept(
    db: &Connection,
    proposal: &KnowledgeProposal,
    sources: &[u64],
) -> Result<(), IndexError> {
    let at = now();
    // Close every replaced edge before checking the new hierarchy. Otherwise a
    // valid reparenting can look cyclic solely because of proposal ordering.
    for draft in &proposal.draft.relations {
        if let Some(replaces) = &draft.replaces {
            let prior = read::relation(db, replaces, &[])?;
            if prior.expired_at.is_some() {
                return Err(IndexError(
                    "The relationship to replace has already changed. Review a new proposal."
                        .into(),
                ));
            }
            db.execute(
                "UPDATE index_knowledge_relations SET expired=?2 WHERE id=?1 AND expired IS NULL",
                params![replaces, number(at)?],
            )
            .map_err(|e| db_error(&e))?;
        }
    }
    let mut entities = BTreeMap::new();
    for (index, source) in proposal.draft.entities.iter().enumerate() {
        let id = format!("entity-{}-{index}", proposal.id);
        let entity = GraphEntity {
            id: id.clone(),
            kind: source.kind,
            label: source.label.clone(),
            evidence: source
                .evidence
                .iter()
                .map(|i| sources[usize::from(*i)])
                .collect(),
            created_at: at,
        };
        db.execute(
            "INSERT INTO index_knowledge_entities(id,details) VALUES(?1,?2)",
            params![id, json(&entity)?],
        )
        .map_err(|e| db_error(&e))?;
        for evidence in &entity.evidence {
            db.execute(
                "INSERT INTO index_knowledge_entity_sources(entity_id,evidence_id) VALUES(?1,?2)",
                params![id, number(*evidence)?],
            )
            .map_err(|e| db_error(&e))?;
        }
        entities.insert(source.local_id.clone(), id);
    }
    let resolve = |endpoint: &DraftEndpoint| -> Result<GraphEndpoint, IndexError> {
        match endpoint {
            DraftEndpoint::New(id) => Ok(GraphEndpoint::Entity(
                entities.get(id).ok_or_else(missing)?.clone(),
            )),
            DraftEndpoint::Entity(id) => {
                entity(db, id, &[])?;
                Ok(GraphEndpoint::Entity(id.clone()))
            }
            DraftEndpoint::Source(index) => Ok(GraphEndpoint::Source(sources[usize::from(*index)])),
        }
    };
    for (index, draft) in proposal.draft.relations.iter().enumerate() {
        let record = GraphRelation {
            id: format!("relation-{}-{index}", proposal.id),
            subject: resolve(&draft.subject)?,
            relation: draft.relation,
            object: resolve(&draft.object)?,
            statement: draft.statement.clone(),
            evidence: draft
                .evidence
                .iter()
                .map(|i| sources[usize::from(*i)])
                .collect(),
            basis: EvidenceBasis {
                tier: EvidenceTier::ModelInference,
                confidence: draft.confidence,
                activity_id: Some(proposal.id),
                recorded_at: at,
                observed_at: None,
                valid_time: draft.valid_time.clone(),
            },
            proposal_id: proposal.id,
            origin: Some(proposal.origin.clone()),
            replaces: draft.replaces.clone(),
            expired_at: None,
            sources_current: true,
        };
        let subject = entity_id(&record.subject);
        let object = entity_id(&record.object);
        if record.relation == RelationKind::PartOf {
            let cycle_sql = format!(
                "WITH RECURSIVE parents(id) AS (SELECT ?1 UNION SELECT r.object FROM index_knowledge_relations r JOIN parents p ON r.subject=p.id WHERE r.kind='\"part_of\"' AND r.expired IS NULL AND r.object IS NOT NULL AND {GRANTED_RELATION}) SELECT EXISTS(SELECT 1 FROM parents WHERE id=?2)"
            );
            let cycle: bool = db
                .query_row(&cycle_sql, params![object, subject], |r| r.get(0))
                .map_err(|e| db_error(&e))?;
            if cycle {
                return Err(IndexError(
                    "Part-of relationships must not form a cycle.".into(),
                ));
            }
        }
        db.execute("INSERT INTO index_knowledge_relations(id,proposal_id,subject,object,kind,created,details) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![record.id,number(proposal.id)?,subject,object,json(&record.relation)?,number(at)?,json(&record)?]).map_err(|e|db_error(&e))?;
        for source in &record.evidence {
            db.execute("INSERT INTO index_knowledge_relation_sources(relation_id,evidence_id) VALUES(?1,?2)",params![record.id,number(*source)?]).map_err(|e|db_error(&e))?;
        }
    }
    Ok(())
}

fn entity_id(endpoint: &GraphEndpoint) -> Option<&str> {
    match endpoint {
        GraphEndpoint::Entity(id) => Some(id),
        GraphEndpoint::Source(_) => None,
    }
}
