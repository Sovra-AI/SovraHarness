//! Owned background scans. Cancellation bypasses the mutation queue; searches
//! run independently so an ongoing scan cannot block the command loop.

use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_channel::Sender;
use sovra_core::{
    Event,
    file_index::{FileIndex, IndexCommand, IndexError, IndexEvent, IndexedRoot, ScanObserver},
};
use tokio::sync::Notify;

mod monitor;
mod semantic;

#[derive(Default)]
struct Controls {
    cancellations: BTreeMap<String, Arc<AtomicBool>>,
    preferences: BTreeMap<String, bool>,
    reconfiguring: std::collections::BTreeSet<String>,
}
type SharedControls = Arc<Mutex<Controls>>;

pub(crate) struct IndexRuntime {
    pub(crate) reader: Option<Arc<dyn sovra_core::file_index::IndexedFiles>>,
    store: Option<Arc<dyn FileIndex>>,
    commands: Sender<IndexCommand>,
    events: Sender<Event>,
    controls: SharedControls,
    wake: Arc<Notify>,
    stopped: Arc<AtomicBool>,
    actor: tokio::task::JoinHandle<()>,
    semantic: Option<semantic::Runtime>,
    readers: Vec<tokio::task::JoinHandle<()>>,
    handle: tokio::runtime::Handle,
}

impl IndexRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn FileIndex>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
        repository: Option<sovra_providers::ModelRepository>,
    ) -> Self {
        let semantic = store
            .as_ref()
            .filter(|s| s.semantic().is_some())
            .zip(repository)
            .map(|(store, repository)| {
                semantic::Runtime::start(store.clone(), repository, events.clone(), handle)
            });
        let reader = store.as_ref().map(|store| {
            Arc::new(Reader(
                store.clone(),
                semantic.as_ref().map(|s| s.service.clone()),
                events.clone(),
            )) as Arc<dyn sovra_core::file_index::IndexedFiles>
        });
        let (commands, receiver) = async_channel::bounded(16);
        let controls = Arc::new(Mutex::new(Controls::default()));
        let wake = Arc::new(Notify::new());
        let stopped = Arc::new(AtomicBool::new(false));
        let actor = handle.spawn({
            let store = store.clone();
            let events = events.clone();
            let controls = controls.clone();
            let wake = Arc::clone(&wake);
            let stopped = stopped.clone();
            async move {
                let Some(store) = store else {
                    return;
                };
                run_actor(store, receiver, events, controls, wake, stopped).await;
            }
        });
        Self {
            reader,
            store,
            commands,
            events,
            controls,
            wake,
            stopped,
            actor,
            semantic,
            readers: vec![],
            handle: handle.clone(),
        }
    }

    pub(crate) fn send(&mut self, command: IndexCommand) {
        let Some(store) = self.store.clone() else {
            self.error("Folder indexing is unavailable in this session.");
            return;
        };
        match &command {
            IndexCommand::SetScanOptions { root_id, .. } => {
                let mut controls = self
                    .controls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                controls.reconfiguring.insert(root_id.clone());
                if let Some(token) = controls.cancellations.get(root_id) {
                    token.store(true, Ordering::Relaxed);
                }
            }
            IndexCommand::ConfigureSemantic { root_id, target } => {
                let result = self
                    .semantic
                    .as_ref()
                    .ok_or_else(|| {
                        IndexError("Set up an embedding connection in Models first.".into())
                    })
                    .and_then(|s| s.configure(root_id.clone(), target.clone()));
                if let Err(error) = result {
                    self.error(&error.to_string());
                }
                return;
            }
            IndexCommand::Cancel { root_id } | IndexCommand::Remove { root_id } => {
                if matches!(command, IndexCommand::Remove { .. })
                    && let Some(semantic) = &self.semantic
                {
                    semantic.revoke(root_id);
                }
                let mut controls = self
                    .controls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                controls.reconfiguring.remove(root_id);
                controls.preferences.insert(root_id.clone(), false);
                if let Some(token) = controls.cancellations.get(root_id) {
                    token.store(true, Ordering::Relaxed);
                }
                if matches!(command, IndexCommand::Cancel { .. }) {
                    // Pause is not a queued scan. Wake the actor so it does not
                    // wait out a skipped one-second tick.
                    self.wake.notify_one();
                    return;
                }
            }
            IndexCommand::SetAutoRefresh { root_id, enabled } => {
                let mut controls = self
                    .controls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                controls.reconfiguring.remove(root_id);
                controls.preferences.insert(root_id.clone(), *enabled);
                if !enabled && let Some(token) = controls.cancellations.get(root_id) {
                    token.store(true, Ordering::Relaxed);
                }
                self.wake.notify_one();
                return;
            }
            IndexCommand::Refresh { root_id } => {
                let mut controls = self
                    .controls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let tokens = &mut controls.cancellations;
                if tokens.contains_key(root_id) {
                    return;
                }
                tokens.insert(root_id.clone(), Arc::new(AtomicBool::new(false)));
            }
            IndexCommand::List
            | IndexCommand::Search { .. }
            | IndexCommand::ListKnowledge
            | IndexCommand::ReviewKnowledge { .. } => {
                self.readers.retain(|task| !task.is_finished());
                if self.readers.len() >= 4 {
                    self.error("Wait for the current file search to finish, then try again.");
                    return;
                }
                let events = self.events.clone();
                let reader = self.reader.clone();
                self.readers.push(self.handle.spawn(async move {
                    if let IndexCommand::Search { request_id, query } = command {
                        if let Some(reader) = reader {
                            match reader.search_report(query).await {
                                Ok(result) => {
                                    let _ =
                                        events.try_send(Event::FileIndex(IndexEvent::Results {
                                            request_id,
                                            passages: result.passages,
                                        }));
                                    if !result.notices.is_empty() {
                                        let _ = events.try_send(Event::FileIndex(
                                            IndexEvent::SearchNotice {
                                                request_id,
                                                message: result.notices.join(" "),
                                            },
                                        ));
                                    }
                                }
                                Err(error) => {
                                    let _ = events.try_send(Event::FileIndex(IndexEvent::Error(
                                        error.to_string(),
                                    )));
                                }
                            }
                        }
                        return;
                    }
                    let result = tokio::task::spawn_blocking(move || match command {
                        IndexCommand::Search { request_id, query } => {
                            store.search(&query).map(|passages| IndexEvent::Results {
                                request_id,
                                passages,
                            })
                        }
                        IndexCommand::List => store.list().map(IndexEvent::Roots),
                        IndexCommand::ListKnowledge => knowledge(store.as_ref())?
                            .pending()
                            .map(IndexEvent::KnowledgePending),
                        IndexCommand::ReviewKnowledge {
                            proposal_id,
                            approve,
                        } => knowledge(store.as_ref())?
                            .decide(proposal_id, approve)
                            .map(|_| IndexEvent::KnowledgeReviewed {
                                proposal_id,
                                accepted: approve,
                            }),
                        IndexCommand::AddRoot { .. }
                        | IndexCommand::SetScanOptions { .. }
                        | IndexCommand::Refresh { .. }
                        | IndexCommand::Cancel { .. }
                        | IndexCommand::Remove { .. }
                        | IndexCommand::SetAutoRefresh { .. }
                        | IndexCommand::ConfigureSemantic { .. } => {
                            Err(IndexError("Invalid index read operation.".into()))
                        }
                    })
                    .await;
                    report(result, &events);
                }));
                return;
            }
            IndexCommand::AddRoot { .. } => {}
        }
        if let Err(error) = self.commands.try_send(command) {
            if let IndexCommand::Refresh { root_id } = error.into_inner() {
                self.controls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .cancellations
                    .remove(&root_id);
            }
            self.error("Too many indexing operations are queued. Wait briefly and retry.");
        }
    }

    fn error(&self, message: &str) {
        let _ = self
            .events
            .try_send(Event::FileIndex(IndexEvent::Error(message.into())));
    }
}

