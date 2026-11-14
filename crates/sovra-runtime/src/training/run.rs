//! One run on this computer: start, train in slices, save, pause, stop, finish.

use std::{
    sync::{Arc, atomic::AtomicBool, atomic::Ordering},
    time::{Duration, Instant},
};

use sovra_core::{
    group::ContentHash,
    training::{
        MAX_CHECKPOINT_BYTES, MAX_METRIC_POINTS, MetricPoint, TrainingBase, TrainingError,
        TrainingMetrics, TrainingPhase, TrainingPlan, TrainingSession, TrainingSource,
    },
};
use tokio::task::JoinError;

use super::{Actor, Wanted, storage};
use crate::private_storage::{
    random_secret, read_private, read_private_bounded, write_private, write_private_bytes,
};

/// How long one slice trains before the actor looks at the run again. Short
/// enough for the page to feel live, long enough that a fast model is not
/// slowed by the hand-over.
const SLICE: Duration = Duration::from_millis(200);
/// A crash replays at most about this much training.
pub(super) const MIN_SAVE_EVERY_MS: u64 = 3_000;
const MAX_SAVE_EVERY_MS: u64 = 60_000;
/// A checkpoint of a large model takes a while to write. Saving no more often
/// than this many times its cost keeps saving to a few percent of the run.
const SAVE_COST_FACTOR: u64 = 25;

const WORKER_STOPPED: &str = "The training worker stopped. Resume from the last saved step.";

/// A checkpoint a slice made, or why it could not, and what encoding it cost.
type Encoded = (Result<Vec<u8>, TrainingError>, u64);

/// What a slice brings back with the run.
pub(super) struct Slice {
    session: Box<dyn TrainingSession>,
    rounds: u32,
    /// One point per round, at most [`MAX_METRIC_POINTS`].
    points: Vec<MetricPoint>,
    worked_ms: u64,
    error: Option<TrainingError>,
    /// The run as it is at the end of the slice, when one was due.
    checkpoint: Option<Result<Vec<u8>, TrainingError>>,
    encode_ms: u64,
}

