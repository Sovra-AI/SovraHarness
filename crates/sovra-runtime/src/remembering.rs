//! One owned remembering worker; no model work runs on the command loop.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::AdkHarness;
use sovra_core::{
    EpisodeKind, Event, MemoryCandidate, MemoryClaim, MemoryEvent, MemoryInput, MemorySearch,
    MemoryStatus, MemoryStore, NewEpisode, RecallQuery, consolidation_context, fuse_rrf,
};
use tokio::sync::watch;
use tokio::task::JoinSet;

#[derive(Clone, Default)]
pub(crate) struct Selection {
    initialized: bool,
    enabled: bool,
    harness: Option<AdkHarness>,
    pause_epoch: u64,
}

#[derive(Clone)]
pub(crate) struct MemoryControl {
    sender: watch::Sender<Selection>,
    requested: Arc<AtomicBool>,
    recall_changes: watch::Sender<u64>,
}
impl MemoryControl {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(Selection::default());
        Self {
            sender,
            requested: Arc::new(AtomicBool::new(true)),
            recall_changes: watch::channel(0).0,
        }
    }
    pub(crate) fn request(&self, enabled: bool) {
        self.requested.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.sender.send_modify(|selection| {
                selection.initialized = true;
                selection.enabled = false;
                selection.pause_epoch = selection.pause_epoch.saturating_add(1);
            });
        }
    }
    pub(crate) fn publish(&self, enabled: bool, harness: Option<AdkHarness>) {
        self.sender.send_if_modified(|selection| {
            let enabled = enabled && self.requested.load(Ordering::SeqCst);
            let same_model = match (&selection.harness, &harness) {
                (Some(old), Some(new)) => old.shares_memory_model(new),
                (None, None) => true,
                _ => false,
            };
            if selection.initialized && selection.enabled == enabled && same_model {
                return false;
            }
            selection.initialized = true;
            selection.enabled = enabled;
            selection.harness = harness;
            true
        });
    }

    pub(crate) fn enabled(&self) -> bool {
        self.requested.load(Ordering::SeqCst) && self.sender.borrow().enabled
    }
    pub(crate) fn pause_epoch(&self) -> u64 {
        self.sender.borrow().pause_epoch
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<Selection> {
        self.sender.subscribe()
    }

    /// A committed user edit revokes older readers without changing automatic
    /// remembering settings or cancelling unrelated queued extraction jobs.
    pub(crate) fn invalidate_recall(&self) {
        self.recall_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub(crate) fn recall_revision(&self) -> u64 {
        *self.recall_changes.borrow()
    }
    pub(crate) fn subscribe_recall(&self) -> watch::Receiver<u64> {
        self.recall_changes.subscribe()
    }
}

pub(crate) struct RememberingRuntime {
    inputs: Sender<MemoryInput>,
    control: MemoryControl,
    events: Sender<Event>,
    task: tokio::task::JoinHandle<()>,
}
impl RememberingRuntime {
    pub(crate) fn start(
        store: Arc<dyn MemoryStore>,
        control: MemoryControl,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (inputs, receiver) = async_channel::bounded(64);
        let actor = Worker {
            store,
            events: events.clone(),
            control: control.clone(),
            config: control.sender.subscribe(),
            inputs: receiver,
            jobs: JoinSet::new(),
            active: None,
            pause_epoch: 0,
        };
        Self {
            inputs,
            control,
            events,
            task: handle.spawn(actor.run()),
        }
    }
    pub(crate) fn record_user(&self, session: &str, content: &str) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if !self.control.enabled() || content.trim().is_empty() {
            return;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos());
        let key = format!("{stamp}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let input = MemoryInput {
            key,
            episode: NewEpisode {
                session_id: session.into(),
                kind: EpisodeKind::UserMessage,
                content: content.into(),
                occurred_at: crate::now(),
            },
        };
        if self.inputs.try_send(input).is_err() {
            let _ = self
                .events
                .try_send(Event::Memory(MemoryEvent::Status(MemoryStatus::Failed(
                    "Remembering is busy. This message remains in your chat history.".into(),
                ))));
        }
    }
    pub(crate) async fn stop(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for RememberingRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Worker {
    store: Arc<dyn MemoryStore>,
    events: Sender<Event>,
    control: MemoryControl,
    config: watch::Receiver<Selection>,
    inputs: Receiver<MemoryInput>,
    jobs: JoinSet<Result<Vec<MemoryCandidate>, String>>,
    active: Option<(MemoryClaim, String)>,
    pause_epoch: u64,
}
impl Worker {
    async fn status(&self, status: MemoryStatus) {
        let _ = self
            .events
            .send(Event::Memory(MemoryEvent::Status(status)))
            .await;
    }
    async fn reconcile(&mut self, selection: &Selection) -> bool {
        self.jobs.abort_all();
        while self.jobs.join_next().await.is_some() {}
        if !selection.initialized {
            self.status(MemoryStatus::Loading).await;
            return false;
        }
        let must_cancel = !selection.enabled || self.pause_epoch != selection.pause_epoch;
        if must_cancel {
            while self.inputs.try_recv().is_ok() {}
            if self.store.set_memory_enabled(false).await.is_err() {
                self.status(MemoryStatus::Failed(
                    "Remembering could not be paused in storage.".into(),
                ))
                .await;
                self.active = None;
                return false;
            }
        } else if let Some((claim, _)) = self.active.take() {
            let _ = self
                .store
                .fail_memory(
                    &claim,
                    "The selected AI changed. Remembering will retry.",
                    crate::now(),
                )
                .await;
        }
        self.active = None;
        self.pause_epoch = selection.pause_epoch;
        if self
            .store
            .set_memory_enabled(selection.enabled)
            .await
            .is_err()
        {
            self.status(MemoryStatus::Failed(
                "Remembering settings could not be applied.".into(),
            ))
            .await;
            return false;
        }
        self.status(if !selection.enabled {
            MemoryStatus::Paused
        } else if selection.harness.is_none() {
            MemoryStatus::WaitingForModel
        } else {
            MemoryStatus::Ready
        })
        .await;
        true
    }
    async fn run(mut self) {
        let mut selection = self.config.borrow_and_update().clone();
        let mut ready = self.reconcile(&selection).await;
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                changed = self.config.changed() => {
                    if changed.is_err() { break; }
                    selection = self.config.borrow_and_update().clone();
                    ready = self.reconcile(&selection).await;
                }
                input = self.inputs.recv() => {
                    let Ok(input) = input else { break; };
                    if !ready || !selection.enabled || !self.control.enabled() { continue; }
                    let session_id = input.episode.session_id.clone();
                    match self.store.enqueue_memory(input).await {
                        Ok(episode_id) => { let _ = self.events.send(Event::Memory(MemoryEvent::Queued { session_id, episode_id })).await; }
                        Err(_) => self.status(MemoryStatus::Failed("This message could not be queued for remembering.".into())).await,
                    }
                }
                result = self.jobs.join_next(), if !self.jobs.is_empty() => {
                    if self.config.has_changed().unwrap_or(true) || !self.control.enabled() { continue; }
                    let Some((claim, model)) = self.active.take() else { continue; };
                    let outcome = match result {
                        Some(Ok(outcome)) => outcome,
                        _ => Err("Remembering was interrupted.".into()),
                    };
                    match outcome {
                        Ok(candidates) => {
                            if self.store.complete_memory(&claim, candidates, &model, crate::now()).await.is_ok() {
                                self.status(MemoryStatus::Ready).await;
                            } else {
                                let _ = self.store.fail_memory(&claim, "Remembering could not save this result.", crate::now()).await;
                                self.status(MemoryStatus::Failed("Remembering could not save this result.".into())).await;
                            }
                        }
                        Err(message) => {
                            let _ = self.store.fail_memory(&claim, &message, crate::now()).await;
                            self.status(MemoryStatus::Failed(message)).await;
                        }
                    }
                    if let Ok(job) = self.store.memory_job(&claim.episode.id).await { let _ = self.events.send(Event::Memory(MemoryEvent::Processed(job))).await; }
                }
                _ = tick.tick() => {
                    if !ready { ready = self.reconcile(&selection).await; }
                    if !ready || !selection.enabled || !self.control.enabled() || !self.jobs.is_empty() || self.active.is_some() { continue; }
                    let Some(harness) = selection.harness.clone() else { continue; };
                    match self.store.claim_memory(crate::now()).await {
                        Ok(Some(claim)) => {
                            let source = claim.episode.clone();
                            self.active = Some((claim, harness.memory_model_name().into()));
                            let store = self.store.clone();
                            self.status(MemoryStatus::Working).await;
                            self.jobs.spawn(async move {
                                tokio::time::timeout(Duration::from_secs(90), async move {
                                    let known = tokio::time::timeout(Duration::from_secs(2), async {
                                        let now = crate::now();
                                        let query = MemorySearch { text: source.content.clone(), ..Default::default() };
                                        let legs = crate::recalling::keyword_candidates(store.as_ref(), &query, now).await?;
                                        let related = fuse_rrf(legs, 24).into_iter().map(|hit| hit.fact).collect();
                                        let recent = store.recall(RecallQuery::recent(8)).await?.into_iter().map(|hit| hit.fact).collect();
                                        Ok::<_, sovra_core::MemoryError>(consolidation_context(related, recent, now))
                                    }).await.map_err(|_| "Earlier memories took too long to check.".to_owned())?
                                        .map_err(|_| "Earlier memories could not be read.".to_owned())?;
                                    harness.extract_memories(&source, &known).await.map_err(|error| error.to_string())
                                }).await.unwrap_or_else(|_| Err("Remembering took too long and will retry.".into()))
                            });
                        }
                        Ok(None) => {}
                        Err(_) => self.status(MemoryStatus::Failed("Remembering could not read its pending work.".into())).await,
                    }
                }
            }
        }
    }
}
