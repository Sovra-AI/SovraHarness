//! Owned incremental memory embeddings and query vectors using the shared provider transport.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use async_channel::Sender;
use sovra_core::{
    Event, MEMORY_EMBEDDING_BATCH, MemoryCursor, MemoryError, MemoryEvent, MemoryListQuery,
    MemorySearch, MemorySemanticPhase, MemorySemanticSetting, MemorySemanticState,
    MemorySemanticStatus, MemoryStore, MemoryVector, Timestamp,
    file_index::semantic::TextEmbedder as _, models::ModelDestination,
};
use sovra_providers::{HttpEmbedder, ModelRepository};
use tokio::sync::watch;

use crate::remembering::MemoryControl;

pub(crate) struct SemanticRuntime {
    pub(crate) service: Option<Arc<SemanticMemory>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub(crate) struct SemanticMemory {
    store: Arc<dyn MemoryStore>,
    repository: ModelRepository,
    control: MemoryControl,
    blocked: AtomicU64,
    changes: watch::Sender<u64>,
}

impl SemanticRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn MemoryStore>>,
        repository: Option<ModelRepository>,
        control: MemoryControl,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (Some(store), Some(repository)) = (store, repository) else {
            return Self {
                service: None,
                task: None,
            };
        };
        let service = Arc::new(SemanticMemory {
            store,
            repository,
            control,
            blocked: AtomicU64::new(0),
            changes: watch::channel(0).0,
        });
        let task = handle.spawn(run(service.clone(), events));
        Self {
            service: Some(service),
            task: Some(task),
        }
    }

    pub(crate) async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for SemanticRuntime {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl SemanticMemory {
    pub(crate) fn revoke(&self) -> u64 {
        self.control.invalidate_recall();
        let mut token = 0;
        self.changes.send_modify(|value| {
            *value = value.wrapping_add(1).max(1);
            token = *value;
            self.blocked.store(token, Ordering::SeqCst);
        });
        token
    }

    pub(crate) async fn configure(
        &self,
        target: Option<ModelDestination>,
        token: u64,
    ) -> Result<MemorySemanticState, MemoryError> {
        if let Some(target) = &target {
            self.embedder(target).await?;
        }
        let state = self.store.configure_memory_semantics(target).await?;
        // A queued later selection owns revocation until its own write completes.
        let _ = self
            .blocked
            .compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst);
        self.control.invalidate_recall();
        self.changes
            .send_modify(|value| *value = value.wrapping_add(1));
        Ok(state)
    }

    fn allowed(&self) -> bool {
        self.control.enabled() && self.blocked.load(Ordering::SeqCst) == 0
    }

    async fn embedder(&self, target: &ModelDestination) -> Result<HttpEmbedder, MemoryError> {
        let repository = self.repository.clone();
        let target = target.clone();
        tokio::task::spawn_blocking(move || HttpEmbedder::from_repository(&repository, &target))
            .await
            .map_err(|error| MemoryError::Backend(error.to_string()))?
            .map_err(|error| MemoryError::Backend(error.to_string()))
    }

    pub(crate) async fn search(
        &self,
        query: &MemorySearch,
        now: Timestamp,
    ) -> Result<Vec<sovra_core::Fact>, MemoryError> {
        if !self.allowed() {
            return Ok(Vec::new());
        }
        let Some(setting) = self.store.memory_semantics().await?.setting else {
            return Ok(Vec::new());
        };
        if setting.dimension.is_none() {
            return Ok(Vec::new());
        }
        let revision = *self.changes.borrow();
        let embedder = self.embedder(&setting.target).await?;
        if !self.allowed() || revision != *self.changes.borrow() {
            return Ok(Vec::new());
        }
        let vectors = embedder
            .embed(std::slice::from_ref(&query.text))
            .await
            .map_err(|error| MemoryError::Backend(error.to_string()))?;
        if !self.allowed() || revision != *self.changes.borrow() {
            return Ok(Vec::new());
        }
        let vector = vectors
            .first()
            .ok_or_else(|| MemoryError::Backend("Missing memory query vector.".into()))?;
        let hits = self
            .store
            .semantic_memories(&setting, vector, query.clone(), now)
            .await?;
        if !self.allowed() || revision != *self.changes.borrow() {
            return Ok(Vec::new());
        }
        Ok(hits.into_iter().map(|hit| hit.fact).collect())
    }

    async fn batch(
        &self,
        setting: &MemorySemanticSetting,
        cursor: Option<MemoryCursor>,
    ) -> Result<Option<MemoryCursor>, MemoryError> {
        let page = self
            .store
            .list_memories(MemoryListQuery {
                limit: MEMORY_EMBEDDING_BATCH,
                before: cursor,
                include_history: true,
                ..Default::default()
            })
            .await?;
        let ids = page
            .facts
            .into_iter()
            .map(|fact| fact.id)
            .collect::<Vec<_>>();
        let inputs = self.store.memory_embedding_inputs(setting, &ids).await?;
        if inputs.is_empty() {
            return Ok(page.next);
        }
        let policy = self.store.memory_policy().await?;
        let revision = *self.changes.borrow();
        let recall_revision = self.control.recall_revision();
        let pause = self.control.pause_epoch();
        let embedder = self.embedder(&setting.target).await?;
        let valid = || {
            self.allowed()
                && revision == *self.changes.borrow()
                && pause == self.control.pause_epoch()
                && recall_revision == self.control.recall_revision()
        };
        if !policy.enabled || !valid() {
            return Err(MemoryError::InvalidInput(
                "Remembering permission changed.".into(),
            ));
        }
        let texts: Vec<_> = inputs.iter().map(|input| input.text.clone()).collect();
        let vectors = embedder
            .embed(&texts)
            .await
            .map_err(|error| MemoryError::Backend(error.to_string()))?;
        if !valid() {
            return Err(MemoryError::InvalidInput(
                "Remembering permission changed.".into(),
            ));
        }
        let vectors = inputs
            .into_iter()
            .zip(vectors)
            .map(|(input, vector)| MemoryVector {
                fact_id: input.fact_id,
                vector,
            })
            .collect();
        self.store
            .commit_memory_vectors(setting, policy, vectors)
            .await?;
        Ok(page.next)
    }
}