async fn run_actor(
    store: Arc<dyn FileIndex>,
    receiver: async_channel::Receiver<IndexCommand>,
    events: Sender<Event>,
    controls: SharedControls,
    wake: Arc<Notify>,
    stopped: Arc<AtomicBool>,
) {
    let mut monitors = monitor::Monitors::new(events.clone());
    let initial = store.clone();
    let roots = tokio::task::spawn_blocking(move || {
        initial.recover()?;
        initial.list()
    })
    .await;
    match roots {
        Ok(Ok(roots)) => {
            let _ = events.try_send(Event::FileIndex(IndexEvent::Roots(roots.clone())));
            let knowledge_store = store.clone();
            let pending = tokio::task::spawn_blocking(move || {
                knowledge_store.knowledge().map_or_else(
                    || Ok(vec![]),
                    sovra_core::file_index::graph::FileKnowledge::pending,
                )
            })
            .await;
            report(
                pending.map(|r| r.map(IndexEvent::KnowledgePending)),
                &events,
            );
            monitors.sync(&store, &roots, None).await;
        }
        result => report(result.map(|r| r.map(IndexEvent::Roots)), &events),
    }
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let preferences = {
            let mut control = controls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut control.preferences)
        };
        if !preferences.is_empty() {
            for (id, enabled) in &preferences {
                monitors.set_enabled(id, *enabled);
            }
            let work_store = store.clone();
            let work_events = events.clone();
            let result = tokio::task::spawn_blocking(move || {
                for (id, enabled) in preferences {
                    if let Err(error) = work_store.set_auto_refresh(&id, enabled) {
                        let message = if enabled { error.to_string() } else {
                            format!("Automatic updates are paused for this session, but the preference could not be saved for reopening: {error}")
                        };
                        let _ = work_events.try_send(Event::FileIndex(IndexEvent::Error(message)));
                    }
                }
                work_store.list()
            })
            .await;
            match result {
                Ok(Ok(roots)) => {
                    let _ = events.try_send(Event::FileIndex(IndexEvent::Roots(roots.clone())));
                    monitors.sync(&store, &roots, None).await;
                }
                result => report(result.map(|r| r.map(IndexEvent::Roots)), &events),
            }
        }
        let command = tokio::select! {
            biased;
            // Preference edits are stored beside the queue. A stored permit
            // still fires if the edit landed before this wait.
            () = wake.notified() => continue,
            command = receiver.recv() => match command {
                Ok(command) => command,
                Err(_) => break,
            },
            _ = interval.tick() => {
                let Some(root_id) = monitors.due() else { continue; };
                let mut control = controls.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if control.preferences.get(&root_id) == Some(&false) || control.cancellations.contains_key(&root_id) {
                    continue;
                }
                control.cancellations.insert(root_id.clone(), Arc::new(AtomicBool::new(false)));
                IndexCommand::Refresh { root_id }
            }
        };
        if let IndexCommand::Refresh { root_id } = &command {
            monitors.started(root_id);
        }
        let work_store = store.clone();
        let work_controls = controls.clone();
        let work_events = events.clone();
        let work_stopped = stopped.clone();
        let result = tokio::task::spawn_blocking(move || {
            mutate(
                &work_store,
                command,
                &work_controls,
                work_stopped,
                work_events,
            )
        })
        .await;
        match result {
            Ok(Ok((roots, scanned))) => {
                let _ = events.try_send(Event::FileIndex(IndexEvent::Roots(roots.clone())));
                monitors.sync(&store, &roots, scanned.as_deref()).await;
            }
            result => report(
                result.map(|r| r.map(|(roots, _)| IndexEvent::Roots(roots))),
                &events,
            ),
        }
    }
}

