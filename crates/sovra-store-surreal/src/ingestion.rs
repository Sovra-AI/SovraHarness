//! Persisted ingestion leases; model work stays outside database transactions.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};
use sovra_core::{
    EpisodeId, FactId, MAX_MEMORY_ATTEMPTS, MAX_MEMORY_CANDIDATES, MAX_PENDING_MEMORIES,
    MEMORY_LEASE_MILLIS, MEMORY_RETRY_MILLIS, MemoryCandidate, MemoryClaim, MemoryError,
    MemoryExtractedChunk, MemoryExtraction, MemoryInput, MemoryJob, MemoryJobState, MemoryPolicy,
    MemoryStore, NewFact, Timestamp, validate_memory_batch,
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

#[derive(SurrealValue)]
struct AdmissionResult {
    id: RecordId,
    dependency_complete: bool,
}

impl SurrealStore {
    // Existing jobs use their own row as the cancellation fence. Pause and source
    // withdrawal both write that row, so stale job writes conflict in the pinned
    // engine. Admission and explicit retry still need their separate policy fence.
    pub(super) async fn stage_extraction(
        &self,
        claim: &MemoryClaim,
        extracted: &MemoryExtractedChunk,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        let mut persisted = claim.clone();
        persisted.episode = self.episode(&claim.episode.id).await?;
        validate_memory_batch(&persisted.chunk()?, &extracted.candidates)?;
        if extracted.model.trim().is_empty() || extracted.model.len() > 512 {
            return Err(MemoryError::InvalidInput("invalid extraction model".into()));
        }
        let payload = serde_json::to_string(extracted)
            .map_err(|error| MemoryError::Backend(error.to_string()))?;
        self.query("BEGIN TRANSACTION;
            LET $job = (SELECT * FROM ONLY $job_id);
            LET $policy = (SELECT * FROM ONLY meta:remembering);
            IF !$policy.enabled OR $job.episode.revoked_at IS NOT NONE
                OR $job.generation != $generation OR $policy.generation != $generation
                OR $job.state != 'processing' OR $job.token != $lease_token OR $job.lease_until <= $now
                OR $job.cursor != $cursor { THROW 'extraction lease is stale'; };
            IF $job.extracted_json IS NOT NONE AND $job.extracted_json != $payload { THROW 'extraction already saved'; };

            UPDATE $job_id SET extracted_json = $payload;
            COMMIT TRANSACTION;", ExtractionBindings {
                job_id: job_id(&claim.episode.id)?, generation: claim.generation,
                lease_token: claim.token.clone(), now: at.millis(), cursor: claim.cursor, payload,
            }).await?;
        Ok(())
    }

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
        self.admit_source(sovra_core::MemorySourceAdmission::user(input))
            .await
            .map(|receipt| receipt.source_id)
    }

    pub(super) async fn admit_source(
        &self,
        input: sovra_core::MemorySourceAdmission,
    ) -> Result<sovra_core::MemorySourceReceipt, MemoryError> {
        input.validate()?;
        let base = stable_key(&[&input.episode.session_id, &input.key])?;
        let key = if input.version == 1 {
            base.clone()
        } else {
            stable_key(&[
                &input.episode.session_id,
                &input.key,
                &input.version.to_string(),
            ])?
        };
        let dependencies_supplied = input.dependencies.is_some();
        let mut dependencies = input.dependencies.clone().unwrap_or_default();
        dependencies.sources.sort_by(|a, b| a.0.cmp(&b.0));
        dependencies.sources.dedup();
        dependencies.facts.sort_by(|a, b| a.0.cmp(&b.0));
        dependencies.facts.dedup();
        let manifest = dependencies_supplied
            .then(|| serde_json::to_string(&dependencies))
            .transpose()
            .map_err(|error| MemoryError::Backend(error.to_string()))?;
        let requested_sources = dependencies
            .sources
            .iter()
            .map(|id| parse_id(&id.0, "episode"))
            .collect::<Result<Vec<_>, _>>()?;
        let requested_facts = dependencies
            .facts
            .iter()
            .map(|id| parse_id(&id.0, "fact"))
            .collect::<Result<Vec<_>, _>>()?;
        let version = input.version;
        let extract = input.needs_extraction();
        let metadata = serde_json::to_string(&sovra_core::MemorySourceAttribution {
            producer_key: input.key,
            author: input.author.clone(),
            provenance: input.provenance,
        })
        .map_err(|error| MemoryError::Backend(error.to_string()))?;
        let origin = match input.author {
            sovra_core::MemorySourceAuthor::User => "user_statement",
            sovra_core::MemorySourceAuthor::Assistant { .. } => "assistant_output",
        };
        let data = NewEpisodeRow {
            session_id: input.episode.session_id,
            kind: kind_to_str(input.episode.kind).into(),
            content: input.episode.content,
            occurred_at: input.episode.occurred_at.millis(),
        };
        let mut bindings = surrealdb::types::Object::default();
        bindings.insert("episode", RecordId::new("episode", key.clone()));
        bindings.insert("job", RecordId::new("memory_job", key));
        bindings.insert("family", RecordId::new("episode", base));
        bindings.insert("data", data);
        bindings.insert("created", self.next_fact_ms());
        bindings.insert("version", version);
        bindings.insert("metadata", metadata);
        bindings.insert("origin", origin);
        bindings.insert("extract", extract);
        bindings.insert("expected_generation", input.expected_generation);
        bindings.insert("dependencies_supplied", dependencies_supplied);
        bindings.insert("dependency_manifest", manifest);
        bindings.insert("requested_sources", requested_sources);
        bindings.insert("requested_facts", requested_facts);
        bindings.insert(
            "dependency_limit",
            sovra_core::MAX_MEMORY_SOURCE_DEPENDENCIES,
        );
        let admitted: Option<AdmissionResult> = returned(self.query(
            &format!("BEGIN TRANSACTION;{}", [super::reply_dependencies::RESOLVE,
            "
             LET $policy = (SELECT * FROM ONLY meta:remembering);
             IF $expected_generation IS NOT NONE AND
                 (!$policy.enabled OR $policy.generation != $expected_generation) {
                 THROW 'source capture permission changed';
             };
             LET $existing = (SELECT * FROM ONLY $episode);
             IF $existing != NONE {
                 IF $existing.session_id != $data.session_id OR $existing.kind != $data.kind
                     OR $existing.content != $data.content OR $existing.occurred_at != $data.occurred_at
                     OR $existing.origin != $origin OR $existing.permission_scope != 'private'
                     OR $existing.source_version != $version
                     OR $existing.dependency_manifest != $dependency_manifest
                     OR ($existing.source_metadata IS NOT NONE AND $existing.source_metadata != $metadata) {
                     THROW 'memory input identity was reused with different evidence or authorship';
                 };
             } ELSE {
                 IF !$policy.enabled { THROW 'remembering is paused'; };
                 IF array::len((SELECT id FROM episode WHERE source_family = $family
                     AND (revoked_at IS NOT NONE OR raw_withdrawn_at IS NOT NONE) LIMIT 1)) > 0 { THROW 'this source was withdrawn'; };
                 UPDATE meta:remembering SET revision += 1;
                 CREATE ONLY $episode CONTENT $data;
                 UPDATE $episode SET origin = $origin, permission_scope = 'private', source_version = $version,
                     source_family = $family, source_metadata = $metadata, admitted_at = $created,
                     dependency_manifest = $dependency_manifest, dependency_complete = $dependency_complete,
                     dependency_sources = IF $dependency_complete { $dependency_sources } ELSE { [] },
                     dependency_facts = IF $dependency_complete { $dependency_facts } ELSE { [] };
                 IF $extract {
                     CREATE ONLY $job SET episode = $episode, state = 'pending', attempts = 0,
                         generation = $policy.generation, retry_at = 0, lease_until = 0,
                         created_at = $created, fact_ids = [], cursor = 0, session_id = $data.session_id;
                 };
             };
             RETURN (SELECT id, dependency_complete FROM ONLY $episode);
             COMMIT TRANSACTION;"].concat()), bindings,
        ).await?)?;
        admitted
            .map(|admitted| {
                tracing::debug!(
                    helper = !extract,
                    dependency_complete = admitted.dependency_complete,
                    "source admitted"
                );
                sovra_core::MemorySourceReceipt {
                    source_id: EpisodeId(admitted.id.to_sql()),
                    version,
                    has_processing_job: extract,
                }
            })
            .ok_or_else(|| MemoryError::Backend("source admission returned no episode".into()))
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
                 WHERE $policy.enabled AND generation = $policy.generation AND attempts < $attempts AND episode.revoked_at IS NONE
                 AND ((state = 'pending' AND retry_at <= $now) OR (state = 'processing' AND lease_until <= $now))
                 AND array::len((SELECT id FROM memory_job
                     WHERE session_id = $parent.session_id AND state IN ['pending', 'processing']
                     AND (created_at < $parent.created_at OR (created_at = $parent.created_at AND id < $parent.id)) LIMIT 1)) = 0
                 ORDER BY created_at ASC, id ASC LIMIT 1)[0].id;
             IF $next != NONE {

                 UPDATE $next SET state = 'processing', attempts += 1, token = rand::ulid(), lease_until = $lease;
             };
             RETURN IF $next = NONE { NONE } ELSE {
                 SELECT episode.* AS episode, token, generation, attempts, lease_until, cursor, extracted_json, (SELECT VALUE revision FROM ONLY meta:memory_facts) AS revision FROM ONLY $next
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
        let mut persisted_claim = claim.clone();
        persisted_claim.episode = source.clone();
        validate_memory_batch(&persisted_claim.chunk()?, &candidates)?;
        let next_cursor = sovra_core::memory_chunk_end(&source.content, claim.cursor)?;
        let prior = self.read_job(&source.id).await?;
        let mut accumulated = prior.candidates;
        accumulated.extend(candidates.clone());
        let accumulated = serde_json::to_string(&accumulated)
            .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
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
        // Empty extraction only advances the job. Revoking concurrent recall in
        // that case can hide valid details from a question asked just afterwards.
        let sql = format!("BEGIN TRANSACTION;
            LET $job = (SELECT * FROM ONLY $job_id);
            LET $policy = (SELECT * FROM ONLY meta:remembering);
            IF ($job.state = 'complete' OR $job.cursor > $cursor) AND $job.token = $lease_token AND $job.fingerprint = $fingerprint {{
                RETURN $job.fact_ids;
            }} ELSE {{
                IF !$policy.enabled OR $job.episode.revoked_at IS NOT NONE OR $job.generation != $generation OR $policy.generation != $generation
                    OR $job.state != 'processing' OR $job.token != $lease_token OR $job.lease_until <= $now OR $job.cursor != $cursor {{
                    THROW 'memory lease is stale or remembering is paused';
                }};
                IF (SELECT VALUE revision FROM ONLY meta:memory_facts) != $revision {{ THROW 'memory reconciliation is stale'; }};
                IF array::len($writes) > 0 {{ UPDATE meta:memory_facts SET revision += 1; }};

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
                    IF (SELECT VALUE stopped_at FROM ONLY $selected) IS NOT NONE {{
                        UPDATE $write.source SET revoked_at = $now;
                    }};
                    LET $saved = (SELECT created_at, valid_at FROM ONLY $selected);
                    UPSERT type::record('memory_alias', type::string($selected) + ':' + $write.alias_key)
                        SET fact = $selected, content = $write.data.content, entities = $write.data.entities,
                            created_at = $saved.created_at, valid_at = $saved.valid_at;
                    UPDATE $job_id SET fact_ids = array::union(fact_ids, [$selected]);
                }};
                UPDATE $job_id SET state = IF $finished {{ 'complete' }} ELSE {{ 'pending' }},
                    cursor = $next_cursor, attempts = IF $finished {{ attempts }} ELSE {{ 0 }}, retry_at = 0,
                    fingerprint = $fingerprint, error = NONE, result_json = $accumulated, extractor = $model, extracted_json = NONE;
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
                    accumulated,
                    cursor: claim.cursor,
                    next_cursor,
                    finished: next_cursor == source.content.len(),
                    revision: claim.revision,
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

    pub(super) async fn retry_failed(&self) -> Result<usize, MemoryError> {
        let count: Option<usize> = returned(
            self.query(
                "BEGIN TRANSACTION;
             LET $policy = (SELECT * FROM ONLY meta:remembering);
             IF !$policy.enabled { THROW 'Turn on remembering before retrying messages.'; };
             LET $failed = (SELECT id, created_at FROM memory_job WHERE state = 'failed' AND episode.revoked_at IS NONE
                     ORDER BY created_at ASC, id ASC LIMIT $bound);
             UPDATE meta:remembering SET revision += 1;
             FOR $job IN $failed {
                 UPDATE $job.id SET state = 'pending', attempts = 0, error = NONE,
                     retry_at = 0, lease_until = 0, token = NONE, generation = $policy.generation;
             };
             RETURN array::len($failed);
             COMMIT TRANSACTION;",
                ("bound", MAX_PENDING_MEMORIES),
            )
            .await?,
        )?;
        count.ok_or_else(|| MemoryError::Backend("Retry returned no capture count.".into()))
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
    extracted_json: Option<String>,
    cursor: usize,
    revision: u64,
    episode: EpisodeRow,
    token: String,
    generation: u64,
    attempts: u32,
    lease_until: i64,
}
impl ClaimRow {
    fn into_claim(self) -> Result<MemoryClaim, MemoryError> {
        Ok(MemoryClaim {
            extracted: self
                .extracted_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| MemoryError::Backend(error.to_string()))?,
            cursor: self.cursor,
            revision: self.revision,
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
struct ExtractionBindings {
    job_id: RecordId,
    generation: u64,
    lease_token: String,
    now: i64,
    cursor: usize,
    payload: String,
}

#[derive(Debug, SurrealValue)]
struct FinishBindings {
    accumulated: String,
    cursor: usize,
    next_cursor: usize,
    finished: bool,
    revision: u64,
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

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    async fn source(store: &SurrealStore) -> EpisodeId {
        store
            .enqueue_memory(MemoryInput {
                key: "fixture".into(),
                episode: sovra_core::NewEpisode {
                    session_id: "queue-fence".into(),
                    kind: sovra_core::EpisodeKind::UserMessage,
                    content: "Fixture wording.".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap()
    }
    async fn fence(store: &SurrealStore) -> u64 {
        store
            .query(
                "SELECT VALUE revision FROM ONLY meta:remembering;",
                surrealdb::types::Object::default(),
            )
            .await
            .unwrap()
            .take::<Option<u64>>(0)
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn independent_queue_work_does_not_rewrite_the_shared_policy_fence() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        source(&store).await;
        let revision = fence(&store).await;
        let recall_revision = store.memory_revision().await.unwrap();
        let first = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
        assert_eq!(
            fence(&store).await,
            revision,
            "claiming an existing job must not contend with unrelated jobs on policy metadata"
        );
        store
            .save_memory_extraction(
                &first,
                &MemoryExtractedChunk {
                    candidates: vec![],
                    model: "fixture".into(),
                },
                Timestamp(2_001),
            )
            .await
            .unwrap();
        assert_eq!(fence(&store).await, revision);
        store
            .fail_memory(&first, "fixture", Timestamp(2_002))
            .await
            .unwrap();
        assert_eq!(fence(&store).await, revision);
        let resumed = store
            .claim_memory(Timestamp(20_000))
            .await
            .unwrap()
            .unwrap();
        store
            .complete_memory(&resumed, vec![], "fixture", Timestamp(20_001))
            .await
            .unwrap();
        assert_eq!(fence(&store).await, revision);
        assert_eq!(
            store.memory_revision().await.unwrap(),
            recall_revision,
            "finishing without any facts must not invalidate an unrelated recall"
        );
    }

    #[tokio::test]
    async fn pinned_engine_rejects_job_writes_from_before_pause_or_source_withdrawal() {
        for withdraw in [false, true] {
            let store = SurrealStore::open(crate::StoreConfig::in_memory())
                .await
                .unwrap();
            let source = source(&store).await;
            let claim = store.claim_memory(Timestamp(2_000)).await.unwrap().unwrap();
            let tx = store.db.clone().begin().await.unwrap();
            let state: Option<String> = tx
                .query("SELECT VALUE state FROM ONLY $job;")
                .bind(("job", job_id(&source).unwrap()))
                .await
                .unwrap()
                .take(0)
                .unwrap();
            assert_eq!(state.as_deref(), Some("processing"));
            if withdraw {
                store
                    .stop_memory_source(&source, Timestamp(2_001))
                    .await
                    .unwrap();
            } else {
                store.set_memory_enabled(false).await.unwrap();
            }
            // Simulate the job write shared by claim/stage/complete/fail. No policy
            // write is needed: revocation and pause both cancel this same job row.
            let written = tx
                .query("UPDATE $job SET state = 'processing', extracted_json = 'stale';")
                .bind(("job", job_id(&source).unwrap()))
                .await;
            let rejected = match written {
                Ok(response) => response.check().is_err(),
                Err(_) => true,
            };
            if rejected {
                let _ = tx.cancel().await;
            } else {
                assert!(
                    tx.commit().await.is_err(),
                    "a stale job write must conflict with cancellation"
                );
            }
            assert_eq!(
                store.memory_job(&source).await.unwrap().state,
                MemoryJobState::Cancelled
            );
            assert!(
                store
                    .save_memory_extraction(
                        &claim,
                        &MemoryExtractedChunk {
                            candidates: vec![],
                            model: "fixture".into()
                        },
                        Timestamp(2_002)
                    )
                    .await
                    .is_err()
            );
        }
    }
}
