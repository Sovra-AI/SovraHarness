use super::{
    Connection, IndexError, SummaryHit, SummaryPage, SummaryQuery, SummaryRecord, configuration,
    db_error, decode, get_root, json, number, params, unavailable, unsigned,
};
use rusqlite::OptionalExtension as _;

// Grants stay revoked after re-adding a folder: the old foreign keys remain NULL.
const GRANTED: &str = "NOT EXISTS (
    SELECT 1 FROM index_summary_scopes g WHERE g.configuration_id=s.configuration_id AND g.root_id IS NULL
) AND EXISTS (SELECT 1 FROM index_summary_sources e WHERE e.summary_id=s.id)
AND NOT EXISTS (
    SELECT 1 FROM index_summary_sources e WHERE e.summary_id=s.id
    AND (e.root_id IS NULL OR e.file_id IS NULL OR e.version_id IS NULL)
)";

const CURRENT: &str = "s.invalidated IS NULL AND NOT EXISTS (
    SELECT 1 FROM index_summary_sources e LEFT JOIN index_files f ON f.id=e.file_id
    WHERE e.summary_id=s.id AND (f.id IS NULL OR f.available!=1 OR f.current_version IS NOT e.version_id)
)";

struct StoredSummary {
    details: String,
    invalidated_at: Option<i64>,
    reason: Option<String>,
    current: bool,
    configuration_id: u64,
}

pub(super) fn record(
    db: &Connection,
    id: u64,
    include_stale: bool,
) -> Result<SummaryRecord, IndexError> {
    let row = db.query_row(
        &format!("SELECT s.details,s.invalidated,s.reason,({CURRENT}),s.configuration_id FROM index_summaries s WHERE s.id=?1 AND {GRANTED}"),
        [number(id)?], |r| Ok(StoredSummary { details:r.get(0)?,invalidated_at:r.get(1)?,reason:r.get(2)?,current:r.get(3)?,configuration_id:unsigned(r, 4)? }),
    ).optional().map_err(|e| db_error(&e))?;
    let StoredSummary {
        details,
        invalidated_at,
        reason,
        current,
        configuration_id,
    } = row.ok_or_else(unavailable)?;
    let invalidated_at = invalidated_at
        .map(|at| u64::try_from(at).map_err(|_| unavailable()))
        .transpose()?;
    let configuration = configuration(db, configuration_id)?;
    let mut record: SummaryRecord = decode(&details)?;
    record.content.validate(record.inputs.len())?;
    if record.id != id
        || record.configuration_id != configuration_id
        || record.model != configuration.plan.summarizer
        || invalidated_at.is_some() != reason.is_some()
        || (!current && invalidated_at.is_none())
    {
        return Err(IndexError(
            "Stored summary provenance is inconsistent. Rebuild this map from its sources.".into(),
        ));
    }
    if !current && !include_stale {
        return Err(IndexError("This summary is stale. Read its history explicitly or rebuild it from current sources.".into()));
    }
    record.invalidated_at = invalidated_at;
    record.invalidation = reason.map(|reason| decode(&reason)).transpose()?;
    Ok(record)
}

pub(super) fn query(db: &Connection, query: &SummaryQuery) -> Result<SummaryPage, IndexError> {
    for root in &query.roots {
        get_root(db, root)?;
    }
    for id in &query.configuration_ids {
        configuration(db, *id)?;
    }
    let mut statement = db
        .prepare(&format!(
            "SELECT s.id FROM index_summaries s WHERE {GRANTED}
        AND (?1 OR ({CURRENT})) AND s.level BETWEEN ?2 AND ?3
        AND (?4 IS NULL OR s.id < ?4)
        AND (json_array_length(?5)=0 OR s.configuration_id IN (SELECT value FROM json_each(?5)))
        AND (json_array_length(?6)=0 OR NOT EXISTS (
            SELECT 1 FROM index_summary_sources e WHERE e.summary_id=s.id
            AND e.root_id NOT IN (SELECT value FROM json_each(?6))))
        AND (?7 IS NULL OR instr(s.search_text,?7)>0)
        ORDER BY s.id DESC LIMIT ?8"
        ))
        .map_err(|e| db_error(&e))?;
    let ids = statement
        .query_map(
            params![
                query.include_stale,
                query.min_level,
                query.max_level,
                query.before_id.map(number).transpose()?,
                json(&query.configuration_ids)?,
                json(&query.roots)?,
                query.text.as_ref().map(|text| text.to_lowercase()),
                u32::try_from(query.limit + 1).expect("validated summary page size")
            ],
            |r| unsigned(r, 0),
        )
        .map_err(|e| db_error(&e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| db_error(&e))?;
    let has_more = ids.len() > query.limit;
    let summaries = ids
        .into_iter()
        .take(query.limit)
        .map(|id| {
            let record = record(db, id, query.include_stale)?;
            Ok(SummaryHit {
                id: record.id,
                configuration_id: record.configuration_id,
                kind: record.kind,
                level: record.level,
                preview: record.content.embedding_text().chars().take(280).collect(),
                title: record.content.title,
                source_passages: record.source_passages,
                source_files: record.source_files,
                stale: record.invalidated_at.is_some(),
            })
        })
        .collect::<Result<Vec<_>, IndexError>>()?;
    let next_before = has_more.then(|| summaries.last().expect("nonempty summary page").id);
    Ok(SummaryPage {
        summaries,
        next_before,
    })
}
