//! Owned training actor: blocking learner work, pause/resume and private checkpoints.
use std::{path::PathBuf, sync::Arc, time::Duration};

use async_channel::{Receiver, Sender};
use sovra_core::{
    Event,
    training::{
        MAX_CHECKPOINT_BYTES, TrainingCommand, TrainingEngine, TrainingEvent, TrainingPhase,
        TrainingSession, TrainingState,
    },
};
use tokio::task::JoinHandle;

mod grid;
pub(crate) use grid::PeerInbox;

use crate::private_storage::{
    private_directory, private_file, random_secret, read_private, read_private_bounded,
    write_private, write_private_bytes,
};

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
            let mut actor = Actor {
                config,
                events,
                session: None,
                filename: None,
                state: TrainingState::default(),
                storage_lock: None,
                clock: std::time::Instant::now(),
                groups,
                help_task: None,
                help_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                help_after: 0,
            };
            actor.run(incoming, peers).await;
        });
        Self { commands, task }
    }

    pub(crate) fn send(&self, command: TrainingCommand) {
        let _ = self.commands.try_send(Some(command));
    }

    pub(crate) async fn stop(self) {
        let _ = self.commands.send(None).await;
        let _ = self.task.await;
    }
}

struct Actor {
    config: Option<TrainingConfig>,
    events: Sender<Event>,
    session: Option<Box<dyn TrainingSession>>,
    filename: Option<String>,
    state: TrainingState,
    storage_lock: Option<std::fs::File>,
    clock: std::time::Instant,
    groups: crate::groups::GroupTraining,
    help_task: Option<JoinHandle<grid::HelpOutcome>>,
    help_cancel: Arc<std::sync::atomic::AtomicBool>,
    help_after: u64,
}

impl Actor {
    async fn publish(&mut self) {
        let _ = self
            .events
            .send(Event::Training(TrainingEvent(self.state.clone())))
            .await;
    }

