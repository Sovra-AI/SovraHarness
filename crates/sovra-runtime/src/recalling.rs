//! Scoped per-turn recall. Queries and evidence reads have one deadline and pause epoch.

use crate::remembering::MemoryControl;
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
    let mut legs = Vec::new();
    for term in query.terms() {
        let hits = store.recall(query.query(term, now)).await?;
        legs.push(
            hits.into_iter()
                .filter(|hit| query.accepts(&hit.fact, now))
                .map(|hit| query.as_known(hit.fact))
                .collect(),
        );
    }
    Ok(legs)
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
    async fn retrieve(
        &self,
        query: MemorySearch,
        deadline: Instant,
    ) -> Result<MemoryContext, MemoryError> {
        query.validate()?;
        let policy = self.store.memory_policy().await?;
        if !policy.enabled || !self.allowed() {
            return Ok(MemoryContext::default());
        }
        let now = crate::now();
        let mut legs = keyword_candidates(self.store.as_ref(), &query, now).await?;
        if !self.allowed() {
            return Ok(MemoryContext::default());
        }
        let mut context = MemoryContext::default();
        if let Some(semantics) = &self.semantics {
            match tokio::time::timeout(Duration::from_millis(900), semantics.search(&query, now))
                .await
            {
                Ok(Ok(facts)) => legs.push(facts),
                Ok(Err(_)) | Err(_) => {
                    context.notice = Some(
                        "Search by meaning was unavailable; matching words were still checked."
                            .into(),
                    );
                }
            }
        }
        let mut seeds = Vec::new();
        let mut deferred = Vec::new();
        for hit in fuse_rrf(legs, 24) {
            if !self.allowed() {
                return Ok(MemoryContext::default());
            }
            if seeds.len() == MAX_MEMORY_PRIMARY_ITEMS {
                deferred.push(hit.fact);
            } else if let Some(id) = &hit.fact.source {
                let source = self.store.episode(id).await?;
                if context.push(&hit.fact, &source) {
                    seeds.push(hit.fact);
                }
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
            if let Some(id) = &fact.source {
                let source = self.store.episode(id).await?;
                context.push(&fact, &source);
            }
        }
        let final_policy = self.store.memory_policy().await?;
        if !self.allowed() || !final_policy.enabled || final_policy.generation != policy.generation
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
        let mut related = Vec::new();
        for fact in search.related_facts(seeds, candidates, now) {
            if !self.allowed() {
                return Ok(Vec::new());
            }
            if let Some(id) = &fact.source {
                let source = self.store.episode(id).await?;
                related.push((fact, source));
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
            result = tokio::time::timeout_at(deadline, self.retrieve(query, deadline)) => {
                result.unwrap_or_else(|_| Err(MemoryError::Backend("Saved details took too long to check.".into())))
            }
        }
    }
}
