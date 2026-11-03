//! Bounded parallel extraction with ordered, revision-checked consolidation; no model work runs on the command loop.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::AdkHarness;
use sovra_core::{
    EpisodeKind, Event, MemoryClaim, MemoryEvent, MemoryInput, MemorySearch, MemoryStatus,
    MemoryStore, NewEpisode, RecallQuery, consolidation_context, fuse_rrf,
};
use tokio::sync::{Mutex, Semaphore, watch};
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
    inputs: Sender<CaptureInput>,
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
        slots: usize,
    ) -> Self {
        static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let slots = if slots == 0 {
            sovra_core::DEFAULT_MEMORY_EXTRACTION_SLOTS
        } else {
            slots.clamp(1, 16)
        };
        let permits = SLOTS
            .get_or_init(|| Arc::new(Semaphore::new(slots)))
            .clone();
        let (inputs, receiver) = async_channel::bounded(64);
        let actor = Worker {
            store,
            events: events.clone(),
            control: control.clone(),
            config: control.sender.subscribe(),
            inputs: receiver,
            jobs: JoinSet::new(),
            active: std::collections::HashMap::new(),
            slots,
            permits,
            pause_epoch: 0,
        };
        Self {
            inputs,
            control,
            events,
            task: handle.spawn(actor.run()),
        }
    }
    pub(crate) async fn record_user(
        &self,
        session: &str,
        content: &str,
        current: Option<&sovra_core::ConversationInput>,
    ) -> Option<tokio::sync::oneshot::Receiver<sovra_core::EpisodeId>> {
        if !self.control.enabled() || content.trim().is_empty() {
            return None;
        }
        let (saved, receipt) = tokio::sync::oneshot::channel();
        if let Some(source) = current.and_then(|input| input.source.clone()) {
            let _ = saved.send(source);
            return Some(receipt);
        }
        let key = current
            .filter(|input| input.occurred_at.is_some())
            .map_or_else(source_key, |input| format!("message:{}", input.id));
        let content = current.map_or(content, |input| input.text.as_str());
        let input = MemoryInput {
            key,
            episode: NewEpisode {
                session_id: session.into(),
                kind: EpisodeKind::UserMessage,
                content: content.into(),
                occurred_at: current
                    .and_then(|input| input.occurred_at)
                    .unwrap_or_else(crate::now),
            },
        };
        let message = current.map(|current| sovra_core::MemorySourceMessage {
            id: Some(current.id.clone()),
            in_reply_to: None,
            role: sovra_core::MessageRole::User,
            text: content.into(),
        });
        if self
            .inputs
            .send(CaptureInput {
                input,
                message,
                saved: Some(saved),
                epoch: self.control.pause_epoch(),
            })
            .await
            .is_err()
        {
            let _ = self
                .events
                .try_send(Event::Memory(MemoryEvent::Status(MemoryStatus::Failed(
                    "This message could not be saved for remembering.".into(),
                ))));
            return None;
        }
        Some(receipt)
    }
    pub(crate) async fn stop(&mut self) {
        // Closing keeps accepted evidence in the queue until its ordered storage
        // write finishes. Model work can resume from the persisted job on reopen.
        self.inputs.close();
        let _ = (&mut self.task).await;
    }
}
impl Drop for RememberingRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct CaptureInput {
    input: MemoryInput,
    message: Option<sovra_core::MemorySourceMessage>,
    saved: Option<tokio::sync::oneshot::Sender<sovra_core::EpisodeId>>,
    epoch: u64,
}

