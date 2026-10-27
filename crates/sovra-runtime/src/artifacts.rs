//! The artifact library: saves, edits and deletes artifacts through the memory
//! store, writes copies the user asked to keep, and prepares browser views.
//! Without a store the library still works for the session, in memory.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::Sender;
use sovra_adk::ArtifactSaver;
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, artifact_id, browser_page,
};
use sovra_core::{Event, MemoryStore};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct ArtifactRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    store: Option<Arc<dyn MemoryStore>>,
    /// `None` until first loaded from the store.
    library: Mutex<Option<BTreeMap<String, Artifact>>>,
    sequence: AtomicU64,
    events: Sender<Event>,
    /// Where browser views are written; they are throwaway copies.
    view_dir: PathBuf,
}

impl ArtifactRuntime {
    pub(crate) fn new(
        store: Option<Arc<dyn MemoryStore>>,
        events: Sender<Event>,
        view_dir: PathBuf,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                library: Mutex::new(None),
                sequence: AtomicU64::new(0),
                events,
                view_dir,
            }),
        }
    }

    /// The `save_artifact` tool's persistence, for agents.
    pub(crate) fn saver(&self) -> ArtifactSaver {
        let runtime = self.clone();
        ArtifactSaver::from_fn(move |draft| {
            let runtime = runtime.clone();
            async move { runtime.save(draft).await }
        })
    }

    pub(crate) fn handle(&self, handle: &tokio::runtime::Handle, command: ArtifactCommand) {
        let runtime = self.clone();
        drop(handle.spawn(async move { runtime.run(command).await }));
    }

    async fn run(&self, command: ArtifactCommand) {
        let event = match command {
            ArtifactCommand::Load => {
                let library = self.library().await;
                ArtifactEvent::Loaded(
                    library
                        .as_ref()
                        .map_or_else(Vec::new, |l| l.values().cloned().collect()),
                )
            }
            ArtifactCommand::Save(draft) => match self.save(draft).await {
                // `save` already announced it.
                Ok(_) => return,
                Err(message) => ArtifactEvent::Error(message),
            },
            ArtifactCommand::Edit { id, title, content } => {
                let edited = {
                    let mut library = self.library().await;
                    library
                        .as_mut()
                        .and_then(|l| l.get_mut(&id))
                        .map(|artifact| {
                            artifact.edit(&title, &content, crate::now());
                            artifact.clone()
                        })
                };
                match edited {
                    Some(artifact) => {
                        self.persist(&artifact).await;
                        ArtifactEvent::Saved(artifact)
                    }
                    None => missing(),
                }
            }
            ArtifactCommand::Delete { id } => {
                let removed = self.library().await.as_mut().and_then(|l| l.remove(&id));
                if removed.is_none() {
                    missing()
                } else {
                    if let Some(store) = &self.inner.store
                        && let Err(error) = store.delete_artifact(&id, crate::now()).await
                    {
                        tracing::warn!(%error, "failed to delete artifact");
                    }
                    ArtifactEvent::Deleted { id }
                }
            }
            ArtifactCommand::Export { id, path } => match self.get(&id).await {
                Some(artifact) => match tokio::fs::write(&path, artifact.export_text()).await {
                    Ok(()) => ArtifactEvent::Exported { id, path },
                    Err(error) => {
                        tracing::warn!(%error, "failed to export artifact");
                        ArtifactEvent::Error(
                            "Sovra couldn't save the copy there. Try another folder.".into(),
                        )
                    }
                },
                None => missing(),
            },
            ArtifactCommand::Open { id } => match self.get(&id).await {
                Some(artifact) => self.write_view(&artifact).await,
                None => missing(),
            },
        };
        let _ = self.inner.events.send(Event::Artifacts(event)).await;
    }

    async fn save(&self, draft: ArtifactDraft) -> Result<Artifact, String> {
        let at = crate::now();
        let sequence = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        let artifact = Artifact::from_draft(artifact_id(at, sequence), draft, at);
        self.library()
            .await
            .get_or_insert_with(BTreeMap::new)
            .insert(artifact.id.clone(), artifact.clone());
        self.persist(&artifact).await;
        let _ = self
            .inner
            .events
            .send(Event::Artifacts(ArtifactEvent::Saved(artifact.clone())))
            .await;
        Ok(artifact)
    }

    /// Store failures are logged; the artifact stays available for this session.
    async fn persist(&self, artifact: &Artifact) {
        if let Some(store) = &self.inner.store
            && let Err(error) = store.save_artifact(artifact).await
        {
            tracing::warn!(%error, "failed to save artifact");
        }
    }

    async fn get(&self, id: &str) -> Option<Artifact> {
        self.library()
            .await
            .as_ref()
            .and_then(|l| l.get(id).cloned())
    }

    async fn library(&self) -> tokio::sync::MutexGuard<'_, Option<BTreeMap<String, Artifact>>> {
        let mut library = self.inner.library.lock().await;
        if library.is_none() {
            let loaded = match &self.inner.store {
                Some(store) => store.list_artifacts().await.unwrap_or_else(|error| {
                    tracing::warn!(%error, "failed to load artifacts");
                    Vec::new()
                }),
                None => Vec::new(),
            };
            *library = Some(loaded.into_iter().map(|a| (a.id.clone(), a)).collect());
        }
        library
    }

    async fn write_view(&self, artifact: &Artifact) -> ArtifactEvent {
        let path = self.inner.view_dir.join(format!("{}.html", artifact.id));
        let written = async {
            tokio::fs::create_dir_all(&self.inner.view_dir).await?;
            tokio::fs::write(&path, browser_page(artifact)).await
        }
        .await;
        match written {
            Ok(()) => ArtifactEvent::ReadyToOpen {
                id: artifact.id.clone(),
                path: path.to_string_lossy().into_owned(),
            },
            Err(error) => {
                tracing::warn!(%error, "failed to write artifact view");
                ArtifactEvent::Error("Sovra couldn't open this artifact.".into())
            }
        }
    }
}

fn missing() -> ArtifactEvent {
    ArtifactEvent::Error("That artifact is no longer in your library.".into())
}