async fn publish(
    events: &Sender<Event>,
    previous: &mut Option<MemorySemanticStatus>,
    state: MemorySemanticState,
    phase: MemorySemanticPhase,
) {
    let status = MemorySemanticStatus { state, phase };
    if previous.as_ref() != Some(&status) {
        let _ = events
            .send(Event::Memory(MemoryEvent::Semantic(status.clone())))
            .await;
        *previous = Some(status);
    }
}

async fn run(service: Arc<SemanticMemory>, events: Sender<Event>) {
    let mut changes = service.changes.subscribe();
    let mut settings = service.control.subscribe();
    let mut reviews = service.control.subscribe_recall();
    let mut cursor = None;
    let mut failure = None::<(u64, String)>;
    let mut previous = None;
    loop {
        let state = match service.store.memory_semantics().await {
            Ok(state) => state,
            Err(error) => {
                publish(
                    &events,
                    &mut previous,
                    MemorySemanticState::default(),
                    MemorySemanticPhase::Failed(error.to_string()),
                )
                .await;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        if failure
            .as_ref()
            .is_some_and(|(generation, _)| *generation != state.generation)
        {
            failure = None;
            cursor = None;
        }
        let phase = if service.blocked.load(Ordering::SeqCst) != 0 {
            MemorySemanticPhase::Failed("Search requests are stopped while settings change. Choose the search helper again if the change failed.".into())
        } else if state.setting.is_none() {
            MemorySemanticPhase::Disabled
        } else if !service.control.enabled() {
            MemorySemanticPhase::Paused
        } else if let Some((_, message)) = &failure {
            MemorySemanticPhase::Failed(message.clone())
        } else if previous.as_ref().is_some_and(|status| {
            status.state.generation == state.generation
                && status.phase == MemorySemanticPhase::Ready
        }) {
            MemorySemanticPhase::Ready
        } else {
            MemorySemanticPhase::Updating
        };
        publish(&events, &mut previous, state.clone(), phase.clone()).await;
        if let Some(setting) = &state.setting
            && matches!(
                phase,
                MemorySemanticPhase::Updating | MemorySemanticPhase::Ready
            )
        {
            let result = tokio::select! {
                biased;
                _ = changes.changed() => { cursor = None; continue; }
                _ = settings.changed() => { cursor = None; continue; }
                _ = reviews.changed() => { cursor = None; continue; }
                result = tokio::time::timeout(Duration::from_secs(30), service.batch(setting, cursor.clone())) => result.unwrap_or_else(|_| Err(MemoryError::Backend("The search helper timed out. Choose it again to retry.".into()))),
            };
            match result {
                Ok(next) => {
                    cursor = next;
                    let current = service
                        .store
                        .memory_semantics()
                        .await
                        .unwrap_or(state.clone());
                    if !service.allowed() || current.generation != state.generation {
                        cursor = None;
                        continue;
                    }
                    let phase = if cursor.is_some() {
                        MemorySemanticPhase::Updating
                    } else {
                        MemorySemanticPhase::Ready
                    };
                    publish(&events, &mut previous, current, phase).await;
                }
                Err(error) => {
                    let message = format!(
                        "Search by meaning is unavailable: {error} Matching words still work. Choose the search helper again to retry."
                    );
                    failure = Some((state.generation, message.clone()));
                    publish(
                        &events,
                        &mut previous,
                        state,
                        MemorySemanticPhase::Failed(message),
                    )
                    .await;
                }
            }
        }
        let delay = if cursor.is_some() && failure.is_none() {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(2)
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _ = changes.changed() => { cursor = None; }
            _ = settings.changed() => { cursor = None; }
            _ = reviews.changed() => { cursor = None; }
        }
    }
}
