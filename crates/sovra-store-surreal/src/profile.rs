//! Small persisted capture summaries for Knowledge, without loading transcripts.

use sovra_core::{
    EpisodeId, MemoryActivity, MemoryError, MemoryJobState, MemoryOverview, Timestamp,
};
use surrealdb::IndexedResults;
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

use super::{SurrealStore, backend, ingestion::job_state};

impl SurrealStore {
    pub(super) async fn memory_overview(&self) -> Result<MemoryOverview, MemoryError> {
        let mut result = self.query(
            "SELECT count() AS total FROM fact WHERE stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE GROUP ALL;
             SELECT count() AS total FROM fact WHERE stopped_at IS NONE AND (invalid_at IS NOT NONE OR expired_at IS NOT NONE) GROUP ALL;
             SELECT count() AS total FROM fact WHERE stopped_at IS NOT NONE GROUP ALL;
             SELECT state, count() AS total FROM memory_job GROUP BY state;
             SELECT id, episode, episode.session_id AS session_id, created_at, state,
                 array::len(fact_ids) AS saved, error FROM memory_job ORDER BY created_at DESC, id DESC LIMIT 8;
             SELECT id, created_at FROM fact ORDER BY created_at DESC, id DESC LIMIT 1;",
            surrealdb::types::Object::default(),
        ).await?;
        let mut overview = MemoryOverview {
            current: count(&mut result, 0)?,
            history: count(&mut result, 1)?,
            stopped: count(&mut result, 2)?,
            ..MemoryOverview::default()
        };
        let jobs: Vec<StateCount> = result.take(3).map_err(|error| backend(&error))?;
        for row in jobs {
            match job_state(&row.state)? {
                MemoryJobState::Pending => overview.waiting = row.total,
                MemoryJobState::Processing => overview.working = row.total,
                MemoryJobState::Complete => overview.checked = row.total,
                MemoryJobState::Failed => overview.failed = row.total,
                MemoryJobState::Cancelled => overview.cancelled = row.total,
            }
        }
        let recent: Vec<ActivityRow> = result.take(4).map_err(|error| backend(&error))?;
        overview.recent_activity = recent
            .into_iter()
            .map(|row| {
                Ok(MemoryActivity {
                    episode_id: EpisodeId(row.episode.to_sql()),
                    session_id: row.session_id,
                    created_at: Timestamp(row.created_at),
                    state: job_state(&row.state)?,
                    saved: row.saved,
                    error: row.error,
                })
            })
            .collect::<Result<_, MemoryError>>()?;
        let latest: Vec<SavedAt> = result.take(5).map_err(|error| backend(&error))?;
        overview.last_saved_at = latest.first().map(|row| Timestamp(row.created_at));
        Ok(overview)
    }
}

fn count(result: &mut IndexedResults, index: usize) -> Result<usize, MemoryError> {
    let rows: Vec<Count> = result.take(index).map_err(|error| backend(&error))?;
    Ok(rows.first().map_or(0, |row| row.total))
}

#[derive(Debug, SurrealValue)]
struct Count {
    total: usize,
}
#[derive(Debug, SurrealValue)]
struct StateCount {
    state: String,
    total: usize,
}
#[derive(Debug, SurrealValue)]
struct SavedAt {
    created_at: i64,
}
#[derive(Debug, SurrealValue)]
struct ActivityRow {
    episode: RecordId,
    session_id: String,
    created_at: i64,
    state: String,
    saved: usize,
    error: Option<String>,
}
