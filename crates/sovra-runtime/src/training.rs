//! Owned training actor.
//!
//! One task owns the run, its saved files and the model library. The learning
//! itself happens on blocking tasks the actor owns: a run trains in short
//! slices, and between two slices the actor answers commands and the group, so
//! Pause takes effect within one round and a chat turn is never kept waiting.
//!
//! - `run`: starting, slices, checkpoints, pause, stop and finishing.
//! - `library`: finished models on disk, trying one out, sharing one.
//! - `grid`: the group's side: serving work for a shared run, helping another.
//! - `auto`: finding a shared run to help when the person allows it.
//! - `delivery`: giving finished models to the group, and receiving them.
//! - `storage`: file names and bounded readers.
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use async_channel::{Receiver, Sender};
use sovra_core::{
    Event,
    group::DeviceId,
    training::{
        HardwareInfo, ModelId, TrainingCommand, TrainingEngine, TrainingEvent, TrainingPhase,
        TrainingSession, TrainingState,
    },
};
use tokio::task::{JoinError, JoinHandle, JoinSet};

mod auto;
mod delivery;
mod grid;
mod library;
mod run;
mod storage;
pub(crate) use grid::PeerInbox;

/// How often the page is told about progress while a run trains.
const PUBLISH_EVERY_MS: u64 = 250;
/// How often the actor looks at things nothing else wakes it for.
const TICK: Duration = Duration::from_millis(250);

/// The composition root supplies both the implementation and its storage location.
pub struct TrainingConfig {
    pub engine: Arc<dyn TrainingEngine>,
    pub directory: PathBuf,
}

impl std::fmt::Debug for TrainingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrainingConfig").finish_non_exhaustive()
    }
}

pub(crate) struct TrainingRuntime {
    commands: Sender<Option<TrainingCommand>>,
    task: JoinHandle<()>,
}

impl TrainingRuntime {
    pub(crate) fn start(
        config: Option<TrainingConfig>,
        events: Sender<Event>,
        groups: crate::groups::GroupTraining,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, incoming) = async_channel::unbounded();
        let (inbox, peers) = PeerInbox::new();
        groups.attach(inbox);
        let task = handle.spawn(async move {
            let mut actor = Actor::new(config, events, groups);
            actor.run(incoming, peers).await;
        });
        Self { commands, task }
    }

    pub(crate) fn send(&self, command: TrainingCommand) {
        // The channel has no limit, so this only fails once the runtime is
        // stopping and nothing can act on the command any more.
        if self.commands.try_send(Some(command)).is_err() {
            tracing::warn!("a training command arrived after training had stopped");
        }
    }

    pub(crate) async fn stop(self) {
        let _ = self.commands.send(None).await;
        let _ = self.task.await;
    }
}

/// What to do with the run once the slice that is out comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    Continue,
    Pause,
    Stop,
}

/// How the current run ended, kept beside it on disk.
#[derive(Debug, Clone, Copy, Default)]
struct Ending {
    /// The run ended before its last round: Stop, or it stopped improving.
    stopped: bool,
    /// The library model the run was saved as.
    exported: Option<ModelId>,
}

struct Actor {
    config: Option<TrainingConfig>,
    events: Sender<Event>,
    state: TrainingState,
    storage_lock: Option<std::fs::File>,
    clock: std::time::Instant,
    groups: crate::groups::GroupTraining,

    /// The run, while no slice has it.
    session: Option<Box<dyn TrainingSession>>,
    /// The run's file inside the training folder.
    filename: Option<String>,
    /// The blocking task that has the run right now.
    slice: Option<JoinHandle<run::Slice>>,
    /// Asks the slice to hand the run back after its current round.
    halt: Arc<AtomicBool>,
    wanted: Wanted,
    /// Rounds have been done that are not on disk yet.
    dirty: bool,
    saved_ms: u64,
    /// Time between checkpoints, kept well above what one costs to write.
    save_every_ms: u64,
    published_ms: u64,
    /// Progress changed since the page was last told.
    unpublished: bool,
    ending: Ending,
    /// The saved run could not be reopened at start. Do not retry on every Load.
    unreadable: bool,

