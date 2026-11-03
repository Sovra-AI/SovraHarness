//! Scoped per-turn recall. Queries and evidence reads have one deadline and pause epoch.

use crate::remembering::MemoryControl;
use futures::{StreamExt as _, TryStreamExt as _, stream};
use sovra_core::{
    Episode, Fact, MAX_MEMORY_PRIMARY_ITEMS, MemoryContext, MemoryError, MemoryReader,
    MemorySearch, MemoryStore, Timestamp, fuse_rrf,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

const RECALL_DEADLINE: Duration = Duration::from_secs(2);
const RELATED_DEADLINE: Duration = Duration::from_millis(200);
const FINAL_POLICY_RESERVE: Duration = Duration::from_millis(100);
const EVIDENCE_RESERVE: Duration = Duration::from_millis(400);
const KEYWORD_READ_CONCURRENCY: usize = 4;

async fn read_before<T>(
    deadline: Instant,
    read: impl Future<Output = Result<T, MemoryError>>,
) -> Result<T, MemoryError> {
    tokio::time::timeout_at(deadline, read)
        .await
        .map_err(|_| MemoryError::Backend("Saved evidence took too long to check.".into()))?
}

struct Reader {
    semantics: Option<Arc<crate::memory_semantic::SemanticMemory>>,
    store: Arc<dyn MemoryStore>,
    control: MemoryControl,
    epoch: u64,
    revision: u64,
}

/// Both recall and extraction use the same bounded lexical matching and temporal rules.
/// Callers own cancellation and the total IO deadline.
pub(crate) async fn keyword_candidates(
    store: &dyn MemoryStore,
    query: &MemorySearch,
    now: Timestamp,
) -> Result<Vec<Vec<Fact>>, MemoryError> {
    // Preserve term order for deterministic fusion while overlapping independent
    // IO. Dropping this stream also drops every in-flight read on pause/deadline.
    stream::iter(query.terms())
        .map(|term| async move {
            let hits = store.recall(query.query(term, now)).await?;
            Ok(hits
                .into_iter()
                .filter(|hit| query.accepts(&hit.fact, now))
                .map(|hit| query.as_known(hit.fact))
                .collect())
        })
        .buffered(KEYWORD_READ_CONCURRENCY)
        .try_collect()
        .await
}

pub(crate) fn reader(
    store: Option<&Arc<dyn MemoryStore>>,
    control: &MemoryControl,
    semantics: Option<Arc<crate::memory_semantic::SemanticMemory>>,
) -> Option<Arc<dyn MemoryReader>> {
    if !control.enabled() {
        return None;
    }
    store.map(|store| {
        Arc::new(Reader {
            semantics,
            store: store.clone(),
            control: control.clone(),
            epoch: control.pause_epoch(),
            revision: control.recall_revision(),
        }) as Arc<dyn MemoryReader>
    })
}
impl Reader {
    fn allowed(&self) -> bool {
        self.control.enabled()
            && self.control.pause_epoch() == self.epoch
            && self.control.recall_revision() == self.revision
    }
    async fn retrieve_batch(
        &self,
        queries: Vec<MemorySearch>,
        deadline: Instant,
    ) -> Result<Vec<MemoryContext>, MemoryError> {
        let policy = self.store.memory_policy().await?;
        let revision = self.store.memory_revision().await?;
        let count = queries.len();
        if !policy.enabled || !self.allowed() {
            return Ok(vec![MemoryContext::default(); count]);
        }
        let queries_deadline = deadline - FINAL_POLICY_RESERVE;
        let contexts = stream::iter(queries)
            .map(|query| async move {
                read_before(
                    queries_deadline,
                    self.retrieve(query, queries_deadline, false),
                )
                .await
                .unwrap_or_else(|_| MemoryContext::unavailable())
            })
            .buffered(sovra_core::MAX_MEMORY_SEARCHES)
            .collect()
            .await;
        // A fast query may finish before another query observes a correction or
        // revocation. Validate the entire batch again before releasing any result.
        let final_policy = self.store.memory_policy().await?;
        if !self.allowed()
            || !final_policy.enabled
            || final_policy.generation != policy.generation
            || self.store.memory_revision().await? != revision
        {
            return Ok(vec![MemoryContext::default(); count]);
        }
        Ok(contexts)
    }
    async fn retrieve(
        &self,
        query: MemorySearch,
        deadline: Instant,
        include_background: bool,
    ) -> Result<MemoryContext, MemoryError> {
        query.validate()?;
        let policy = self.store.memory_policy().await?;
        if !policy.enabled || !self.allowed() {
            return Ok(MemoryContext::default());
        }
        let now = crate::now();
        let revision = self.store.memory_revision().await?;
        let semantic = async {
            match &self.semantics {
                Some(service) => {
                    tokio::time::timeout(Duration::from_millis(900), service.search(&query, now))
                        .await
                        .map_err(|_| ())
                        .and_then(|result| result.map_err(|_| ()))
                }
                None => Ok(Vec::new()),
            }
        };
        let read_deadline = deadline - FINAL_POLICY_RESERVE - EVIDENCE_RESERVE;
        let background = async {
            match include_background
                .then(|| query.background_query(now))
                .flatten()
            {
                Some(query) => self.store.recall(query).await,
                None => Ok(Vec::new()),
            }
        };
        // Each owned future keeps its result when a sibling fails or times out.
        // Leave a separate budget for provenance and the mandatory final policy check.
        let (lexical, semantic, sources, background) = tokio::join!(
            read_before(
                read_deadline,
                keyword_candidates(self.store.as_ref(), &query, now)
            ),
            tokio::time::timeout_at(read_deadline, semantic),
            read_before(
                read_deadline,
                self.store.search_memory_sources(query.clone(), now)
            ),
            read_before(read_deadline, background),
        );
        let mut context = MemoryContext::default();
        if lexical.is_err() || sources.is_err() || background.is_err() {
            context.notice = Some("Some saved evidence could not be checked. Available evidence is included; missing results do not prove a detail was never saved.".into());
        }
        let mut legs = lexical.unwrap_or_default();
        match semantic {
            Ok(Ok(facts)) => legs.push(facts),
            Ok(Err(())) | Err(_) => {
                context.notice.get_or_insert_with(|| {
                    "Search by meaning was unavailable; matching words were still checked.".into()
                });
            }
        }
        // Reserve evidence space even when the extractor omitted the relevant detail.
        for source in sources.unwrap_or_default() {
            context.push_source(source);
        }
        let hits = fuse_rrf(legs, 24);
        let background = background
            .unwrap_or_default()
            .into_iter()
            .filter(|hit| query.accepts(&hit.fact, now))
            .take(8)
            .collect::<Vec<_>>();
        let ids = hits
            .iter()
            .chain(&background)
            .filter_map(|hit| hit.fact.source.clone())
            .collect::<Vec<_>>();
        let evidence = read_before(
            deadline - FINAL_POLICY_RESERVE,
            self.store.memory_episodes(&ids),
        )
        .await;
        if evidence.is_err() {
            context.notice = Some("Some supporting records could not be checked. Only verified evidence is included; other saved details may exist.".into());
        }
        let evidence = evidence.unwrap_or_default();
        if ids
            .iter()
            .any(|id| !evidence.iter().any(|source| &source.id == id))
        {
            context.notice.get_or_insert_with(|| "Some saved details were omitted because their sources are unavailable or not permitted. They may still be available for review in Knowledge.".into());
        }
        let mut seeds = Vec::new();
        let mut deferred = Vec::new();
        for hit in hits {
            if !self.allowed() {
                return Ok(MemoryContext::default());
            }
            if seeds.len() == MAX_MEMORY_PRIMARY_ITEMS {
                deferred.push(hit.fact);
            } else if let Some(source) = evidence
                .iter()
                .find(|source| Some(&source.id) == hit.fact.source.as_ref())
                && context.push(&hit.fact, source)
            {
                seeds.push(hit.fact);
            }
        }
        // Related context is optional: a slow extra read must not discard direct matches.
        let extra_time = deadline
            .saturating_duration_since(Instant::now())
            .saturating_sub(FINAL_POLICY_RESERVE)
            .min(RELATED_DEADLINE);
        if !extra_time.is_zero()
            && let Ok(Ok(related)) =
                tokio::time::timeout(extra_time, self.related(&query, &seeds, now)).await
        {
            for (fact, source) in related {
                context.push(&fact, &source);
            }
        }
        for fact in deferred {
            if context.memories.len() == sovra_core::MAX_MEMORY_CONTEXT_ITEMS {
                break;
            }
            if !self.allowed() {
                return Ok(MemoryContext::default());
            }
            if let Some(source) = evidence
                .iter()
                .find(|source| Some(&source.id) == fact.source.as_ref())
            {
                context.push(&fact, source);
            }
        }
        for hit in background {
            if let Some(source) = evidence
                .iter()
                .find(|source| Some(&source.id) == hit.fact.source.as_ref())
            {
                context.push_background(&hit.fact, source);
            }
        }
        let final_policy = self.store.memory_policy().await?;
        if !self.allowed()
            || !final_policy.enabled
            || final_policy.generation != policy.generation
            || self.store.memory_revision().await? != revision
        {
            return Ok(MemoryContext::default());
        }
        Ok(context)
    }

    async fn related(
        &self,
        search: &MemorySearch,
        seeds: &[Fact],
        now: Timestamp,
    ) -> Result<Vec<(Fact, Episode)>, MemoryError> {
        let mut candidates = Vec::new();
        for query in search.related_queries(seeds, now) {
            if !self.allowed() {
                return Ok(Vec::new());
            }
            let hits = self.store.recall(query.clone()).await?;
            candidates.extend(
                hits.into_iter()
                    .filter(|hit| {
                        search.accepts(&hit.fact, now)
                            && query
                                .entity
                                .as_ref()
                                .is_some_and(|name| hit.fact.entities.contains(name))
                    })
                    .take(query.limit)
                    .map(|hit| hit.fact),
            );
        }
        let facts = search.related_facts(seeds, candidates, now);
        let ids = facts
            .iter()
            .filter_map(|fact| fact.source.clone())
            .collect::<Vec<_>>();
        let sources = self.store.memory_episodes(&ids).await?;
        let mut related = Vec::new();
        for fact in facts {
            if !self.allowed() {
                return Ok(Vec::new());
            }
            if let Some(source) = sources
                .iter()
                .find(|source| Some(&source.id) == fact.source.as_ref())
            {
                related.push((fact, source.clone()));
            }
        }
        Ok(related)
    }
}
#[async_trait::async_trait]
impl MemoryReader for Reader {
    async fn search(&self, query: MemorySearch) -> Result<MemoryContext, MemoryError> {
        if !self.allowed() {
            return Ok(MemoryContext::default());
        }
        let mut changes = self.control.subscribe();
        let mut edits = self.control.subscribe_recall();
        let deadline = Instant::now() + RECALL_DEADLINE;
        tokio::select! {
            biased;
            _ = changes.changed() => Ok(MemoryContext::default()),
            _ = edits.changed() => Ok(MemoryContext::default()),
            result = tokio::time::timeout_at(deadline, self.retrieve(query, deadline, true)) => {
                result.unwrap_or_else(|_| Err(MemoryError::Backend("Saved details took too long to check.".into())))
            }
        }
    }

    async fn search_batch(
        &self,
        queries: Vec<MemorySearch>,
    ) -> Result<Vec<MemoryContext>, MemoryError> {
        sovra_core::validate_memory_searches(&queries)?;
        let count = queries.len();
        if !self.allowed() {
            return Ok(vec![MemoryContext::default(); count]);
        }
        let mut changes = self.control.subscribe();
        let mut edits = self.control.subscribe_recall();
        let deadline = Instant::now() + RECALL_DEADLINE;
        tokio::select! {
            biased;
            _ = changes.changed() => Ok(vec![MemoryContext::default(); count]),
            _ = edits.changed() => Ok(vec![MemoryContext::default(); count]),
            result = read_before(deadline, self.retrieve_batch(queries, deadline)) => result,
        }
    }
}
