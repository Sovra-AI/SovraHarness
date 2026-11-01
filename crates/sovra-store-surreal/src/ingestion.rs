//! Persisted ingestion leases; model work stays outside database transactions.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};
use sovra_core::{
    EpisodeId, FactId, MAX_MEMORY_ATTEMPTS, MAX_MEMORY_CANDIDATES, MAX_PENDING_MEMORIES,
    MEMORY_LEASE_MILLIS, MEMORY_RETRY_MILLIS, MemoryCandidate, MemoryClaim, MemoryError,
    MemoryExtraction, MemoryInput, MemoryJob, MemoryJobState, MemoryPolicy, MemoryStore, NewFact,
    Timestamp, validate_memory_batch,
};
use surrealdb::IndexedResults;
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

use super::{EpisodeRow, NewEpisodeRow, SurrealStore, backend, facts, kind_to_str, parse_id};

fn stable_key(parts: &[&str]) -> Result<String, MemoryError> {
    let bytes =
        serde_json::to_vec(parts).map_err(|error| MemoryError::Backend(error.to_string()))?;
    let mut key = String::from("memory_");
    for byte in Sha256::digest(bytes) {
        let _ = write!(key, "{byte:02x}");
    }
    Ok(key)
}

fn job_id(episode: &EpisodeId) -> Result<RecordId, MemoryError> {
    let episode = parse_id(&episode.0, "episode")?;
    Ok(RecordId::new("memory_job", episode.key))
}

fn returned<T: SurrealValue>(mut response: IndexedResults) -> Result<T, MemoryError>
where
    usize: surrealdb::opt::QueryResult<T>,
{
    let index = response.num_statements().saturating_sub(2);
    response.take(index).map_err(|error| backend(&error))
}