fn mutate(
    store: &Arc<dyn FileIndex>,
    command: IndexCommand,
    controls: &SharedControls,
    stopped: Arc<AtomicBool>,
    events: Sender<Event>,
) -> Result<(Vec<IndexedRoot>, Option<String>), IndexError> {
    let root_id = match command {
        IndexCommand::SetScanOptions { root_id, options } => {
            controls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reconfiguring
                .remove(&root_id);
            store.set_scan_options(&root_id, options)?;
            Some(root_id)
        }
        IndexCommand::AddRoot { path } => Some(store.add_root(&path)?.id),
        IndexCommand::Refresh { root_id } => Some(root_id),
        IndexCommand::Remove { root_id } => {
            store.remove(&root_id)?;
            None
        }
        IndexCommand::List
        | IndexCommand::Cancel { .. }
        | IndexCommand::SetAutoRefresh { .. }
        | IndexCommand::Search { .. }
        | IndexCommand::ConfigureSemantic { .. }
        | IndexCommand::ListKnowledge
        | IndexCommand::ReviewKnowledge { .. } => {
            return Err(IndexError("Invalid background index operation.".into()));
        }
    };
    if let Some(id) = &root_id {
        let token = {
            let mut control = controls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            control
                .cancellations
                .entry(id.clone())
                .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                .clone()
        };
        let observer = Observer {
            root_id: id.clone(),
            controls: controls.clone(),
            token,
            stopped,
            events,
        };
        let result = store.scan(id, &observer);
        controls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancellations
            .remove(id);
        result?;
    }
    Ok((store.list()?, root_id))
}

#[derive(Debug)]
struct Reader(
    Arc<dyn FileIndex>,
    Option<Arc<semantic::Service>>,
    Sender<Event>,
);

fn knowledge(
    store: &dyn FileIndex,
) -> Result<&dyn sovra_core::file_index::graph::FileKnowledge, IndexError> {
    store
        .knowledge()
        .ok_or_else(|| IndexError("File knowledge is unavailable in this session.".into()))
}

