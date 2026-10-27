use super::{
    Connection, EvidenceBasis, EvidenceTier, IndexError, PreparedSummary,
    SUMMARY_ALGORITHM_REVISION, SUMMARY_PROMPT_REVISION, SummaryInputRef, SummaryRecord,
    SummaryText, db_error, json, now, number, params, unavailable, unsigned,
};

pub(super) fn commit(
    db: &Connection,
    prepared: &PreparedSummary,
    output: &SummaryText,
    attempt: u64,
) -> Result<SummaryRecord, IndexError> {
    let count: u64 = db
        .query_row("SELECT count(*) FROM index_summaries", [], |r| {
            unsigned(r, 0)
        })
        .map_err(|e| db_error(&e))?;
    if count >= 10_000 {
        return Err(IndexError(
            "The local summary history has reached its 10,000-record limit.".into(),
        ));
    }
    let at = now();
    db.execute("INSERT INTO index_summaries(configuration_id,kind,level,signature,created,details,search_text) VALUES(?1,?2,?3,?4,?5,'',?6)",
        params![number(prepared.configuration.id)?,json(&prepared.request.kind)?,prepared.level,prepared.content_key,number(at)?,output.embedding_text().to_lowercase()])
        .map_err(|e|db_error(&e))?;
    let id = u64::try_from(db.last_insert_rowid()).map_err(|_| unavailable())?;
    for input in &prepared.inputs {
        match &input.reference {
            SummaryInputRef::Passage(reference) => {
                db.execute("INSERT OR IGNORE INTO index_summary_sources(summary_id,root_id,file_id,version_id,ordinal) VALUES(?1,?2,?3,?4,?5)",
                    params![number(id)?,reference.root_id,reference.file_id,reference.version_id,reference.ordinal]).map_err(|e|db_error(&e))?;
            }
            SummaryInputRef::Summary(child) => {
                db.execute(
                    "INSERT INTO index_summary_edges(parent,child) VALUES(?1,?2)",
                    params![number(id)?, number(*child)?],
                )
                .map_err(|e| db_error(&e))?;
                db.execute("INSERT OR IGNORE INTO index_summary_sources(summary_id,root_id,file_id,version_id,ordinal) SELECT ?1,root_id,file_id,version_id,ordinal FROM index_summary_sources WHERE summary_id=?2",
                    params![number(id)?,number(*child)?]).map_err(|e|db_error(&e))?;
            }
        }
    }
    let (source_passages,source_files): (u64,u64) = db.query_row("SELECT count(*),count(DISTINCT file_id) FROM index_summary_sources WHERE summary_id=?1",[number(id)?],|r|Ok((unsigned(r, 0)?,unsigned(r, 1)?))).map_err(|e|db_error(&e))?;
    let record = SummaryRecord {
        id,
        configuration_id: prepared.configuration.id,
        kind: prepared.request.kind,
        level: prepared.level,
        content: output.clone(),
        inputs: prepared.inputs.clone(),
        model: prepared.configuration.plan.summarizer.clone(),
        prompt_revision: SUMMARY_PROMPT_REVISION.into(),
        algorithm_revision: SUMMARY_ALGORITHM_REVISION.into(),
        basis: EvidenceBasis {
            tier: EvidenceTier::ModelInference,
            confidence: None,
            activity_id: Some(attempt),
            recorded_at: at,
            observed_at: None,
            valid_time: None,
        },
        source_passages,
        source_files,
        invalidated_at: None,
        invalidation: None,
    };
    db.execute(
        "UPDATE index_summaries SET details=?2 WHERE id=?1",
        params![number(id)?, json(&record)?],
    )
    .map_err(|e| db_error(&e))?;
    Ok(record)
}