impl SurrealStore {
    pub(super) async fn read_memory_policy(&self) -> Result<MemoryPolicy, MemoryError> {
        let policy: Option<PolicyRow> = self
            .query(
                "SELECT enabled, generation FROM ONLY meta:remembering;",
                surrealdb::types::Object::default(),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        policy
            .map(PolicyRow::into_policy)
            .ok_or_else(|| MemoryError::Backend("missing memory policy".into()))
    }

    pub(super) async fn change_memory_policy(
        &self,
        enabled: bool,
    ) -> Result<MemoryPolicy, MemoryError> {
        let policy: Option<PolicyRow> = returned(
            self.query(
                "BEGIN TRANSACTION;
             LET $policy = (SELECT * FROM ONLY meta:remembering);
             IF $policy.enabled != $enabled {
                 UPDATE meta:remembering SET enabled = $enabled, generation += 1;
                 IF !$enabled {
                     UPDATE memory_job SET state = 'cancelled', token = NONE
                         WHERE state IN ['pending', 'processing'];
                 };
             };
             RETURN (SELECT enabled, generation FROM ONLY meta:remembering);
             COMMIT TRANSACTION;",
                ("enabled", enabled),
            )
            .await?,
        )?;
        policy
            .map(PolicyRow::into_policy)
            .ok_or_else(|| MemoryError::Backend("missing memory policy".into()))
    }

    pub(super) async fn enqueue(&self, input: MemoryInput) -> Result<EpisodeId, MemoryError> {
        input.validate()?;
        let key = stable_key(&[&input.episode.session_id, &input.key])?;
        let episode = RecordId::new("episode", key.clone());
        let job = RecordId::new("memory_job", key);
        let data = NewEpisodeRow {
            session_id: input.episode.session_id,
            kind: kind_to_str(input.episode.kind).into(),
            content: input.episode.content,
            occurred_at: input.episode.occurred_at.millis(),
        };
        let id: Option<RecordId> = returned(self.query(
            "BEGIN TRANSACTION;
             LET $existing = (SELECT * FROM ONLY $episode);
             IF $existing != NONE {
                 IF $existing.session_id != $data.session_id OR $existing.kind != $data.kind
                     OR $existing.content != $data.content OR $existing.occurred_at != $data.occurred_at {
                     THROW 'memory input key was reused with different evidence';
                 };
             } ELSE {
                 LET $policy = (SELECT * FROM ONLY meta:remembering);
                 IF !$policy.enabled { THROW 'remembering is paused'; };
                 LET $active = (SELECT id FROM memory_job WHERE state IN ['pending', 'processing'] LIMIT $bound);
                 IF array::len($active) >= $bound { THROW 'remembering queue is full'; };
                 UPDATE meta:remembering SET revision += 1;
                 CREATE ONLY $episode CONTENT $data;
                 CREATE ONLY $job SET episode = $episode, state = 'pending', attempts = 0,
                     generation = $policy.generation, retry_at = 0, lease_until = 0,
                     created_at = $created, fact_ids = [];
             };
             RETURN $episode;
             COMMIT TRANSACTION;",
            ("episode", episode, "job", job, "data", data, "bound", MAX_PENDING_MEMORIES, "created", super::now_ms()),
        ).await?)?;
        id.map(|id| EpisodeId(id.to_sql()))
            .ok_or_else(|| MemoryError::Backend("enqueue returned no episode".into()))
    }

    pub(super) async fn claim(&self, at: Timestamp) -> Result<Option<MemoryClaim>, MemoryError> {
        let lease = at
            .millis()
            .checked_add(MEMORY_LEASE_MILLIS)
            .ok_or_else(|| MemoryError::InvalidInput("invalid memory lease time".into()))?;
        let row: Option<ClaimRow> = returned(self.query(
            "BEGIN TRANSACTION;
             LET $policy = (SELECT * FROM ONLY meta:remembering);
             UPDATE memory_job SET state = 'failed', error = 'Remembering was interrupted too many times.'
                 WHERE state = 'processing' AND lease_until <= $now AND attempts >= $attempts;
             LET $next = (SELECT id, created_at FROM memory_job
                 WHERE $policy.enabled AND generation = $policy.generation AND attempts < $attempts
                 AND ((state = 'pending' AND retry_at <= $now) OR (state = 'processing' AND lease_until <= $now))
                 ORDER BY created_at ASC, id ASC LIMIT 1)[0].id;
             IF $next != NONE {
                 UPDATE meta:remembering SET revision += 1;
                 UPDATE $next SET state = 'processing', attempts += 1, token = rand::ulid(), lease_until = $lease;
             };
             RETURN IF $next = NONE { NONE } ELSE {
                 SELECT episode.* AS episode, token, generation, attempts, lease_until FROM ONLY $next
             };
             COMMIT TRANSACTION;",
            ("now", at.millis(), "lease", lease, "attempts", MAX_MEMORY_ATTEMPTS),
        ).await?)?;
        row.map(ClaimRow::into_claim).transpose()
    }

    pub(super) async fn finish(
        &self,
        claim: &MemoryClaim,
        candidates: Vec<MemoryCandidate>,
        model: &str,
        at: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        if candidates.len() > MAX_MEMORY_CANDIDATES || model.trim().is_empty() || model.len() > 512
        {
            return Err(MemoryError::InvalidInput(
                "invalid memory extraction batch".into(),
            ));
        }
        // Validate against persisted evidence, not a caller-modified claim snapshot.
        let source = self.episode(&claim.episode.id).await?;
        validate_memory_batch(&source, &candidates)?;
        let payload = serde_json::to_string(&candidates)
            .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
        let fingerprint = stable_key(&[&payload, model])?;
        let mut writes = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let mut entities = candidate.entities.clone();
            entities.sort();
            entities.dedup();
            let identity =
                serde_json::to_string(&(&candidate.content, entities, candidate.valid_at))
                    .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
            let same_as = candidate
                .same_as
                .as_ref()
                .map(|id| parse_id(&id.0, "fact"))
                .transpose()?;
            let mut write = facts::prepare(
                NewFact {
                    content: candidate.content.clone(),
                    entities: candidate.entities,
                    embedding: None,
                    source: Some(source.id.clone()),
                    valid_at: candidate.valid_at,
                    replaces: candidate.replaces,
                },
                self.next_fact_ms(),
                Some(MemoryExtraction {
                    model: model.into(),
                    evidence: candidate.evidence,
                }),
            )?;
            write.id = Some(RecordId::new(
                "fact",
                stable_key(&[&source.id.0, &identity])?,
            ));
            write.alias_key = Some(stable_key(&[&identity])?);
            write.same_as = same_as;
            writes.push(write);
        }
        let sql = format!("BEGIN TRANSACTION;
            LET $job = (SELECT * FROM ONLY $job_id);
            LET $policy = (SELECT * FROM ONLY meta:remembering);
            IF $job.state = 'complete' AND $job.token = $lease_token AND $job.fingerprint = $fingerprint {{
                RETURN $job.fact_ids;
            }} ELSE {{
                IF !$policy.enabled OR $job.generation != $generation OR $policy.generation != $generation
                    OR $job.state != 'processing' OR $job.token != $lease_token OR $job.lease_until <= $now {{
                    THROW 'memory lease is stale or remembering is paused';
                }};
                UPDATE meta:remembering SET revision += 1;
                FOR $write IN $writes {{
                    LET $same = IF $write.same_as != NONE {{ (SELECT * FROM ONLY $write.same_as) }} ELSE {{ NONE }};
                    IF $write.same_as != NONE AND (
                        $same.id IS NONE OR $same.stopped_at IS NOT NONE OR $same.invalid_at IS NOT NONE OR $same.expired_at IS NOT NONE
                        OR array::sort(array::distinct($same.entities)) != array::sort(array::distinct($write.data.entities))
                        OR ($write.match_valid_at IS NOT NONE AND $same.valid_at != $write.match_valid_at)
                    ) {{ THROW 'memory reuse is stale or incompatible'; }};
                    LET $known = IF $write.replaces != NONE {{ NONE }} ELSE IF $write.same_as != NONE {{ $write.same_as }} ELSE {{
                        LET $direct = (SELECT id, created_at FROM fact WHERE content = $write.data.content
                            AND array::sort(array::distinct(entities)) = array::sort(array::distinct($write.data.entities))
                            AND ($write.match_valid_at IS NONE OR valid_at = $write.match_valid_at)
                            ORDER BY created_at DESC LIMIT 1)[0];
                        LET $alias = (SELECT fact AS id, created_at FROM memory_alias WHERE content = $write.data.content
                            AND array::sort(array::distinct(entities)) = array::sort(array::distinct($write.data.entities))
                            AND ($write.match_valid_at IS NONE OR valid_at = $write.match_valid_at)
                            ORDER BY created_at DESC LIMIT 1)[0];
                        IF $alias.id IS NOT NONE AND ($direct.id IS NONE OR $alias.created_at > $direct.created_at) {{ $alias.id }} ELSE {{ $direct.id }}
                    }};
                    LET $selected = IF $known != NONE {{
                        $known
                    }} ELSE {{
                        {}
                        $created
                    }};
                    LET $saved = (SELECT created_at, valid_at FROM ONLY $selected);
                    UPSERT type::record('memory_alias', type::string($selected) + ':' + $write.alias_key)
                        SET fact = $selected, content = $write.data.content, entities = $write.data.entities,
                            created_at = $saved.created_at, valid_at = $saved.valid_at;
                    UPDATE $job_id SET fact_ids = array::union(fact_ids, [$selected]);
                }};
                UPDATE $job_id SET state = 'complete', fingerprint = $fingerprint, error = NONE, result_json = $payload, extractor = $model;
            }};
            RETURN (SELECT VALUE fact_ids FROM ONLY $job_id);
            COMMIT TRANSACTION;", facts::WRITE);
        let ids: Vec<RecordId> = returned(
            self.query(
                &sql,
                FinishBindings {
                    job_id: job_id(&source.id)?,
                    lease_token: claim.token.clone(),
                    generation: claim.generation,
                    fingerprint,
                    payload,
                    model: model.into(),
                    now: at.millis(),
                    writes,
                },
            )
            .await?,
        )?;
        Ok(ids.into_iter().map(|id| FactId(id.to_sql())).collect())
    }

    pub(super) async fn fail(
        &self,
        claim: &MemoryClaim,
        message: &str,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        if message.trim().is_empty() || message.len() > 1_024 {
            return Err(MemoryError::InvalidInput(
                "invalid remembering error message".into(),
            ));
        }
        let retry = at
            .millis()
            .checked_add(MEMORY_RETRY_MILLIS)
            .ok_or_else(|| MemoryError::InvalidInput("invalid memory retry time".into()))?;
        self.query("BEGIN TRANSACTION;
            LET $job = (SELECT * FROM ONLY $job_id);
            LET $policy = (SELECT * FROM ONLY meta:remembering);
            IF !$policy.enabled OR $policy.generation != $generation OR $job.generation != $generation
                OR $job.state != 'processing' OR $job.token != $lease_token OR $job.lease_until <= $now {
                THROW 'memory lease is stale or remembering is paused';
            };
            UPDATE meta:remembering SET revision += 1;
            UPDATE $job_id SET state = IF attempts >= $attempts { 'failed' } ELSE { 'pending' },
                error = $message, retry_at = $retry, token = NONE;
            COMMIT TRANSACTION;", FailBindings {
                job_id: job_id(&claim.episode.id)?, lease_token: claim.token.clone(), generation: claim.generation,
                now: at.millis(), retry, message: message.to_owned(), attempts: MAX_MEMORY_ATTEMPTS,
            }).await?;
        Ok(())
    }

