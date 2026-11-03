//! Small persisted capture summaries for Knowledge, without loading transcripts.

use sovra_core::{
    EpisodeId, MemoryActivity, MemoryError, MemoryJobState, MemoryOverview, Timestamp,
};
use surrealdb::IndexedResults;
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

use super::{SurrealStore, backend, ingestion::job_state};

impl SurrealStore {
    pub(super) async fn subject_page(
        &self,
        query: sovra_core::MemorySubjectQuery,
    ) -> Result<sovra_core::MemorySubjectPage, MemoryError> {
        query.validate()?;
        let mut rows: Vec<SubjectCount> = self
            .query(
                "SELECT out.name AS name, count() AS total FROM mentions
             WHERE in.stopped_at IS NONE AND in.invalid_at IS NONE AND in.expired_at IS NONE
             AND string::lowercase(out.name) != 'user'
             AND ($search = '' OR string::contains(string::lowercase(out.name), $search))
             AND ($after IS NONE OR out.name > $after)
             GROUP BY name ORDER BY name ASC LIMIT $limit;",
                (
                    "search",
                    query.search.trim().to_lowercase(),
                    "after",
                    query.after,
                    "limit",
                    query.limit + 1,
                ),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let next = if more {
            rows.last().map(|row| row.name.clone())
        } else {
            None
        };
        Ok(sovra_core::MemorySubjectPage {
            subjects: rows
                .into_iter()
                .map(|row| sovra_core::MemorySubject {
                    name: row.name,
                    details: row.total,
                })
                .collect(),
            next,
        })
    }

    pub(super) async fn memory_overview(&self) -> Result<MemoryOverview, MemoryError> {
        let mut result = self.query(
            "SELECT count() AS total FROM fact WHERE stopped_at IS NONE AND invalid_at IS NONE AND expired_at IS NONE GROUP ALL;
             SELECT count() AS total FROM fact WHERE stopped_at IS NONE AND (invalid_at IS NOT NONE OR expired_at IS NOT NONE) GROUP ALL;
             SELECT count() AS total FROM fact WHERE stopped_at IS NOT NONE GROUP ALL;
             SELECT state, count() AS total FROM memory_job GROUP BY state;
             SELECT id, episode, episode.session_id AS session_id, created_at, state,
                 array::len(fact_ids) AS saved, error FROM memory_job ORDER BY created_at DESC, id DESC LIMIT 8;
             SELECT id, created_at FROM fact ORDER BY created_at DESC, id DESC LIMIT 1;
             SELECT out.name AS name, count() AS total FROM mentions
                 WHERE in.stopped_at IS NONE AND in.invalid_at IS NONE AND in.expired_at IS NONE
                 AND string::lowercase(out.name) != 'user'
                 GROUP BY name ORDER BY total DESC, name ASC LIMIT 13;",
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
        let mut subjects: Vec<SubjectCount> = result.take(6).map_err(|error| backend(&error))?;
        overview.more_subjects = subjects.len() > 12;
        subjects.truncate(12);
        overview.subjects = subjects
            .into_iter()
            .map(|row| sovra_core::MemorySubject {
                name: row.name,
                details: row.total,
            })
            .collect();
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

#[derive(Debug, SurrealValue)]
struct SubjectCount {
    name: String,
    total: usize,
}