struct Worker {
    store: Arc<dyn MemoryStore>,
    events: Sender<Event>,
    control: MemoryControl,
    config: watch::Receiver<Selection>,
    inputs: Receiver<CaptureInput>,
    jobs: JoinSet<Result<(), String>>,
    active: std::collections::HashMap<tokio::task::Id, MemoryClaim>,
    slots: usize,
    permits: Arc<Semaphore>,
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
                self.active.clear();
                return false;
            }
        } else {
            for (_, claim) in self.active.drain() {
                let _ = self
                    .store
                    .fail_memory(
                        &claim,
                        "The selected AI changed. Remembering will retry.",
                        crate::now(),
                    )
                    .await;
            }
        }
        self.active.clear();
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
        let mut dispatch_pending = ready;
        let mut foreground = crate::model_priority::subscribe();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                changed = self.config.changed() => {
                    if changed.is_err() { break; }
                    selection = self.config.borrow_and_update().clone();
                    ready = self.reconcile(&selection).await;
                    dispatch_pending = ready;
                }
                input = self.inputs.recv() => {
                    let Ok(input) = input else { break; };
                    if !ready || !selection.enabled || !self.control.enabled() || input.epoch != self.control.pause_epoch() { continue; }
                    let session_id = input.input.episode.session_id.clone();
                    match self.store.enqueue_memory(input.input).await {
                        Ok(episode_id) => {
                            dispatch_pending = true;
                            if let Some(saved) = input.saved { let _ = saved.send(episode_id.clone()); }
                            let _ = self.events.send(Event::Memory(MemoryEvent::SourceSaved {
                                session_id: session_id.clone(), message: input.message,
                                receipt: sovra_core::MemorySourceReceipt { source_id: episode_id.clone(), version: 1, has_processing_job: true },
                            })).await;
                            let _ = self.events.send(Event::Memory(MemoryEvent::Queued { session_id, episode_id })).await;
                        }
                        Err(_) => self.status(MemoryStatus::Failed("This message could not be queued for remembering.".into())).await,
                    }
                }
                result = self.jobs.join_next_with_id(), if !self.jobs.is_empty() => {
                    let (id, outcome) = match result {
                        Some(Ok((id, outcome))) => (id, outcome),
                        Some(Err(error)) => (error.id(), Err("Remembering was interrupted.".into())),
                        None => continue,
                    };
                    let Some(claim) = self.active.remove(&id) else { continue; };
                    dispatch_pending = true;
                    if self.config.has_changed().unwrap_or(true) || !self.control.enabled() { continue; }
                    if let Err(message) = outcome {
                        let _ = self.store.fail_memory(&claim, &message, crate::now()).await;
                        self.status(MemoryStatus::Failed(message)).await;
                    } else if self.jobs.is_empty() {
                        self.status(MemoryStatus::Ready).await;
                    }
                    if let Ok(job) = self.store.memory_job(&claim.episode.id).await {
                        let _ = self.events.send(Event::Memory(MemoryEvent::Processed(job))).await;
                    }
                }
                changed = foreground.changed() => {
                    if changed.is_err() { break; }
                    dispatch_pending = true;
                }
                _ = tick.tick() => {
                    if !ready { ready = self.reconcile(&selection).await; }
                    // Poll only for delayed retries, expired leases or permits held
                    // by another worker. New evidence and free slots wake directly.
                    dispatch_pending = true;
                }
                () = std::future::ready(()), if dispatch_pending && ready
                    && selection.enabled && self.control.enabled()
                    && self.jobs.len() < self.slots && !crate::model_priority::busy() => {
                    dispatch_pending = false;
                    let Some(harness) = selection.harness.clone() else { continue; };
                    let Ok(permit) = self.permits.clone().try_acquire_owned() else { continue; };
                    match self.store.claim_memory(crate::now()).await {
                        Ok(Some(claim)) => {
                            let store = self.store.clone();
                            let task_claim = claim.clone();
                            self.status(MemoryStatus::Working).await;
                            let task = self.jobs.spawn(async move {
                                let _permit = permit;
                                tokio::time::timeout(Duration::from_secs(90), process_chunk(store, harness, task_claim))
                                    .await.unwrap_or_else(|_| Err("Remembering took too long and will retry.".into()))
                            });
                            self.active.insert(task.id(), claim);
                            // One claim per select turn keeps capture and pause ahead
                            // of dispatch while filling every available slot.
                            dispatch_pending = true;
                        }
                        Ok(None) => {}
                        Err(_) => self.status(MemoryStatus::Failed("Remembering could not read its pending work.".into())).await,
                    }
                }
            }
        }
        self.jobs.abort_all();
        while self.jobs.join_next().await.is_some() {}
        for (_, claim) in self.active.drain() {
            let _ = self
                .store
                .fail_memory(
                    &claim,
                    "Remembering was interrupted when Sovra closed.",
                    crate::now(),
                )
                .await;
        }
    }
}