#[async_trait::async_trait]
impl sovra_core::file_index::IndexedFiles for Reader {
    async fn propose_knowledge(
        &self,
        draft: sovra_core::file_index::graph::GraphDraft,
        origin: sovra_core::file_index::graph::GraphOrigin,
    ) -> Result<sovra_core::file_index::graph::KnowledgeProposal, IndexError> {
        let store = self.0.clone();
        let (proposal, pending) = retrieve(move || {
            let graph = knowledge(store.as_ref())?;
            let proposal = graph.propose(&draft, &origin)?;
            Ok((proposal, graph.pending()?))
        })
        .await?;
        let _ = self
            .2
            .try_send(Event::FileIndex(IndexEvent::KnowledgePending(pending)));
        Ok(proposal)
    }
    async fn knowledge(
        &self,
        query: sovra_core::file_index::graph::GraphQuery,
    ) -> Result<sovra_core::file_index::graph::GraphPage, IndexError> {
        let store = self.0.clone();
        retrieve(move || knowledge(store.as_ref())?.query(&query)).await
    }
    async fn knowledge_evidence(
        &self,
        id: u64,
    ) -> Result<sovra_core::file_index::graph::GraphEvidence, IndexError> {
        let store = self.0.clone();
        retrieve(move || knowledge(store.as_ref())?.evidence(id)).await
    }
    async fn search_report(
        &self,
        query: sovra_core::file_index::IndexQuery,
    ) -> Result<sovra_core::file_index::semantic::SearchReport, IndexError> {
        if let Some(service) = &self.1 {
            return service.search(query).await;
        }
        let store = self.0.clone();
        let passages = retrieve(move || store.search(&query)).await?;
        Ok(sovra_core::file_index::semantic::SearchReport {
            passages,
            notices: vec![],
        })
    }
    async fn history(
        &self,
        query: sovra_core::file_index::FileHistoryQuery,
    ) -> Result<sovra_core::file_index::FileHistory, IndexError> {
        let store = self.0.clone();
        retrieve(move || store.history(&query)).await
    }
    async fn read_history(
        &self,
        reference: sovra_core::file_index::HistoryPassageRef,
    ) -> Result<sovra_core::file_index::HistoricalPassage, IndexError> {
        let store = self.0.clone();
        retrieve(move || store.read_history(&reference)).await
    }
    async fn folders(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        let store = self.0.clone();
        retrieve(move || store.list()).await
    }
    async fn catalogue(
        &self,
        query: sovra_core::file_index::CatalogQuery,
    ) -> Result<sovra_core::file_index::CatalogPage, IndexError> {
        let store = self.0.clone();
        retrieve(move || store.catalogue(&query)).await
    }

    async fn search(
        &self,
        query: sovra_core::file_index::IndexQuery,
    ) -> Result<Vec<sovra_core::file_index::IndexPassage>, IndexError> {
        self.search_report(query).await.map(|r| r.passages)
    }
    async fn read(
        &self,
        reference: sovra_core::file_index::PassageRef,
    ) -> Result<sovra_core::file_index::IndexPassage, IndexError> {
        let store = self.0.clone();
        retrieve(move || store.read(&reference)).await
    }
}

async fn retrieve<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IndexError> + Send + 'static,
) -> Result<T, IndexError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::task::spawn_blocking(work),
    )
    .await
    .map_err(|_| {
        IndexError("Reading the file index took too long. Try a more specific query.".into())
    })?
    .map_err(|_| {
        IndexError("The index reader stopped unexpectedly. Refresh Knowledge and retry.".into())
    })?
}

impl Drop for IndexRuntime {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.actor.abort();
        for task in &self.readers {
            task.abort();
        }
    }
}

struct Observer {
    root_id: String,
    controls: SharedControls,
    token: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    events: Sender<Event>,
}
impl ScanObserver for Observer {
    fn cancellation(&self) -> Option<sovra_core::file_index::ScanCancellation> {
        use sovra_core::file_index::ScanCancellation;
        if self.token.load(Ordering::Relaxed) {
            let controls = self
                .controls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Some(if controls.reconfiguring.contains(&self.root_id) {
                ScanCancellation::Interrupted
            } else {
                ScanCancellation::Requested
            })
        } else if self.stopped.load(Ordering::Relaxed) {
            Some(ScanCancellation::Interrupted)
        } else {
            None
        }
    }
    fn progress(&self, root: &IndexedRoot) {
        let _ = self
            .events
            .try_send(Event::FileIndex(IndexEvent::Progress(root.clone())));
    }
}

fn report(
    result: Result<Result<IndexEvent, IndexError>, tokio::task::JoinError>,
    events: &Sender<Event>,
) {
    let event = match result {
        Ok(Ok(event)) => event,
        Ok(Err(error)) => IndexEvent::Error(error.to_string()),
        Err(_) => IndexEvent::Error(
            "The indexing worker stopped unexpectedly. Refresh the folder to retry.".into(),
        ),
    };
    let _ = events.try_send(Event::FileIndex(event));
}