/// Trains until the slice is used up, the run is complete, the actor asks for
/// the run back or a chat turn starts. Always finishes the round it is on.
fn train(
    mut session: Box<dyn TrainingSession>,
    clock: Instant,
    halt: &AtomicBool,
    checkpoint: bool,
) -> Slice {
    let started = Instant::now();
    let mut rounds = 0_u32;
    let mut points = Vec::new();
    let mut error = None;
    while !halt.load(Ordering::SeqCst)
        && !crate::model_priority::busy()
        && !session.progress().is_complete()
        && points.len() < MAX_METRIC_POINTS
    {
        let now_ms = u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Err(failed) = session.advance(now_ms) {
            error = Some(failed);
            break;
        }
        rounds = rounds.saturating_add(1);
        points.extend(session.metrics());
        if started.elapsed() >= SLICE {
            break;
        }
    }
    let worked_ms = millis(started);
    let encoding = Instant::now();
    let due = checkpoint || (rounds > 0 && session.progress().is_complete());
    let checkpoint = (error.is_none() && due).then(|| session.checkpoint());
    Slice {
        session,
        rounds,
        points,
        worked_ms,
        error,
        checkpoint,
        encode_ms: millis(encoding),
    }
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl Actor {
    /// Reopens the saved run, if there is one, as paused or complete. It never
    /// starts training by itself, and it never restores sharing with the group.
    pub(super) async fn load(&mut self) -> Result<(), String> {
        let (Some(root), Some(engine)) = (self.root(), self.engine()) else {
            return Ok(());
        };
        let exists = tokio::fs::try_exists(root.join(storage::LATEST))
            .await
            .map_err(|_| "Could not read saved training.".to_owned())?;
        if !exists {
            self.state.phase = TrainingPhase::Ready;
            return Ok(());
        }
        let bytes = read_private(&root, storage::LATEST).await?;
        let filename: String = serde_json::from_slice(&bytes)
            .map_err(|_| "Saved training index is damaged.".to_owned())?;
        if !storage::valid_filename(&filename) {
            return Err("Saved training filename is invalid.".into());
        }
        let bytes = read_private_bounded(&root, &filename, MAX_CHECKPOINT_BYTES as u64).await?;
        let notes_root = root.clone();
        let notes_file = filename.clone();
        let (session, notes) = tokio::task::spawn_blocking(move || {
            let session = engine.restore(&bytes)?;
            Ok::<_, TrainingError>((session, storage::read_notes(&notes_root, &notes_file)))
        })
        .await
        .map_err(|_| "Could not restore training.".to_owned())?
        .map_err(|error| error.to_string())?;

        let progress = session.progress();
        // History from a different moment of the run than the model would draw
        // a chart of rounds that were never kept.
        let mut metrics = if notes
            .metrics
            .latest()
            .is_none_or(|point| point.round <= progress.rounds)
        {
            notes.metrics
        } else {
            TrainingMetrics::default()
        };
        metrics.eta_ms = None;
        if let Some(point) = session.metrics() {
            metrics.push(point);
        }
        if let Some((round, text)) = session.sample() {
            metrics.set_sample(text.as_bytes(), round);
        }
        self.ending.stopped = notes.stopped;
        self.ending.exported = notes.exported;
        self.state.phase = if progress.is_complete() || self.ending.stopped {
            TrainingPhase::Complete
        } else {
            TrainingPhase::Paused
        };
        self.state.progress = progress;
        self.state.plan = Some(session.plan());
        self.state.metrics = metrics;
        self.state.saved_at = Some(root.join(&filename).display().to_string());
        self.filename = Some(filename);
        self.session = Some(session);
        self.dirty = false;
        self.saved_ms = self.now_ms();
        self.unreadable = false;
        Ok(())
    }

    pub(super) async fn start(&mut self, plan: TrainingPlan) -> Result<(), String> {
        if self.state.phase.busy() || self.slice.is_some() || self.held_by_help() {
            self.state.notice = Some(
                if self.state.phase.busy() {
                    "Training is already running. Pause or stop it before starting another."
                } else {
                    "This computer is helping your group train. Stop helping before starting your own training."
                }
                .into(),
            );
            return Ok(());
        }
        let (Some(root), Some(engine)) = (self.root(), self.engine()) else {
            return Ok(());
        };
        let (architecture, base) = match plan.base {
            TrainingBase::Fresh(model) => (model.architecture(), None),
            TrainingBase::Continue(model) => {
                if self.state.model(model).is_none() {
                    return Err(
                        "That model is no longer in Your models. Choose another starting point."
                            .into(),
                    );
                }
                let (facts, file) = self.read_library_model(model).await?;
                (facts.header.architecture, Some(file))
            }
        };
        plan.validate(architecture)
            .map_err(|error| error.to_string())?;
        let group = if plan.with_group {
            Some(
                self.groups
                    .snapshot()
                    .await
                    .ok_or("Create or join a group first.")?,
            )
        } else {
            None
        };
        let text = if let Some(text) = plan.source.text().map_err(|error| error.to_string())? {
            text.to_owned()
        } else {
            let TrainingSource::File(path) = plan.source.clone() else {
                return Err(TrainingError::InvalidInput.to_string());
            };
            tokio::task::spawn_blocking(move || storage::read_text_file(&path))
                .await
                .map_err(|_| "Sovra could not read that file.".to_owned())??
        };
        // The person's own training comes before help Sovra started by itself.
        self.yield_auto_help();
        // The run being replaced keeps its file; what it learned so far also
        // goes to Your models, so starting again never loses a model.
        self.keep_unfinished().await;

        let before = self.state.phase;
        self.state.phase = TrainingPhase::Preparing;
        self.publish().await;
        let created = self.create(&root, engine, plan, text, base).await;
        let (filename, session) = match created {
            Ok(created) => created,
            Err(error) => {
                self.state.phase = before;
                return Err(error);
            }
        };
        self.state.progress = session.progress();
        self.state.plan = Some(session.plan());
        self.state.metrics = TrainingMetrics::default();
        if let Some(point) = session.metrics() {
            self.state.metrics.push(point);
        }
        self.state.saved_at = Some(root.join(&filename).display().to_string());
        self.state.phase = TrainingPhase::Running;
        self.state.helped_rounds = 0;
        self.state.helping = None;
        self.filename = Some(filename);
        self.session = Some(session);
        self.wanted = Wanted::Continue;
        self.dirty = false;
        self.saved_ms = self.now_ms();
        self.save_every_ms = MIN_SAVE_EVERY_MS;
        self.ending = super::Ending::default();
        self.unreadable = false;
        self.leases.clear();
        self.peer_rounds.clear();
        self.unanswered.clear();
        // Only this explicit start lets the group see pieces of the text.
        self.state.sharing = group.as_ref().map(|group| group.id);
        if group.is_some() {
            self.state.group = group;
        }
        self.apply_peer_status();
        self.write_notes().await?;
        self.spawn_slice();
        Ok(())
    }

    /// Creates the run and puts its first checkpoint on disk before anything
    /// says it exists.
    async fn create(
        &mut self,
        root: &std::path::Path,
        engine: Arc<dyn sovra_core::training::TrainingEngine>,
        plan: TrainingPlan,
        text: String,
        base: Option<Vec<u8>>,
    ) -> Result<(String, Box<dyn TrainingSession>), String> {
        let run = random_secret()?;
        let filename = storage::run_filename(&run);
        let run = ContentHash::of(run.as_bytes());
        let (session, bytes) = tokio::task::spawn_blocking(move || {
            let session = engine.create(&plan, &text, base.as_deref(), run)?;
            let bytes = session.checkpoint()?;
            Ok::<_, TrainingError>((session, bytes))
        })
        .await
        .map_err(|_| "Training could not start.".to_owned())?
        .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err("Training checkpoint exceeds its limit.".into());
        }
        // Every run gets its own file. Starting again preserves older runs.
        write_private_bytes(root, &filename, bytes).await?;
        write_private(root, storage::LATEST, &filename).await?;
        Ok((filename, session))
    }

    /// Sends the run out to train for one slice.
    pub(super) fn spawn_slice(&mut self) {
        let Some(session) = self.session.take() else {
            self.fail("Resume the saved training to continue.".into());
            return;
        };
        self.halt.store(false, Ordering::SeqCst);
        let halt = self.halt.clone();
        let clock = self.clock;
        let checkpoint =
            self.dirty && self.now_ms().saturating_sub(self.saved_ms) >= self.save_every_ms;
        self.slice = Some(tokio::task::spawn_blocking(move || {
            train(session, clock, &halt, checkpoint)
        }));
    }

    /// Takes the run back from a slice and folds in what it measured.
    pub(super) fn absorb(&mut self, slice: Slice) -> (Option<TrainingError>, Option<Encoded>) {
        let Slice {
            session,
            rounds,
            points,
            worked_ms,
            error,
            checkpoint,
            encode_ms,
        } = slice;
        if rounds > 0 {
            self.dirty = true;
        }
        for point in points {
            self.state.metrics.push(point);
        }
        self.session = Some(session);
        self.observe(worked_ms);
        (error, checkpoint.map(|bytes| (bytes, encode_ms)))
    }

    /// Brings progress, the sample and the time left in line with the run.
    pub(super) fn observe(&mut self, worked_ms: u64) {
        let Some(session) = &self.session else {
            return;
        };
        let progress = session.progress();
        if let Some((round, text)) = session.sample()
            && (self.state.metrics.sample.is_none() || round != self.state.metrics.sample_round)
        {
            self.state.metrics.set_sample(text.as_bytes(), round);
        }
        // Time left assumes the rounds to come cost what the rounds so far did.
        let per_round = progress.tokens / u64::from(progress.rounds.max(1));
        let remaining = u64::from(progress.total_rounds.saturating_sub(progress.rounds))
            .saturating_mul(per_round);
        let elapsed = self.state.metrics.elapsed_ms.saturating_add(worked_ms);
        self.state.metrics.observe(elapsed, remaining);
        self.state.progress = progress;
    }

    pub(super) async fn finish_slice(&mut self, result: Result<Slice, JoinError>) {
        let before = self.state.phase;
        match result {
            Ok(slice) => self.settle_slice(slice).await,
            Err(_) => self.fail(WORKER_STOPPED.into()),
        }
        if self.state.phase == before {
            self.publish_progress().await;
        } else {
            self.publish().await;
        }
    }

    async fn settle_slice(&mut self, slice: Slice) {
        let (error, checkpoint) = self.absorb(slice);
        if let Some(error) = error {
            self.fail(error.to_string());
            return;
        }
        if let Some((bytes, encode_ms)) = checkpoint {
            let written = match bytes {
                Ok(bytes) => self.write_checkpoint(bytes, encode_ms).await,
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = written {
                self.fail(error);
                return;
            }
        }
        let wanted = std::mem::replace(&mut self.wanted, Wanted::Continue);
        if self.state.progress.is_complete() {
            self.finish(None).await;
        } else if self.stalled() {
            self.finish(Some(
                "Training finished early because the check stopped improving.".into(),
            ))
            .await;
        } else {
            match wanted {
                Wanted::Stop => self.finish(None).await,
                Wanted::Pause => self.settle_paused().await,
                Wanted::Continue => {}
            }
        }
        self.serve_waiting().await;
        self.sweep_helpers().await;
        if self.state.phase == TrainingPhase::Running
            && self.session.is_some()
            && !crate::model_priority::busy()
        {
            self.spawn_slice();
        }
    }

    /// Whether the plan asks to finish early and the check has stopped improving.
    pub(super) fn stalled(&self) -> bool {
        self.state.plan.as_ref().is_some_and(|plan| {
            self.state
                .metrics
                .stalled(self.state.progress.rounds, plan.settings.stop_when_stalled)
        })
    }

    /// Puts the run on disk now. The run must be here, not out on a slice.
    pub(super) async fn save_now(&mut self) -> Result<(), String> {
        let session = self
            .session
            .take()
            .ok_or("Resume the saved training to continue.")?;
        let started = Instant::now();
        let (session, bytes) = tokio::task::spawn_blocking(move || {
            let bytes = session.checkpoint();
            (session, bytes)
        })
        .await
        .map_err(|_| WORKER_STOPPED.to_owned())?;
        self.session = Some(session);
        let bytes = bytes.map_err(|error| error.to_string())?;
        self.write_checkpoint(bytes, millis(started)).await
    }

    async fn write_checkpoint(&mut self, bytes: Vec<u8>, encode_ms: u64) -> Result<(), String> {
        let root = self.root().ok_or("Training is unavailable.")?;
        let filename = self.filename.clone().ok_or("Training has no saved file.")?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err("Training checkpoint exceeds its limit.".into());
        }
        let started = Instant::now();
        write_private_bytes(&root, &filename, bytes).await?;
        self.write_notes().await?;
        let cost = encode_ms.saturating_add(millis(started));
        self.save_every_ms = cost
            .saturating_mul(SAVE_COST_FACTOR)
            .clamp(MIN_SAVE_EVERY_MS, MAX_SAVE_EVERY_MS);
        self.dirty = false;
        self.saved_ms = self.now_ms();
        Ok(())
    }

    /// Saves the chart history and how the run ended beside the run's file.
    pub(super) async fn write_notes(&mut self) -> Result<(), String> {
        let (Some(root), Some(name)) = (
            self.root(),
            self.filename.as_deref().and_then(storage::notes_filename),
        ) else {
            return Ok(());
        };
        let notes = storage::RunNotes {
            metrics: self.state.metrics.clone(),
            stopped: self.ending.stopped,
            exported: self.ending.exported,
        };
        write_private(&root, &name, &notes).await
    }

    pub(super) async fn pause(&mut self) -> Result<(), String> {
        if self.state.phase != TrainingPhase::Running {
            return Ok(());
        }
        if self.slice.is_some() {
            // The round in progress finishes and is saved; then it is paused.
            self.wanted = Wanted::Pause;
            self.halt.store(true, Ordering::SeqCst);
            self.state.phase = TrainingPhase::Pausing;
        } else {
            self.settle_paused().await;
        }
        Ok(())
    }

    async fn settle_paused(&mut self) {
        if self.dirty
            && let Err(error) = self.save_now().await
        {
            self.fail(error);
            return;
        }
        self.state.phase = TrainingPhase::Paused;
    }

    pub(super) async fn resume(&mut self) -> Result<(), String> {
        if self.state.phase != TrainingPhase::Paused {
            return Ok(());
        }
        if self.held_by_help() {
            self.state.notice = Some(
                "This computer is helping your group train. Stop helping before resuming your own training."
                    .into(),
            );
            return Ok(());
        }
        self.yield_auto_help();
        if self.session.is_none() {
            // Reopen the saved run after an earlier failure.
            self.load().await?;
        }
        if self.state.phase == TrainingPhase::Paused {
            self.state.phase = TrainingPhase::Running;
            self.spawn_slice();
        }
        Ok(())
    }

    /// Finish now and keep the model as it is.
    pub(super) async fn stop(&mut self) -> Result<(), String> {
        let phase = self.state.phase;
        if self.slice.is_some() {
            if matches!(phase, TrainingPhase::Running | TrainingPhase::Pausing) {
                self.wanted = Wanted::Stop;
                self.halt.store(true, Ordering::SeqCst);
                self.state.phase = TrainingPhase::Pausing;
            }
            return Ok(());
        }
        if !matches!(phase, TrainingPhase::Running | TrainingPhase::Paused) {
            return Ok(());
        }
        if self.session.is_none() {
            self.load().await?;
            if self.state.phase == TrainingPhase::Complete {
                self.export_if_owed().await;
                return Ok(());
            }
        }
        self.finish(None).await;
        Ok(())
    }

    /// The run is over: save it, mark it finished and put the model in the
    /// library. A failure on the way leaves the saved run where it is.
    pub(super) async fn finish(&mut self, notice: Option<String>) {
        if !self.state.progress.is_complete() {
            self.ending.stopped = true;
        }
        // A model trained with the group is shared with that group.
        let group = self.state.sharing;
        self.drop_sharing();
        self.release_helpers();
        if let Err(error) = self.save_now().await {
            self.fail(error);
            return;
        }
        self.state.phase = TrainingPhase::Complete;
        self.state.notice = notice;
        if self.state.progress.rounds == 0 {
            self.state.notice = Some(
                "Training stopped before its first round, so there is no model to keep yet.".into(),
            );
            return;
        }
        if let Err(error) = self.export(group).await {
            self.state.error = Some(export_failed(&error));
        }
    }

    /// Before a new run replaces this one: keep what it learned as a model.
    async fn keep_unfinished(&mut self) {
        if self.session.is_none()
            || self.ending.exported.is_some()
            || self.state.progress.rounds == 0
        {
            return;
        }
        if let Err(error) = self.export(None).await {
            tracing::warn!(%error, "the replaced run could not be added to the library");
            self.state.notice = Some(match &self.state.saved_at {
                Some(path) => format!(
                    "The earlier run could not be added to Your models. Its file is kept at {path}."
                ),
                None => "The earlier run could not be added to Your models.".into(),
            });
        }
    }

    /// A finished run whose model never reached the library gets another try.
    pub(super) async fn export_if_owed(&mut self) {
        if self.state.phase != TrainingPhase::Complete
            || self.ending.exported.is_some()
            || self.session.is_none()
            || self.state.progress.rounds == 0
        {
            return;
        }
        if let Err(error) = self.export(None).await {
            self.state.error.get_or_insert(export_failed(&error));
        }
    }
}

fn export_failed(error: &str) -> String {
    format!(
        "The model could not be added to Your models: {error} The run itself is saved, and Sovra will try again when you reopen Training."
    )
}
