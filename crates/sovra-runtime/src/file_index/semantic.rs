//! Owned background embedding work, explicit consent and hybrid retrieval.

use async_channel::Sender;
use sovra_core::{
    Event,
    file_index::{
        FileIndex, IndexError, IndexEvent, IndexPassage, IndexQuery,
        semantic::{
            EmbeddingTarget, SearchReport, SemanticIndex, SemanticSetting, SemanticStatus,
            TextEmbedder, fuse_passages,
        },
    },
};
use sovra_providers::{HttpEmbedder, ModelRepository};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub(super) struct Service {
    store: Arc<dyn FileIndex>,
    repository: ModelRepository,
    blocked: Mutex<BTreeSet<String>>,
    changed: tokio::sync::watch::Sender<u64>,
}

pub(super) struct Runtime {
    pub(super) service: Arc<Service>,
    commands: Sender<(String, Option<EmbeddingTarget>)>,
    task: tokio::task::JoinHandle<()>,
}

impl Runtime {
    pub(super) fn start(
        store: Arc<dyn FileIndex>,
        repository: ModelRepository,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let service = Arc::new(Service {
            store,
            repository,
            blocked: Mutex::new(BTreeSet::new()),
            changed: tokio::sync::watch::channel(0).0,
        });
        let (commands, receiver) = async_channel::bounded::<(String, Option<EmbeddingTarget>)>(16);
        let task = handle.spawn({
            let service = service.clone();
            async move {
                let mut changes = service.changed.subscribe();
                let mut errors = BTreeMap::<u64,String>::new();
                loop {
                    let work = cycle(service.clone(), &events, &mut errors);
                    tokio::select! {
                        biased;
                        command = receiver.recv() => {
                            let Ok((root, target)) = command else { break; };
                            let result = configure(service.clone(), root.clone(), target).await;
                            if let Err(error) = result {
                                let _ = events.try_send(Event::FileIndex(IndexEvent::Error(error.to_string())));
                            } else {
                                service.blocked.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&root);
                                errors.clear();
                            }
                            changes.borrow_and_update();
                        }
                        _ = changes.changed() => {}
                        () = work => {
                            tokio::select! {
                                () = tokio::time::sleep(Duration::from_secs(2)) => {},
                                _ = changes.changed() => {},
                            }
                        }
                    }
                }
            }
        });
        Self {
            service,
            commands,
            task,
        }
    }

    pub(super) fn revoke(&self, root: &str) {
        self.service
            .blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(root.to_owned());
        self.service.changed.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub(super) fn configure(
        &self,
        root: String,
        target: Option<EmbeddingTarget>,
    ) -> Result<(), IndexError> {
        self.revoke(&root);
        self.commands.try_send((root, target)).map_err(|_| {
            IndexError("Too many semantic settings changes. Wait briefly and retry.".into())
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn capability(store: &dyn FileIndex) -> Result<&dyn SemanticIndex, IndexError> {
    store
        .semantic()
        .ok_or_else(|| IndexError("This index does not support semantic search.".into()))
}

async fn configure(
    service: Arc<Service>,
    root: String,
    target: Option<EmbeddingTarget>,
) -> Result<(), IndexError> {
    super::retrieve(move || {
        if let Some(target) = &target {
            HttpEmbedder::from_repository(&service.repository, target)?;
        }
        capability(service.store.as_ref())?.configure(&root, target.as_ref())
    })
    .await
}

impl Service {
    fn allowed(&self, root: &str) -> bool {
        !self
            .blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(root)
    }

    async fn settings(self: &Arc<Self>) -> Result<Vec<SemanticSetting>, IndexError> {
        let store = self.store.clone();
        super::retrieve(move || capability(store.as_ref())?.settings()).await
    }

    async fn embedder(
        self: &Arc<Self>,
        setting: &SemanticSetting,
    ) -> Result<HttpEmbedder, IndexError> {
        let service = self.clone();
        let setting = setting.clone();
        super::retrieve(move || {
            if !service.allowed(&setting.root_id) {
                return Err(IndexError(
                    "Semantic search was disabled for this folder.".into(),
                ));
            }
            capability(service.store.as_ref())?.status(&setting)?;
            HttpEmbedder::from_repository(&service.repository, &setting.target)
        })
        .await
    }

    pub(super) async fn search(
        self: &Arc<Self>,
        query: IndexQuery,
    ) -> Result<SearchReport, IndexError> {
        query.validate()?;
        let mut changes = self.changed.subscribe();
        let store = self.store.clone();
        let keyword_query = query.clone();
        let keywords = super::retrieve(move || store.search(&keyword_query)).await?;
        let mut notices = vec![];
        let mut legs = vec![keywords];
        let semantic = tokio::select! {
            biased;
            _ = changes.changed() => Err(IndexError("Semantic settings changed during this search; keyword results are shown.".into())),
            result = tokio::time::timeout(Duration::from_secs(10), self.semantic_legs(&query)) => result.unwrap_or_else(|_| Err(IndexError("Semantic search timed out; keyword results are shown.".into()))),
        };
        match semantic {
            Ok((found, messages)) => {
                legs.extend(found);
                notices.extend(messages);
            }
            Err(error) => notices.push(error.to_string()),
        }
        let passages = fuse_passages(legs, query.limit);
        // Network work may outlive a scan or removal. Resolve every final citation
        // against current grants/versions immediately before returning it.
        let store = self.store.clone();
        let passages = super::retrieve(move || {
            Ok(passages
                .into_iter()
                .filter_map(|p| store.read(&p.reference).ok())
                .collect())
        })
        .await?;
        Ok(SearchReport { passages, notices })
    }

    async fn semantic_legs(
        self: &Arc<Self>,
        query: &IndexQuery,
    ) -> Result<(Vec<Vec<IndexPassage>>, Vec<String>), IndexError> {
        let mut legs = vec![];
        let mut notices = vec![];
        for setting in self.settings().await? {
            if !self.allowed(&setting.root_id)
                || (!query.roots.is_empty() && !query.roots.contains(&setting.root_id))
            {
                continue;
            }
            match self.semantic_leg(&setting, query).await {
                Ok((passages, status)) => {
                    if status.embedded < status.total {
                        notices.push(format!("Semantic coverage for folder {}: {} of {} unique passages. Keyword search also runs.", setting.root_id,status.embedded,status.total));
                    }
                    legs.push(passages);
                }
                Err(error) => notices.push(format!(
                    "Semantic search unavailable for folder {}: {} Keyword search still runs.",
                    setting.root_id, error
                )),
            }
        }
        Ok((legs, notices))
    }

    async fn semantic_leg(
        self: &Arc<Self>,
        setting: &SemanticSetting,
        query: &IndexQuery,
    ) -> Result<(Vec<IndexPassage>, SemanticStatus), IndexError> {
        let embedder = self.embedder(setting).await?;
        let vectors = embedder.embed(std::slice::from_ref(&query.text)).await?;
        let vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| IndexError("Missing query embedding.".into()))?;
        let service = self.clone();
        let setting = setting.clone();
        let query = query.clone();
        super::retrieve(move || {
            if !service.allowed(&setting.root_id) {
                return Err(IndexError("Semantic search permission changed.".into()));
            }
            let index = capability(service.store.as_ref())?;
            Ok((
                index.nearest(&setting, &query, &vector)?,
                index.status(&setting)?,
            ))
        })
        .await
    }
}

async fn cycle(service: Arc<Service>, events: &Sender<Event>, errors: &mut BTreeMap<u64, String>) {
    let Ok(settings) = service.settings().await else {
        return;
    };
    errors.retain(|id, _| settings.iter().any(|s| s.id == *id));
    let mut statuses = vec![];
    // Publish consent and progress before the first HTTP request so Disable is
    // available even while a cold model is loading on the server.
    for setting in &settings {
        if let Ok(mut status) = status(service.clone(), setting.clone()).await {
            status.message = errors.get(&setting.id).cloned();
            if !service.allowed(&setting.root_id) {
                status.message = Some("Semantic requests are paused in this session. Choose the model again or disable semantic search.".into());
            }
            statuses.push(status);
        }
    }
    let _ = events.try_send(Event::FileIndex(IndexEvent::Semantic(statuses.clone())));
    for setting in settings {
        if !service.allowed(&setting.root_id) {
            continue;
        }
        // A failed batch is not repeatedly sent to a possibly paid service.
        if let std::collections::btree_map::Entry::Vacant(entry) = errors.entry(setting.id) {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                batch(service.clone(), setting.clone()),
            )
            .await
            .unwrap_or_else(|_| {
                Err(IndexError(
                    "Embedding timed out. Re-enable semantic search to rebuild.".into(),
                ))
            });
            if let Err(error) = result {
                entry.insert(error.to_string());
            }
        }
        if let Ok(mut current) = status(service.clone(), setting.clone()).await {
            current.message = errors.get(&setting.id).cloned();
            if let Some(previous) = statuses.iter_mut().find(|s| s.setting.id == setting.id) {
                *previous = current;
            }
            let _ = events.try_send(Event::FileIndex(IndexEvent::Semantic(statuses.clone())));
        }
    }
}

async fn status(
    service: Arc<Service>,
    setting: SemanticSetting,
) -> Result<SemanticStatus, IndexError> {
    super::retrieve(move || capability(service.store.as_ref())?.status(&setting)).await
}

async fn batch(service: Arc<Service>, setting: SemanticSetting) -> Result<(), IndexError> {
    let store = service.store.clone();
    let current = setting.clone();
    let inputs = super::retrieve(move || capability(store.as_ref())?.pending(&current)).await?;
    if inputs.is_empty() {
        return Ok(());
    }
    let embedder = service.embedder(&setting).await?;
    let texts: Vec<_> = inputs.iter().map(|i| i.text.clone()).collect();
    let vectors = embedder.embed(&texts).await?;
    super::retrieve(move || {
        if !service.allowed(&setting.root_id) {
            return Err(IndexError("Semantic indexing was disabled.".into()));
        }
        capability(service.store.as_ref())?.commit(&setting, &inputs, &vectors)
    })
    .await
}