    pub(super) async fn read_job(&self, episode: &EpisodeId) -> Result<MemoryJob, MemoryError> {
        let row: Option<JobRow> = self
            .query(
                "SELECT episode, state, attempts, error, fact_ids, result_json, extractor FROM ONLY $job;",
                ("job", job_id(episode)?),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        row.ok_or_else(|| MemoryError::NotFound(episode.0.clone()))?
            .into_job()
    }
}

#[derive(Debug, SurrealValue)]
struct PolicyRow {
    enabled: bool,
    generation: u64,
}
impl PolicyRow {
    fn into_policy(self) -> MemoryPolicy {
        MemoryPolicy {
            enabled: self.enabled,
            generation: self.generation,
        }
    }
}

#[derive(Debug, SurrealValue)]
struct ClaimRow {
    episode: EpisodeRow,
    token: String,
    generation: u64,
    attempts: u32,
    lease_until: i64,
}
impl ClaimRow {
    fn into_claim(self) -> Result<MemoryClaim, MemoryError> {
        Ok(MemoryClaim {
            episode: self.episode.into_episode()?,
            token: self.token,
            generation: self.generation,
            attempt: self.attempts,
            lease_until: Timestamp(self.lease_until),
        })
    }
}

#[derive(Debug, SurrealValue)]
struct JobRow {
    result_json: Option<String>,
    extractor: Option<String>,
    episode: RecordId,
    state: String,
    attempts: u32,
    error: Option<String>,
    fact_ids: Vec<RecordId>,
}
impl JobRow {
    fn into_job(self) -> Result<MemoryJob, MemoryError> {
        let state = job_state(&self.state)?;
        let candidates = self
            .result_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| MemoryError::Backend(format!("invalid saved extraction: {error}")))?
            .unwrap_or_default();
        Ok(MemoryJob {
            candidates,
            extraction_model: self.extractor,
            episode_id: EpisodeId(self.episode.to_sql()),
            state,
            attempts: self.attempts,
            error: self.error,
            fact_ids: self
                .fact_ids
                .into_iter()
                .map(|id| FactId(id.to_sql()))
                .collect(),
        })
    }
}

pub(super) fn job_state(value: &str) -> Result<MemoryJobState, MemoryError> {
    Ok(match value {
        "pending" => MemoryJobState::Pending,
        "processing" => MemoryJobState::Processing,
        "complete" => MemoryJobState::Complete,
        "failed" => MemoryJobState::Failed,
        "cancelled" => MemoryJobState::Cancelled,
        _ => return Err(MemoryError::Backend("unknown remembering job state".into())),
    })
}

#[derive(Debug, SurrealValue)]
struct FinishBindings {
    payload: String,
    model: String,
    job_id: RecordId,
    lease_token: String,
    generation: u64,
    fingerprint: String,
    now: i64,
    writes: Vec<facts::FactWrite>,
}
#[derive(Debug, SurrealValue)]
struct FailBindings {
    job_id: RecordId,
    lease_token: String,
    generation: u64,
    now: i64,
    retry: i64,
    message: String,
    attempts: u32,
}