    /// Group requests that need the run while a slice has it.
    waiting: VecDeque<grid::PeerWork>,
    /// Helpers holding a work packet of this run, and when each took it.
    leases: BTreeMap<DeviceId, u64>,
    /// Rounds each helper has given this run.
    peer_rounds: BTreeMap<DeviceId, u32>,
    /// Packets each helper took in a row without returning one, and when it
    /// may next be given another once that reaches the limit.
    unanswered: BTreeMap<DeviceId, (u32, u64)>,
    help_task: Option<JoinHandle<grid::HelpOutcome>>,
    help_cancel: Arc<AtomicBool>,
    help_after: u64,
    /// What "Help the group train" has this computer doing by itself.
    auto: auto::AutoHelp,
    /// Looking for a shared run among the group's computers.
    scout_task: Option<JoinHandle<auto::Scouted>>,
    scout_after: u64,
    scout_round: u32,
    /// Computers on a Sovra that does not know the newer questions, and when
    /// each may be asked again.
    too_old: BTreeMap<DeviceId, u64>,

    /// Asking the group's computers which finished models they offer.
    poll_task: Option<JoinHandle<delivery::Polled>>,
    poll_after: u64,
    poll_round: u32,
    /// The one model being fetched from the group right now.
    fetch_task: Option<JoinHandle<delivery::Fetched>>,
    /// Offered models still to fetch, oldest hint first.
    offered: VecDeque<delivery::Offer>,
    /// Models that could not be taken, and when each may be tried again.
    declined: BTreeMap<ModelId, delivery::Declined>,
    /// The model file last served to the group, kept briefly so a model that
    /// leaves in several parts is read once.
    served: Option<delivery::Served>,
    /// Small owned sends: telling the group about a model, saying "received".
    chores: JoinSet<()>,
    chore_gate: Arc<tokio::sync::Semaphore>,

    hardware_task: Option<JoinHandle<HardwareInfo>>,
    trial_task: Option<JoinHandle<Result<String, String>>>,
    trial_seed: u64,
}

/// Waits for an owned task if there is one, and forgets it once it is done.
/// With no task it never completes, so a `select!` arm on it stays quiet.
async fn joined<T>(task: &mut Option<JoinHandle<T>>) -> Result<T, JoinError> {
    match task {
        Some(handle) => {
            let result = handle.await;
            *task = None;
            result
        }
        None => std::future::pending().await,
    }
}

impl Actor {
    fn new(
        config: Option<TrainingConfig>,
        events: Sender<Event>,
        groups: crate::groups::GroupTraining,
    ) -> Self {
        Self {
            config,
            events,
            state: TrainingState::default(),
            storage_lock: None,
            clock: std::time::Instant::now(),
            groups,
            session: None,
            filename: None,
            slice: None,
            halt: Arc::new(AtomicBool::new(false)),
            wanted: Wanted::Continue,
            dirty: false,
            saved_ms: 0,
            save_every_ms: run::MIN_SAVE_EVERY_MS,
            published_ms: 0,
            unpublished: false,
            ending: Ending::default(),
            unreadable: false,
            waiting: VecDeque::new(),
            leases: BTreeMap::new(),
            peer_rounds: BTreeMap::new(),
            unanswered: BTreeMap::new(),
            help_task: None,
            help_cancel: Arc::new(AtomicBool::new(false)),
            help_after: 0,
            auto: auto::AutoHelp::default(),
            scout_task: None,
            scout_after: 0,
            scout_round: 0,
            too_old: BTreeMap::new(),
            poll_task: None,
            poll_after: 0,
            poll_round: 0,
            fetch_task: None,
            offered: VecDeque::new(),
            declined: BTreeMap::new(),
            served: None,
            chores: JoinSet::new(),
            chore_gate: Arc::new(tokio::sync::Semaphore::new(delivery::FAN_OUT)),
            hardware_task: None,
            trial_task: None,
            trial_seed: 0,
        }
    }

