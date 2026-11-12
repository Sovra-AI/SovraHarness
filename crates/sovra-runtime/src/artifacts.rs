//! The artifact library: saves, edits and deletes artifacts through the memory
//! store, writes copies the user asked to keep, and prepares browser views.
//! Without a store the library still works for the session, in memory.

mod hosting;
mod project;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::Sender;
use sovra_adk::ArtifactSaver;
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, artifact_id,
    browser_page,
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
    websites: Mutex<BTreeMap<String, hosting::Website>>,
    events: Sender<Event>,
    /// Parent location for disposable project build directories.
    view_dir: PathBuf,
    terminal: Option<Arc<dyn sovra_core::terminal::TerminalAccess>>,
    building: Mutex<std::collections::BTreeSet<String>>,
    tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    build_dirs: Mutex<Vec<PathBuf>>,
}

impl ArtifactRuntime {
    pub(crate) fn new(
        store: Option<Arc<dyn MemoryStore>>,
        events: Sender<Event>,
        view_dir: PathBuf,
        terminal: Option<Arc<dyn sovra_core::terminal::TerminalAccess>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                library: Mutex::new(None),
                sequence: AtomicU64::new(0),
                websites: Mutex::new(BTreeMap::new()),
                events,
                view_dir,
                terminal,
                building: Mutex::new(std::collections::BTreeSet::new()),
                tasks: std::sync::Mutex::new(Vec::new()),
                build_dirs: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The `save_artifact` tool's persistence, for agents.
    pub(crate) fn saver(&self) -> ArtifactSaver {
        let runtime = self.clone();
        let hosting = self.clone();
        ArtifactSaver::from_fn(move |draft| {
            let runtime = runtime.clone();
            async move {
                if runtime.inner.store.is_none() {
                    return Err("Your saved-work library is unavailable. Keep the source from this chat and try saving again.".into());
                }
                let existing = runtime.library().await.as_ref().and_then(|library| {
                    library.values().find(|artifact| artifact.source.session_id == draft.source.session_id
                        && artifact.kind == draft.kind && artifact.content == draft.content).cloned()
                });
                if let Some(existing) = existing { return Ok(existing); }
                runtime.save(draft).await
            }
        })
        .with_deployer(move |id, session| {
            let runtime = hosting.clone();
            async move {
                let id = if id.is_empty() {
                    runtime
                        .library()
                        .await
                        .as_ref()
                        .and_then(|library| {
                            library
                                .values()
                                .filter(|artifact| {
                                    matches!(artifact.kind, ArtifactKind::Webpage | ArtifactKind::Project)
                                        && artifact.source.session_id == session
                                })
                                .max_by_key(|artifact| artifact.updated_at)
                                .map(|artifact| artifact.id.clone())
                        })
                        .ok_or_else(|| {
                            "Save a web page in this chat before running it.".to_owned()
                        })?
                } else {
                    id
                };
                let event = if runtime.get(&id).await.is_some_and(|a| a.kind == ArtifactKind::Project) { runtime.run_project(&id, &session).await } else { runtime.deploy(&id).await };
                let result = match &event {
                    ArtifactEvent::Deployed { url, .. } => Ok(url.clone()),
                    ArtifactEvent::Error(message) => Err(message.clone()),
                    _ => Err("The website could not be started.".into()),
                };
                let _ = runtime.inner.events.send(Event::Artifacts(event)).await;
                result
            }
        })
    }

    pub(crate) fn handle(&self, handle: &tokio::runtime::Handle, command: ArtifactCommand) {
        let runtime = self.clone();
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .expect("artifact task registry is not poisoned");
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle.spawn(async move { runtime.run(command).await }));
    }

