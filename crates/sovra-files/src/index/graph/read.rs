use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn relation(
    db: &Connection,
    id: &str,
    roots: &[String],
) -> Result<GraphRelation, IndexError> {
    let (details, expired): (String, Option<i64>) = db
        .query_row(
            "SELECT details,expired FROM index_knowledge_relations WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| db_error(&e))?
        .ok_or_else(missing)?;
    let mut relation: GraphRelation = decode(&details)?;
    relation.expired_at = expired
        .map(u64::try_from)
        .transpose()
        .map_err(|_| missing())?;
    relation.sources_current = true;
    for source in &relation.evidence {
        relation.sources_current &= evidence(db, *source, roots)?.source.current;
    }
    for endpoint in [&relation.subject, &relation.object] {
        match endpoint {
            GraphEndpoint::Entity(id) => {
                entity(db, id, roots)?;
            }
            GraphEndpoint::Source(id) => {
                evidence(db, *id, roots)?;
            }
        }
    }
    Ok(relation)
}

pub(super) fn query(db: &Connection, query: &GraphQuery) -> Result<GraphPage, IndexError> {
    for root in &query.roots {
        super::super::get_root(db, root)?;
    }
    let mut frontier = Vec::new();
    if let Some(id) = &query.entity_id {
        entity(db, id, &query.roots)?;
        frontier.push(id.clone());
    }
    let mut visited = BTreeSet::new();
    let mut nodes = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut relations = vec![];
    let mut truncated = false;
    let mut boundary = None;
    let mut inspected = 0;
    let started = std::time::Instant::now();
    let recorded = query.recorded_at.unwrap_or_else(now);
    'walk: for hop in 0..query.hops {
        let text = if hop == 0 {
            query.text.as_deref().unwrap_or("")
        } else {
            ""
        };
        let sql=format!("SELECT r.id,r.created FROM index_knowledge_relations r
            LEFT JOIN index_knowledge_entities s ON s.id=r.subject
            LEFT JOIN index_knowledge_entities o ON o.id=r.object
            WHERE r.created<=?1 AND (?2 OR r.expired IS NULL OR r.expired>?1)
            AND (?3='[]' OR r.subject IN (SELECT value FROM json_each(?3)) OR r.object IN (SELECT value FROM json_each(?3)))
            AND (?4='' OR instr(lower(json_extract(r.details,'$.statement')),lower(?4))>0 OR instr(lower(json_extract(s.details,'$.label')),lower(?4))>0 OR instr(lower(json_extract(o.details,'$.label')),lower(?4))>0)
            AND {GRANTED_RELATION}
            AND (?6 IS NULL OR r.created<?6 OR (r.created=?6 AND r.id>?7))
            AND NOT EXISTS (SELECT 1 FROM index_knowledge_evidence e WHERE e.id IN (
                SELECT evidence_id FROM index_knowledge_relation_sources WHERE relation_id=r.id
                UNION SELECT evidence_id FROM index_knowledge_entity_sources WHERE entity_id=r.subject OR entity_id=r.object
            ) AND ?5!='[]' AND e.root_id NOT IN (SELECT value FROM json_each(?5)))
            ORDER BY r.created DESC,r.id LIMIT 1001");
        let ids = db
            .prepare(&sql)
            .map_err(|e| db_error(&e))?
            .query_map(
                params![
                    number(recorded)?,
                    query.include_superseded,
                    json(&frontier)?,
                    text,
                    json(&query.roots)?,
                    query
                        .before
                        .as_ref()
                        .map(|c| number(c.recorded_at))
                        .transpose()?,
                    query.before.as_ref().map(|c| c.id.as_str())
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?;
        if ids.len() > 1000 {
            truncated = true;
        }
        let mut next = BTreeSet::new();
        for (id, recorded_at) in ids.into_iter().take(1000) {
            if !visited.insert(id.clone()) {
                continue;
            }
            inspected += 1;
            if inspected > 1000 {
                truncated = true;
                break 'walk;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                return Err(IndexError(
                    "Knowledge traversal took too long. Restrict the folders or entity.".into(),
                ));
            }
            let record = relation(db, &id, &query.roots)?;
            let cursor = sovra_core::file_index::graph::GraphCursor {
                recorded_at: u64::try_from(recorded_at).map_err(|_| missing())?,
                id,
            };
            if !query.include_stale && !record.sources_current {
                boundary = Some(cursor);
                continue;
            }
            let mut temporal = record.clone();
            if query.include_superseded {
                temporal.expired_at = None;
            }
            if !temporal.visible_at(recorded, query.valid_at, query.include_unknown_validity) {
                boundary = Some(cursor);
                continue;
            }
            if relations.len() >= query.limit {
                truncated = true;
                break 'walk;
            }
            for endpoint in [&record.subject, &record.object] {
                match endpoint {
                    GraphEndpoint::Entity(id) => {
                        let node = entity(db, id, &query.roots)?;
                        for id in &node.evidence {
                            sources.insert(*id, evidence(db, *id, &query.roots)?.source);
                        }
                        next.insert(id.clone());
                        nodes.insert(id.clone(), node);
                    }
                    GraphEndpoint::Source(id) => {
                        sources.insert(*id, evidence(db, *id, &query.roots)?.source);
                    }
                }
            }
            for id in &record.evidence {
                sources.insert(*id, evidence(db, *id, &query.roots)?.source);
            }
            relations.push(record);
            boundary = Some(cursor);
        }
        if next.is_empty() {
            break;
        }
        frontier = next.into_iter().collect();
    }
    Ok(GraphPage {
        entities: nodes.into_values().collect(),
        relations,
        sources: sources.into_values().collect(),
        truncated,
        next: if truncated && query.hops == 1 {
            boundary
        } else {
            None
        },
    })
}