async fn process_chunk(
    store: Arc<dyn MemoryStore>,
    harness: AdkHarness,
    mut claim: MemoryClaim,
) -> Result<(), String> {
    static CONSOLIDATION: OnceLock<Mutex<()>> = OnceLock::new();
    crate::model_priority::background_ready().await;
    let source = claim.chunk().map_err(|error| error.to_string())?;
    // Independent extraction never assumes that a fact ID remains current.
    let mut context = store
        .memory_preceding_sources(&source.id)
        .await
        .map_err(|_| "Earlier conversation context could not be read.".to_owned())?;
    if claim.cursor > 0 {
        let mut preceding = claim.episode.clone();
        preceding.content.truncate(claim.cursor);
        context.push(preceding);
    }
    for episode in &mut context {
        let mut start = episode.content.len().saturating_sub(1024);
        while !episode.content.is_char_boundary(start) {
            start += 1;
        }
        episode.content = episode.content[start..].to_owned();
    }
    if store
        .memory_job(&claim.episode.id)
        .await
        .map_err(|error| error.to_string())?
        .state
        == sovra_core::MemoryJobState::Cancelled
    {
        return Err("This original message is no longer available for processing.".into());
    }
    let extracted = if let Some(saved) = &claim.extracted {
        saved.clone()
    } else {
        let saved = sovra_core::MemoryExtractedChunk {
            candidates: harness
                .extract_memories_in_context(&source, &context)
                .await
                .map_err(|error| error.to_string())?,
            model: harness.memory_model_name().to_owned(),
        };
        store
            .save_memory_extraction(&claim, &saved, crate::now())
            .await
            .map_err(|error| {
                tracing::warn!(%error, "failed to stage memory extraction");
                "Extracted details could not be saved for reconciliation.".to_owned()
            })?;
        saved
    };
    let _ordered = CONSOLIDATION.get_or_init(|| Mutex::new(())).lock().await;
    for _ in 0..3 {
        crate::model_priority::background_ready().await;
        if store
            .memory_job(&claim.episode.id)
            .await
            .map_err(|error| error.to_string())?
            .state
            == sovra_core::MemoryJobState::Cancelled
        {
            return Err("This original message is no longer available for processing.".into());
        }
        claim.revision = store
            .memory_revision()
            .await
            .map_err(|error| error.to_string())?;
        let now = crate::now();
        let query = MemorySearch {
            text: source.content.clone(),
            ..Default::default()
        };
        let (legs, recent) = tokio::try_join!(
            crate::recalling::keyword_candidates(store.as_ref(), &query, now),
            store.recall(RecallQuery::recent(8)),
        )
        .map_err(|_| "Earlier memories could not be read.".to_owned())?;
        let related = fuse_rrf(legs, 24).into_iter().map(|hit| hit.fact).collect();
        let known = consolidation_context(
            related,
            recent.into_iter().map(|hit| hit.fact).collect(),
            now,
        );
        let candidates = if known.is_empty() || extracted.candidates.is_empty() {
            extracted.candidates.clone()
        } else {
            harness
                .reconcile_memories(&source, &extracted.candidates, &known)
                .await
                .map_err(|error| error.to_string())?
        };
        match store
            .complete_memory(&claim, candidates, &extracted.model, crate::now())
            .await
        {
            Ok(_) => return Ok(()),
            Err(_) if store.memory_revision().await.ok() != Some(claim.revision) => {}
            Err(_) => return Err("Remembering could not save this result.".into()),
        }
    }
    Err("Saved details changed during processing. This message will retry.".into())
}

/// Assigned once by a producer; storage retries keep this identity unchanged.
pub(crate) fn source_key() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    format!("{stamp}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod scheduling;