    async fn run(&self, command: ArtifactCommand) {
        let event = match command {
            ArtifactCommand::RunProject { id, session_id } => {
                self.run_project(&id, &session_id).await
            }
            ArtifactCommand::Organize {
                id,
                collection,
                pinned,
            } => {
                let mut library = self.library().await;
                match library.as_mut().and_then(|library| library.get_mut(&id)) {
                    Some(existing) => {
                        let mut edited = existing.clone();
                        edited.collection = collection.trim().chars().take(80).collect();
                        edited.pinned = pinned;
                        match self.persist(&edited).await {
                            Ok(()) => {
                                *existing = edited.clone();
                                ArtifactEvent::Saved(Box::new(edited))
                            }
                            Err(message) => ArtifactEvent::Error(message),
                        }
                    }
                    None => missing(),
                }
            }
            ArtifactCommand::Deploy { id } => self.deploy(&id).await,
            ArtifactCommand::StopDeployment { id } => {
                if let Some(website) = self.inner.websites.lock().await.remove(&id) {
                    website.stop().await;
                }
                ArtifactEvent::DeploymentStopped { id }
            }
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
                let mut library = self.library().await;
                match library.as_mut().and_then(|library| library.get_mut(&id)) {
                    Some(existing) => {
                        let mut edited = existing.clone();
                        edited.edit(&title, &content, crate::now());
                        match self.persist(&edited).await {
                            Ok(()) => {
                                *existing = edited.clone();
                                ArtifactEvent::Saved(Box::new(edited))
                            }
                            Err(message) => ArtifactEvent::Error(message),
                        }
                    }
                    None => missing(),
                }
            }
            ArtifactCommand::Delete { id } => {
                let mut library = self.library().await;
                if library
                    .as_ref()
                    .is_some_and(|library| library.contains_key(&id))
                {
                    let result = match &self.inner.store {
                        Some(store) => store.delete_artifact(&id, crate::now()).await,
                        None => Ok(()),
                    };
                    match result {
                        Ok(()) => {
                            if let Some(library) = library.as_mut() {
                                library.remove(&id);
                            }
                            if let Some(website) = self.inner.websites.lock().await.remove(&id) {
                                website.stop().await;
                            }
                            ArtifactEvent::Deleted { id }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to delete artifact");
                            ArtifactEvent::Error(
                                "Sovra couldn't delete this artifact. Your work is still saved."
                                    .into(),
                            )
                        }
                    }
                } else {
                    missing()
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
                Some(artifact) if artifact.kind == ArtifactKind::Project => ArtifactEvent::Error(
                    "Use Build and open to prepare this website project.".into(),
                ),
                Some(artifact) => self.serve(&artifact.id, browser_page(&artifact)).await,
                None => missing(),
            },
        };
        let _ = self.inner.events.send(Event::Artifacts(event)).await;
    }

    async fn deploy(&self, id: &str) -> ArtifactEvent {
        let Some(artifact) = self.get(id).await else {
            return missing();
        };
        if artifact.kind != ArtifactKind::Webpage {
            return ArtifactEvent::Error("Choose a saved web page to run as a website.".into());
        }
        if artifact.content.len() > 8 * 1024 * 1024 {
            return ArtifactEvent::Error(
                "This web page is too large to run locally (8 MB maximum).".into(),
            );
        }
        self.serve(id, artifact.content).await
    }

    async fn serve(&self, id: &str, content: String) -> ArtifactEvent {
        let mut websites = self.inner.websites.lock().await;
        if websites.len() >= 8 && !websites.contains_key(id) {
            return ArtifactEvent::Error(
                "Stop one of your running websites before starting another.".into(),
            );
        }
        match hosting::host(&mut websites, id, hosting::page(content), false).await {
            Ok(url) => ArtifactEvent::Deployed {
                id: id.to_owned(),
                url,
            },
            Err(error) => {
                tracing::warn!(%error, "failed to start artifact website");
                ArtifactEvent::Error("Sovra couldn't start this website. Try again.".into())
            }
        }
    }

    pub(crate) async fn stop(&self) {
        let tasks = std::mem::take(
            &mut *self
                .inner
                .tasks
                .lock()
                .expect("artifact task registry is not poisoned"),
        );
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        for path in std::mem::take(&mut *self.inner.build_dirs.lock().await) {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
        let websites = std::mem::take(&mut *self.inner.websites.lock().await);
        for website in websites.into_values() {
            website.stop().await;
        }
    }

    async fn save(&self, draft: ArtifactDraft) -> Result<Artifact, String> {
        if draft.kind == ArtifactKind::Project {
            sovra_core::artifacts::WebsiteProject::parse(&draft.content)?;
        }
        let at = crate::now();
        let sequence = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        let artifact = Artifact::from_draft(artifact_id(at, sequence), draft, at);
        self.persist(&artifact).await?;
        self.library()
            .await
            .get_or_insert_with(BTreeMap::new)
            .insert(artifact.id.clone(), artifact.clone());
        let _ = self
            .inner
            .events
            .send(Event::Artifacts(ArtifactEvent::Saved(Box::new(
                artifact.clone(),
            ))))
            .await;
        Ok(artifact)
    }

    /// Publish success only after the storage port acknowledges the write.
    async fn persist(&self, artifact: &Artifact) -> Result<(), String> {
        if artifact.kind == ArtifactKind::Project {
            sovra_core::artifacts::WebsiteProject::parse(&artifact.content)?;
        }
        if let Some(store) = &self.inner.store {
            store.save_artifact(artifact).await.map_err(|error| {
                tracing::warn!(%error, "failed to save artifact");
                "Sovra couldn't save this work. Keep the source in chat and try again.".to_owned()
            })?;
        }
        Ok(())
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
}

fn missing() -> ArtifactEvent {
    ArtifactEvent::Error("That artifact is no longer in your library.".into())
}