    async fn publish(&mut self) {
        self.published_ms = self.now_ms();
        self.unpublished = false;
        let _ = self
            .events
            .send(Event::Training(TrainingEvent::new(self.state.clone())))
            .await;
    }

    /// Progress of a running run: told to the page a few times a second at most.
    async fn publish_progress(&mut self) {
        if self.now_ms().saturating_sub(self.published_ms) >= PUBLISH_EVERY_MS {
            self.publish().await;
        } else {
            self.unpublished = true;
        }
    }

    async fn run(
        &mut self,
        commands: Receiver<Option<TrainingCommand>>,
        peers: Receiver<grid::PeerWork>,
    ) {
        let configured = self.config.is_some();
        self.open().await;
        if configured {
            self.publish().await;
        }
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                command = commands.recv() => {
                    match command {
                        Ok(Some(command)) => {
                            self.command(command).await;
                            self.publish().await;
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                incoming = peers.recv() => {
                    if let Ok(incoming) = incoming { self.receive_peer(incoming).await; }
                }
                result = joined(&mut self.slice) => self.finish_slice(result).await,
                result = joined(&mut self.help_task) => {
                    self.finish_help(result);
                    // The next packet is asked for at once, not on the next tick.
                    if !crate::model_priority::busy() {
                        self.tick_help().await;
                    }
                    self.publish().await;
                }
                result = joined(&mut self.scout_task) => {
                    if self.finish_scout(result).await {
                        self.publish().await;
                    }
                }
                result = joined(&mut self.poll_task) => {
                    if self.finish_poll(result).await {
                        self.publish().await;
                    }
                }
                result = joined(&mut self.fetch_task) => {
                    self.finish_fetch(result).await;
                    self.publish().await;
                }
                Some(_) = self.chores.join_next() => {}
                result = joined(&mut self.trial_task) => {
                    self.finish_trial(result);
                    self.publish().await;
                }
                result = joined(&mut self.hardware_task) => {
                    if let Ok(hardware) = result {
                        self.state.hardware = Some(hardware.bounded());
                        self.publish().await;
                    }
                }
                _ = tick.tick() => self.tick().await,
            }
        }
        self.close().await;
    }

    /// Takes the storage lock, reopens the saved run and reads the library.
    async fn open(&mut self) {
        if let Some(config) = &self.config {
            let root = config.directory.clone();
            if let Ok(Ok(lock)) =
                tokio::task::spawn_blocking(move || storage::acquire_lock(&root)).await
            {
                self.storage_lock = Some(lock);
            } else {
                self.state.error = Some("Training storage is unavailable or open in another Sovra window. Close that window and restart this one.".into());
                self.config = None;
            }
        }
        if self.config.is_none() {
            self.state.phase = TrainingPhase::Unavailable;
            return;
        }
        self.probe_hardware();
        if let Err(error) = self.load().await {
            // The file is kept. A new run can start; this one is not retried on
            // every visit to the page.
            self.unreadable = true;
            self.fail(error);
        }
        if let Err(error) = self.load_library().await {
            self.state.error.get_or_insert(error);
        }
    }

    /// Hands every owned task back and saves the run before the app goes away.
    /// No new round is started.
    async fn close(&mut self) {
        self.halt.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(slice) = self.slice.take()
            && let Ok(slice) = slice.await
        {
            self.absorb(slice);
        }
        if self.session.is_some()
            && self.dirty
            && let Err(error) = self.save_now().await
        {
            tracing::warn!(%error, "the last training rounds could not be saved on quit");
        }
        self.stop_help();
        if let Some(task) = self.help_task.take() {
            let _ = task.await;
        }
        // These only wait on the network, so they are cut short.
        if let Some(task) = self.scout_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.poll_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.fetch_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.chores.shutdown().await;
        if let Some(task) = self.trial_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.hardware_task.take() {
            let _ = task.await;
        }
        self.waiting.clear();
        self.session = None;
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn engine(&self) -> Option<Arc<dyn TrainingEngine>> {
        self.config.as_ref().map(|config| config.engine.clone())
    }

    fn root(&self) -> Option<PathBuf> {
        self.config.as_ref().map(|config| config.directory.clone())
    }

    /// Asks the engine about this computer on a blocking task. One at a time.
    fn probe_hardware(&mut self) {
        if self.hardware_task.is_some() {
            return;
        }
        if let Some(engine) = self.engine() {
            self.hardware_task = Some(tokio::task::spawn_blocking(move || engine.hardware()));
        }
    }

    async fn tick(&mut self) {
        self.sync_sharing().await;
        if self.session.is_some() {
            self.sweep_helpers().await;
        }
        let idle = !crate::model_priority::busy();
        if self.state.phase == TrainingPhase::Running && self.slice.is_none() {
            if idle {
                // Interactive work comes first. This is app activity, not OS idle
                // detection.
                self.spawn_slice();
            } else if self.dirty
                && self.now_ms().saturating_sub(self.saved_ms) >= self.save_every_ms
                && let Err(error) = self.save_now().await
            {
                self.fail(error);
                self.publish().await;
            }
        }
        let mut changed = self.tick_auto(idle).await;
        if idle && self.tick_help().await {
            changed = true;
        }
        if self.tick_delivery().await {
            changed = true;
        }
        if changed {
            self.publish().await;
        }
        if self.unpublished && self.now_ms().saturating_sub(self.published_ms) >= PUBLISH_EVERY_MS {
            self.publish().await;
        }
    }

    /// The run itself broke. The in-memory copy is dropped and the last saved
    /// step stays on disk; Resume reopens it.
    fn fail(&mut self, error: String) {
        self.session = None;
        self.wanted = Wanted::Continue;
        self.dirty = false;
        self.state.phase = if self.filename.is_none() {
            TrainingPhase::Ready
        } else if self.ending.stopped || self.state.progress.is_complete() {
            TrainingPhase::Complete
        } else {
            TrainingPhase::Paused
        };
        self.state.error = Some(error);
        self.drop_sharing();
        // Dropping a waiting request answers it "unavailable".
        self.waiting.clear();
    }

    async fn command(&mut self, command: TrainingCommand) {
        if self.config.is_none() {
            return;
        }
        if !matches!(command, TrainingCommand::Load) {
            self.state.error = None;
            self.state.notice = None;
        }
        // A refused command changes nothing but the message.
        if let Err(error) = self.handle(command).await {
            self.state.error = Some(error);
        }
    }

    async fn handle(&mut self, command: TrainingCommand) -> Result<(), String> {
        command.validate().map_err(|error| error.to_string())?;
        match command {
            TrainingCommand::Load => self.reload().await,
            TrainingCommand::Start(plan) => self.start(*plan).await,
            TrainingCommand::Pause => self.pause().await,
            TrainingCommand::Resume => self.resume().await,
            TrainingCommand::Stop => self.stop().await,
            TrainingCommand::StopHelping => {
                self.stop_helping();
                Ok(())
            }
            TrainingCommand::Help(device) => self.start_help(device).await,
            TrainingCommand::Try { model, prompt } => self.try_model(model, prompt),
            TrainingCommand::Share(model) => self.share(model).await,
            TrainingCommand::StopSharing => {
                self.drop_sharing();
                Ok(())
            }
        }
    }

    /// The page was opened: bring everything it shows up to date.
    async fn reload(&mut self) -> Result<(), String> {
        self.refresh_group().await;
        self.probe_hardware();
        if self.session.is_none()
            && self.slice.is_none()
            && !self.unreadable
            && let Err(error) = self.load().await
        {
            self.unreadable = true;
            self.fail(error);
        }
        self.load_library().await?;
        self.export_if_owed().await;
        // Look for models the group is offering now, on every computer of it.
        self.poll_soon();
        self.sync_deliveries().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