    async fn run(
        &mut self,
        commands: Receiver<Option<TrainingCommand>>,
        peers: Receiver<grid::PeerWork>,
    ) {
        let configured = self.config.is_some();
        if let Some(config) = &self.config {
            let root = config.directory.clone();
            if let Ok(Ok(lock)) = tokio::task::spawn_blocking(move || acquire_lock(&root)).await {
                self.storage_lock = Some(lock);
            } else {
                self.state.error = Some("Training storage is unavailable or open in another Sovra window. Close that window and restart this one.".into());
                self.config = None;
            }
        }
        if self.config.is_some() {
            if let Err(error) = self.load().await {
                self.fail(error);
            }
        } else {
            self.state.phase = TrainingPhase::Unavailable;
        }
        if configured {
            self.publish().await;
        }
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                command = commands.recv() => {
                    match command {
                        Ok(Some(command)) => {
                            if let Err(error) = self.command(command).await { self.fail(error); }
                            self.publish().await;
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                incoming = peers.recv() => {
                    if let Ok(incoming) = incoming { self.receive_peer(incoming).await; }
                }
                result = async { self.help_task.as_mut().expect("guarded helper task").await }, if self.help_task.is_some() => {
                    self.finish_help(result);
                    self.publish().await;
                }
                _ = tick.tick() => {
                    // Give interactive work priority. This is app activity, not OS idle detection.
                    if !crate::model_priority::busy() {
                        if self.state.phase == TrainingPhase::Running {
                            if let Err(error) = self.advance().await { self.fail(error); }
                            self.publish().await;
                        }
                        if self.tick_help().await {
                            self.publish().await;
                        }
                    }
                    self.sync_help_status().await;
                }
            }
        }
        // Each acknowledged step is already durable. Never start extra work on quit.
        self.stop_help();
        if let Some(task) = self.help_task.take() {
            let _ = task.await;
        }
        self.groups.set_training_help(false).await;
        self.session = None;
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Group settings can revoke sharing, but only an explicit start can grant
    /// access to the selected run's text. A saved preference is not a new grant.
    async fn sync_help_status(&mut self) {
        let Some(shared_group) = self.state.sharing else {
            return;
        };
        let enabled = self.groups.training_help().await;
        let same_group = self
            .groups
            .snapshot()
            .await
            .is_some_and(|group| group.id == shared_group);
        if !enabled || !same_group {
            self.state.sharing = None;
            self.publish().await;
        }
    }

    fn fail(&mut self, error: String) {
        self.session = None;
        self.state.phase = if self.filename.is_none() {
            TrainingPhase::Ready
        } else if self.state.progress.is_complete() {
            TrainingPhase::Complete
        } else {
            TrainingPhase::Paused
        };
        self.state.error = Some(error);
        self.state.sharing = None;
    }

    async fn command(&mut self, command: TrainingCommand) -> Result<(), String> {
        if self.config.is_none() {
            return Ok(());
        }
        match command {
            TrainingCommand::Load => {
                self.state.group = self.groups.snapshot().await;
                if self.session.is_none() {
                    self.load().await?;
                }
            }
            TrainingCommand::Start(source) => self.start(source, None).await?,
            TrainingCommand::StartShared(source) => {
                if self.state.busy() {
                    return Ok(());
                }
                let group = self
                    .groups
                    .snapshot()
                    .await
                    .ok_or("Create or join a group first.")?;
                let id = group.id;
                self.state.group = Some(group);
                self.start(source, Some(id)).await?;
            }
            TrainingCommand::StopSharing => {
                self.groups.set_training_help(false).await;
                self.state.sharing = None;
            }
            TrainingCommand::Help(device) => self.start_help(device).await?,
            TrainingCommand::Pause => {
                self.stop_help();
                if self.state.phase == TrainingPhase::Running {
                    self.state.phase = TrainingPhase::Paused;
                }
            }
            TrainingCommand::Resume => {
                if self.state.busy() || self.state.phase != TrainingPhase::Paused {
                    return Ok(());
                }
                // Reload the durable model after any previous write/worker failure.
                self.load().await?;
                if self.state.phase == TrainingPhase::Paused {
                    self.state.phase = TrainingPhase::Running;
                }
            }
        }
        Ok(())
    }

    async fn start(
        &mut self,
        source: sovra_core::training::TrainingSource,
        sharing: Option<sovra_core::group::GroupId>,
    ) -> Result<(), String> {
        if self.state.busy() {
            return Ok(());
        }
        let text = source.text().map_err(|e| e.to_string())?.to_owned();
        self.state.phase = TrainingPhase::Preparing;
        self.state.error = None;
        self.publish().await;
        let engine = self
            .config
            .as_ref()
            .expect("configured above")
            .engine
            .clone();
        let run = random_secret()?;
        let filename = format!("run-{run}.json");
        let run = sovra_core::group::ContentHash::of(run.as_bytes());
        let session = tokio::task::spawn_blocking(move || engine.create(&text, run))
            .await
            .map_err(|_| "Training could not start.".to_owned())?
            .map_err(|e| e.to_string())?;
        let (session, bytes) = encode(session).await?;
        let root = &self.config.as_ref().expect("configured above").directory;
        // Every run gets its own file. Starting again preserves older models.
        write_private_bytes(root, &filename, bytes).await?;
        write_private(root, "latest.json", &filename).await?;
        self.state.progress = session.progress();
        self.state.saved_at = Some(root.join(&filename).display().to_string());
        self.filename = Some(filename);
        self.session = Some(session);
        self.state.phase = TrainingPhase::Running;
        self.state.sharing = sharing;
        self.state.helped_rounds = 0;
        self.state.helping = None;
        self.groups.set_training_help(sharing.is_some()).await;
        Ok(())
    }

    async fn load(&mut self) -> Result<(), String> {
        let config = self.config.as_ref().expect("only called when configured");
        let root = config.directory.clone();
        let exists = tokio::fs::try_exists(root.join("latest.json"))
            .await
            .map_err(|_| "Could not read saved training.".to_owned())?;
        if !exists {
            self.state.phase = TrainingPhase::Ready;
            self.state.error = None;
            return Ok(());
        }
        let bytes = read_private(&root, "latest.json").await?;
        let filename: String = serde_json::from_slice(&bytes)
            .map_err(|_| "Saved training index is damaged.".to_owned())?;
        if !valid_filename(&filename) {
            return Err("Saved training filename is invalid.".into());
        }
        let bytes = read_private_bounded(&root, &filename, MAX_CHECKPOINT_BYTES as u64).await?;
        let engine = config.engine.clone();
        let session = tokio::task::spawn_blocking(move || engine.restore(&bytes))
            .await
            .map_err(|_| "Could not restore training.".to_owned())?
            .map_err(|e| e.to_string())?;
        self.state.progress = session.progress();
        self.state.phase = if self.state.progress.is_complete() {
            TrainingPhase::Complete
        } else {
            TrainingPhase::Paused
        };
        self.state.saved_at = Some(root.join(&filename).display().to_string());
        self.state.error = None;
        self.filename = Some(filename);
        self.session = Some(session);
        Ok(())
    }

    async fn advance(&mut self) -> Result<(), String> {
        let Some(mut session) = self.session.take() else {
            return Err("Resume the saved training to continue.".into());
        };
        let now_ms = self.now_ms();
        let (session, bytes) = tokio::task::spawn_blocking(move || {
            session.advance(now_ms).map_err(|e| e.to_string())?;
            let bytes = session.checkpoint().map_err(|e| e.to_string())?;
            Ok::<_, String>((session, bytes))
        })
        .await
        .map_err(|_| {
            "The training worker stopped. Resume from the last saved step.".to_owned()
        })??;
        self.save(session, bytes).await
    }

    async fn save(
        &mut self,
        session: Box<dyn TrainingSession>,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let config = self.config.as_ref().expect("only running when configured");
        let filename = self
            .filename
            .as_ref()
            .ok_or("Training has no saved file.")?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err("Training checkpoint exceeds its limit.".into());
        }
        write_private_bytes(&config.directory, filename, bytes).await?;
        // Never announce progress before both model and optimizer have reached disk.
        self.state.progress = session.progress();
        self.state.error = None;
        if self.state.progress.is_complete() {
            self.state.phase = TrainingPhase::Complete;
            self.state.sharing = None;
        }
        self.session = Some(session);
        Ok(())
    }
}

async fn encode(
    session: Box<dyn TrainingSession>,
) -> Result<(Box<dyn TrainingSession>, Vec<u8>), String> {
    tokio::task::spawn_blocking(move || {
        let bytes = session.checkpoint().map_err(|e| e.to_string())?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err("Training checkpoint exceeds its limit.".into());
        }
        Ok((session, bytes))
    })
    .await
    .map_err(|_| "Could not save training.".to_owned())?
}

fn valid_filename(name: &str) -> bool {
    name.strip_prefix("run-")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn acquire_lock(root: &std::path::Path) -> Result<std::fs::File, String> {
    private_directory(root)?;
    let path = root.join("owner.lock");
    if path.exists() {
        private_file(&path)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| "Cannot open training lock.".to_owned())?;
    private_file(&path)?;
    file.try_lock()
        .map_err(|_| "Training is open in another window.".to_owned())?;
    Ok(file)
}

#[cfg(test)]
mod tests;
