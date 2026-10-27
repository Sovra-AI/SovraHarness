//! Immutable summary DAG storage and source invalidation.

mod read;
mod write;

use super::derived::{bounded, decode, json, number, unsigned};
use super::{LocalFileIndex, db_error, digest, get_root, now};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    EvidenceBasis, EvidenceTier, IndexError,
    summary::{
        MAX_SUMMARY_INPUT_BYTES, MAX_SUMMARY_LEVEL, PreparedSummary, SUMMARY_ALGORITHM_REVISION,
        SUMMARY_PROMPT_REVISION, SummaryAttempt, SummaryConfiguration, SummaryHit, SummaryIndex,
        SummaryInput, SummaryInputRef, SummaryInvalidation, SummaryPage, SummaryPlan, SummaryQuery,
        SummaryRecord, SummaryRequest, SummaryReservation, SummaryText,
    },
};

fn unavailable() -> IndexError {
    IndexError("This summary or its supporting folder permission is unavailable.".into())
}

fn configuration(db: &Connection, id: u64) -> Result<SummaryConfiguration, IndexError> {
    let row: Option<(String, bool, u64, u32)> = db
        .query_row(
            "SELECT plan,enabled,created,calls_used FROM index_summary_configurations WHERE id=?1",
            [number(id)?],
            |r| Ok((r.get(0)?, r.get(1)?, unsigned(r, 2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| db_error(&e))?;
    let (plan, enabled, created_at, calls_used) = row.ok_or_else(unavailable)?;
    let plan: SummaryPlan = decode(&plan)?;
    plan.validate()?;
    let mut statement = db
        .prepare(
            "SELECT root_id FROM index_summary_scopes WHERE configuration_id=?1 ORDER BY ordinal",
        )
        .map_err(|e| db_error(&e))?;
    let roots = statement
        .query_map([number(id)?], |r| r.get::<_, Option<String>>(0))
        .map_err(|e| db_error(&e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| db_error(&e))?
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(unavailable)?;
    if roots != plan.roots {
        return Err(unavailable());
    }
    Ok(SummaryConfiguration {
        id,
        plan,
        enabled,
        created_at,
        calls_used,
    })
}

fn prepare(db: &Connection, request: &SummaryRequest) -> Result<PreparedSummary, IndexError> {
    request.validate()?;
    let configuration = configuration(db, request.configuration_id)?;
    if !configuration.enabled {
        return Err(IndexError(
            "Summary generation is paused for this map.".into(),
        ));
    }
    let mut level = 1;
    let mut bytes = 0;
    let mut inputs = vec![];
    for reference in &request.inputs {
        let input = match reference {
            SummaryInputRef::Passage(reference) => {
                if !configuration.plan.roots.contains(&reference.root_id) {
                    return Err(unavailable());
                }
                let passage=db.query_row(&format!("{} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4 AND f.available=1 AND f.current_version=v.id",super::PASSAGES),
                    params![reference.root_id,reference.file_id,reference.version_id,reference.ordinal],super::passage)
                    .optional().map_err(|e|db_error(&e))?.ok_or_else(||IndexError("A summary input changed. Refresh its current source references.".into()))?;
                SummaryInput {
                    reference: SummaryInputRef::Passage(reference.clone()),
                    label: format!(
                        "{} · lines {}–{} · version {}",
                        passage.path, passage.start_line, passage.end_line, reference.version_id
                    ),
                    text: passage.text,
                    extraction_note: passage.note,
                }
            }
            SummaryInputRef::Summary(id) => {
                let child = read::record(db, *id, false)?;
                if child.configuration_id != configuration.id {
                    return Err(IndexError(
                        "Summary inputs must belong to the same approved map generation.".into(),
                    ));
                }
                level = level.max(child.level.saturating_add(1));
                SummaryInput { reference:reference.clone(),label:format!("Derived summary {id}: {}",child.content.title),
                    text:child.content.embedding_text(),extraction_note:Some("Model-generated summary; claims retain uncertainty and must be checked against raw source evidence.".into()) }
            }
        };
        bytes += input.text.len()
            + input.label.len()
            + input.extraction_note.as_ref().map_or(0, String::len);
        inputs.push(input);
    }
    if bytes > MAX_SUMMARY_INPUT_BYTES || level > MAX_SUMMARY_LEVEL {
        return Err(IndexError("This summary exceeds the input or hierarchy budget. Split its inputs into smaller groups.".into()));
    }
    let content_key = digest(
        json(&(
            request,
            &configuration.plan.summarizer,
            &configuration.plan.embedder,
            SUMMARY_PROMPT_REVISION,
            SUMMARY_ALGORITHM_REVISION,
            &inputs,
        ))?
        .as_bytes(),
    );
    Ok(PreparedSummary {
        configuration,
        request: request.clone(),
        inputs,
        level,
        content_key,
    })
}

fn revalidate(db: &Connection, prepared: &PreparedSummary) -> Result<PreparedSummary, IndexError> {
    let current = prepare(db, &prepared.request)?;
    if current.content_key != prepared.content_key
        || current.inputs != prepared.inputs
        || current.level != prepared.level
        || current.configuration.id != prepared.configuration.id
        || current.configuration.plan != prepared.configuration.plan
    {
        return Err(IndexError("The summary inputs or configuration changed during generation. This response was not saved.".into()));
    }
    Ok(current)
}

fn cached(
    db: &Connection,
    prepared: &PreparedSummary,
) -> Result<Option<SummaryRecord>, IndexError> {
    let id:Option<u64>=db.query_row("SELECT id FROM index_summaries WHERE configuration_id=?1 AND signature=?2 AND invalidated IS NULL",
        params![number(prepared.configuration.id)?,prepared.content_key],|r|unsigned(r, 0)).optional().map_err(|e|db_error(&e))?;
    id.map(|id| read::record(db, id, false)).transpose()
}

fn check_attempt(db: &Connection, attempt: &SummaryAttempt) -> Result<(), IndexError> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM index_summary_attempts WHERE id=?1 AND configuration_id=?2 AND signature=?3 AND state='pending')",
        params![number(attempt.id)?,number(attempt.configuration_id)?,attempt.content_key],|r|r.get(0)).map_err(|e|db_error(&e))?;
    if valid {
        Ok(())
    } else {
        Err(IndexError(
            "This summary request was already finished or is invalid.".into(),
        ))
    }
}

impl SummaryIndex for LocalFileIndex {
    fn configure(&self, plan: &SummaryPlan) -> Result<SummaryConfiguration, IndexError> {
        plan.validate()?;
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        for root in &plan.roots {
            get_root(&tx, root)?;
        }
        let (active, total): (u64, u64) = tx
            .query_row(
                "SELECT coalesce(sum(enabled),0),count(*) FROM index_summary_configurations",
                [],
                |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
            )
            .map_err(|e| db_error(&e))?;
        if active >= 16 || total >= 1000 {
            return Err(IndexError("Pause an existing knowledge map before adding another. At most 1,000 map configurations are retained.".into()));
        }
        let at = now();
        tx.execute(
            "INSERT INTO index_summary_configurations(plan,enabled,created) VALUES(?1,1,?2)",
            params![json(plan)?, number(at)?],
        )
        .map_err(|e| db_error(&e))?;
        let id = u64::try_from(tx.last_insert_rowid()).map_err(|_| unavailable())?;
        for (ordinal, root) in plan.roots.iter().enumerate() {
            tx.execute("INSERT INTO index_summary_scopes(configuration_id,ordinal,root_id) VALUES(?1,?2,?3)",
                params![number(id)?,u32::try_from(ordinal).expect("bounded root ordinal"),root]).map_err(|e|db_error(&e))?;
        }
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(SummaryConfiguration {
            id,
            plan: plan.clone(),
            enabled: true,
            created_at: at,
            calls_used: 0,
        })
    }

    fn configurations(&self) -> Result<Vec<SummaryConfiguration>, IndexError> {
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let mut query=tx.prepare("SELECT c.id FROM index_summary_configurations c WHERE NOT EXISTS(SELECT 1 FROM index_summary_scopes s WHERE s.configuration_id=c.id AND s.root_id IS NULL) ORDER BY enabled DESC,id DESC LIMIT 64").map_err(|e|db_error(&e))?;
        let ids = query
            .query_map([], |r| unsigned(r, 0))
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?;
        ids.into_iter().map(|id| configuration(&tx, id)).collect()
    }

    fn set_enabled(&self, id: u64, enabled: bool) -> Result<(), IndexError> {
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let previous = configuration(&tx, id)?;
        if enabled && !previous.enabled {
            let count: u64 = tx
                .query_row(
                    "SELECT count(*) FROM index_summary_configurations WHERE enabled=1",
                    [],
                    |r| unsigned(r, 0),
                )
                .map_err(|e| db_error(&e))?;
            if count >= 16 {
                return Err(IndexError(
                    "At most sixteen knowledge maps may generate summaries at once.".into(),
                ));
            }
        }
        tx.execute(
            "UPDATE index_summary_configurations SET enabled=?2 WHERE id=?1",
            params![number(id)?, enabled],
        )
        .map_err(|e| db_error(&e))?;
        // A paused request must never commit after a quick pause/resume cycle.
        if !enabled {
            tx.execute("UPDATE index_summary_attempts SET state='cancelled' WHERE configuration_id=?1 AND state='pending'",[number(id)?]).map_err(|e|db_error(&e))?;
        }
        tx.commit().map_err(|e| db_error(&e))
    }

    fn prepare(&self, request: &SummaryRequest) -> Result<PreparedSummary, IndexError> {
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        prepare(&tx, request)
    }

    fn reserve(&self, prepared: &PreparedSummary) -> Result<SummaryReservation, IndexError> {
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let current = revalidate(&tx, prepared)?;
        if let Some(record) = cached(&tx, &current)? {
            return Ok(SummaryReservation::Cached(Box::new(record)));
        }
        if current.configuration.calls_used >= current.configuration.plan.max_calls {
            return Err(IndexError("This knowledge map has used its approved summary request budget. Choose a new build budget to continue.".into()));
        }
        tx.execute(
            "UPDATE index_summary_configurations SET calls_used=calls_used+1 WHERE id=?1",
            [number(current.configuration.id)?],
        )
        .map_err(|e| db_error(&e))?;
        tx.execute("INSERT INTO index_summary_attempts(configuration_id,signature,state,created) VALUES(?1,?2,'pending',?3)",params![number(current.configuration.id)?,current.content_key,number(now())?]).map_err(|e|db_error(&e))?;
        let result = SummaryAttempt {
            id: u64::try_from(tx.last_insert_rowid()).map_err(|_| unavailable())?,
            configuration_id: current.configuration.id,
            content_key: current.content_key,
        };
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(SummaryReservation::Attempt(result))
    }

    fn fail_summary(&self, attempt: &SummaryAttempt, message: &str) -> Result<(), IndexError> {
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        check_attempt(&tx, attempt)?;
        let message: String = message.chars().take(500).collect();
        tx.execute(
            "UPDATE index_summary_attempts SET state='failed',error=?2 WHERE id=?1",
            params![number(attempt.id)?, message],
        )
        .map_err(|e| db_error(&e))?;
        tx.commit().map_err(|e| db_error(&e))
    }

    fn commit(
        &self,
        attempt: &SummaryAttempt,
        prepared: &PreparedSummary,
        output: &SummaryText,
    ) -> Result<SummaryRecord, IndexError> {
        output.validate(prepared.inputs.len())?;
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        check_attempt(&tx, attempt)?;
        if attempt.configuration_id != prepared.configuration.id
            || attempt.content_key != prepared.content_key
        {
            return Err(IndexError(
                "This response does not match its reserved summary request.".into(),
            ));
        }
        let current = revalidate(&tx, prepared)?;
        let record = if let Some(record) = cached(&tx, &current)? {
            record
        } else {
            write::commit(&tx, &current, output, attempt.id)?
        };
        tx.execute(
            "UPDATE index_summary_attempts SET state='finished',result_id=?2 WHERE id=?1",
            params![number(attempt.id)?, number(record.id)?],
        )
        .map_err(|e| db_error(&e))?;
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(record)
    }

    fn read_summary(&self, id: u64, include_stale: bool) -> Result<SummaryRecord, IndexError> {
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        read::record(&tx, id, include_stale)
    }

    fn query_summaries(&self, query: &SummaryQuery) -> Result<SummaryPage, IndexError> {
        query.validate()?;
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        read::query(&tx, query)
    }

    fn invalidate_summary(&self, id: u64) -> Result<u64, IndexError> {
        let _lock = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        read::record(&tx, id, true)?;
        let changed=tx.execute("WITH RECURSIVE dependents(id) AS (
            SELECT ?1 UNION SELECT e.parent FROM index_summary_edges e JOIN dependents d ON e.child=d.id
        ) UPDATE index_summaries SET invalidated=?2,reason=?3 WHERE invalidated IS NULL AND id IN (SELECT id FROM dependents)",
            params![number(id)?,number(now())?,json(&SummaryInvalidation::DependencyInvalidated)?]).map_err(|e|db_error(&e))?;
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(u64::try_from(changed).expect("bounded summary record count"))
    }
}
