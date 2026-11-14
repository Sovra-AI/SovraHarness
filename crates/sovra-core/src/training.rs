//! Training small models on a person's own text, with the group's computers
//! helping and every member receiving the finished model.
//!
//! This module is the pure domain: commands, the state the page shows, the plan
//! and its bounds, the model file, the group messages and the engine ports. The
//! runtime executes the ports on owned blocking tasks. The UI receives progress,
//! metrics and model cards, never tensors, checkpoint contents or the training
//! text. Text is used only when a person explicitly selects it.

use serde::{Deserialize, Serialize};

mod delivery;
mod hardware;
mod library;
mod metrics;
mod models;
mod plan;
mod sample;
mod work;

use crate::group::{ContentHash, DeviceId, GroupId};
pub use delivery::{ModelAssembly, held, model_part, offers};
pub use hardware::{HardwareInfo, MAX_GPUS, MAX_HARDWARE_TEXT_CHARS, TrainingEngineKind};
pub use library::{
    Delivery, LibraryModel, MAX_DELIVERIES, MAX_LIBRARY_MODELS, MAX_RECEIVED_BYTES,
    MAX_RECEIVED_MODELS, MAX_TRAINED_BY_CHARS, MAX_TRIAL_PROMPT_BYTES, ModelCard, ModelOrigin,
    Trial, check_trial_prompt, room_for_received,
};
pub use metrics::{
    ChartPoint, MAX_METRIC_POINTS, MAX_SAMPLE_TEXT_BYTES, MetricPoint, TrainingMetrics, eta_ms,
    guess_choices, guess_quality, sample_text, tokens_per_second,
};
pub use models::{
    Architecture, BASE_MODELS, BaseModel, BaseModelInfo, BlockKind, MAX_GROUP_PARAMETERS,
    MAX_MODEL_BYTES, MODEL_HEADER_BYTES, ModelFacts, ModelFile, ModelHeader, ModelId,
    ParameterBlock, VOCABULARY, decode_model, encode_model, inspect_model, model_id,
};
pub use plan::{
    LEGACY_WORK_BATCH_BYTES, LocalOptimizer, LrSchedule, MAX_FILE_NAME_CHARS, MAX_LOCAL_STEPS,
    MAX_NAME_CHARS, MAX_SAMPLE_BYTES, MAX_TEXT_BYTES, MAX_WINDOWS, MAX_WORK_BATCH_BYTES,
    MIN_TEXT_BYTES, MIN_WORK_BATCH_BYTES, SAMPLE_TEXT, SourceKind, SourceSummary, TrainingBase,
    TrainingPlan, TrainingPlanSummary, TrainingPreset, TrainingSettings, TrainingSource,
    check_name, check_text, text_from_file_bytes,
};
pub use work::{
    BYTE_MODEL_PARAMETERS, MAX_MODEL_PART_BYTES, MAX_OFFER_HAVE, MAX_OFFERED_MODELS, RunSummary,
    TrainingPeerRequest, TrainingPeerResponse, TrainingRecipe, TrainingResult, TrainingTask,
    WorkRecipe, decode_parameters, encode_parameters,
};

/// Largest saved run: the compact optimizer state of the largest model (20 bytes
/// per value), up to 4 MiB of text, and a small header.
pub const MAX_CHECKPOINT_BYTES: usize = 96 * 1024 * 1024;
/// Most other computers of the group listed on the page.
pub const MAX_TRAINING_PEERS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingCommand {
    /// Open the page: read the saved run and library, refresh free memory, and
    /// look for models the group is offering.
    Load,
    /// Start a run. `plan.with_group` is the one explicit choice that lets the
    /// group's computers see pieces of the chosen text.
    Start(Box<TrainingPlan>),
    Pause,
    Resume,
    /// Finish now and keep the model as it is. Nothing is discarded: the model so
    /// far is saved to the library.
    Stop,
    /// Stop helping someone else's run, until the app next starts or the person
    /// turns Help the group train off and on again.
    StopHelping,
    /// Help one named computer's run now. An explicit choice that works without
    /// the Help the group train permission.
    Help(DeviceId),
    /// Type a beginning and let a library model continue it.
    Try {
        model: ModelId,
        prompt: String,
    },
    /// Offer a library model trained here to the current group.
    Share(ModelId),
    /// Stop letting the group help with the current run. The run carries on here.
    StopSharing,
}

impl TrainingCommand {
    /// Checks the parts of a command that need no library or engine: the trial
    /// prompt's length, and a plan whose model shape is already known. A plan that
    /// continues a library model is checked by the runtime once it has the file.
    pub fn validate(&self) -> Result<(), TrainingError> {
        match self {
            Self::Start(plan) => {
                if let Some(architecture) = plan.base.architecture() {
                    plan.validate(architecture)
                } else {
                    check_name(&plan.name)?;
                    plan.source.validate()
                }
            }
            Self::Try { prompt, .. } => check_trial_prompt(prompt),
            Self::Load
            | Self::Pause
            | Self::Resume
            | Self::Stop
            | Self::StopHelping
            | Self::Help(_)
            | Self::Share(_)
            | Self::StopSharing => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrainingPhase {
    #[default]
    Loading,
    Ready,
    Preparing,
    Running,
    Pausing,
    Paused,
    Complete,
    Unavailable,
}

impl TrainingPhase {
    pub fn busy(self) -> bool {
        matches!(
            self,
            Self::Loading | Self::Preparing | Self::Running | Self::Pausing
        )
    }
}

/// The four numbers a helper is told after its result is accepted. The shape is
/// fixed: computers on the first training release read exactly these fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingProgress {
    pub rounds: u32,
    pub total_rounds: u32,
    pub tokens: u64,
    /// Cross entropy in thousandths, for display only. Not a quality score.
    pub loss_milli: Option<u32>,
}

impl TrainingProgress {
    pub fn is_complete(&self) -> bool {
        self.total_rounds > 0 && self.rounds >= self.total_rounds
    }

    /// How far along the run is, 0..=100.
    pub fn percent(&self) -> u8 {
        if self.total_rounds == 0 {
            return 0;
        }
        let percent =
            u64::from(self.rounds.min(self.total_rounds)) * 100 / u64::from(self.total_rounds);
        u8::try_from(percent).unwrap_or(100)
    }
}

/// Everything the Training page shows. Each event carries the whole state, so
/// every list and text in it is bounded and the training text is never in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrainingState {
    pub phase: TrainingPhase,
    pub progress: TrainingProgress,
    /// Where the current run is saved on this computer.
    pub saved_at: Option<String>,
    pub error: Option<String>,
    /// Something worth knowing that is not a failure, such as a received model
    /// that was skipped because the library is full.
    pub notice: Option<String>,
    pub group: Option<TrainingGroup>,
    /// Set while the group may help with this run. Only an explicit start with
    /// the group sets it; it is never saved or restored.
    pub sharing: Option<GroupId>,
    /// Rounds the group's computers contributed to this run.
    pub helped_rounds: u32,
    /// This computer helping someone else's run.
    pub helping: Option<TrainingHelp>,
    pub hardware: Option<HardwareInfo>,
    /// The active or last run's plan, without its text.
    pub plan: Option<TrainingPlanSummary>,
    pub metrics: TrainingMetrics,
    /// "Your models", newest first, at most [`MAX_LIBRARY_MODELS`].
    pub models: Vec<LibraryModel>,
    pub trial: Option<Trial>,
}

/// Another computer of the group, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPeer {
    pub device: DeviceId,
    /// "Person · Computer" from this computer's own group list.
    pub name: String,
    /// Heard from within the last minute.
    pub online: bool,
    /// Working on this computer's shared run right now.
    pub helping: bool,
    /// Rounds it has contributed to this computer's current run.
    pub rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingGroup {
    pub id: GroupId,
    pub name: String,
    pub this_device: DeviceId,
    /// At most [`MAX_TRAINING_PEERS`].
    pub peers: Vec<TrainingPeer>,
}

impl TrainingGroup {
    /// How many of the group's computers are working on this run right now.
    pub fn helpers(&self) -> usize {
        self.peers.iter().filter(|peer| peer.helping).count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpingPhase {
    Working,
    Waiting,
    Stopping,
    Stopped,
    Finished,
}

impl HelpingPhase {
    pub fn active(self) -> bool {
        matches!(self, Self::Working | Self::Waiting | Self::Stopping)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingHelp {
    pub group: GroupId,
    pub peer: TrainingPeer,
    pub phase: HelpingPhase,
    pub accepted_rounds: u32,
    pub progress: Option<TrainingProgress>,
    /// What the other computer says it is training, once it has answered.
    pub run: Option<RunSummary>,
    pub notice: Option<String>,
}

/// A full snapshot of the state. Boxed so the protocol's event stays small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingEvent(pub Box<TrainingState>);

impl TrainingEvent {
    pub fn new(state: TrainingState) -> Self {
        Self(Box::new(state))
    }
}

impl TrainingState {
    /// Whether a new run cannot start yet: one is going, or this computer is
    /// helping someone else's.
    pub fn busy(&self) -> bool {
        self.phase.busy()
            || self
                .helping
                .as_ref()
                .is_some_and(|help| help.phase.active())
    }

    pub fn apply(&mut self, event: &TrainingEvent) {
        self.clone_from(&event.0);
    }

    pub fn model(&self, id: ModelId) -> Option<&LibraryModel> {
        self.models.iter().find(|model| model.card.id == id)
    }

    /// Whether a trial is being worked out right now. One at a time.
    pub fn trying(&self) -> bool {
        self.trial.as_ref().is_some_and(|trial| trial.running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrainingError {
    #[error("This training assignment is invalid, expired, or no longer available.")]
    InvalidWork,
    #[error("Use between 32 bytes and 4 MB of text for training.")]
    InvalidInput,
    /// A setting or name is outside its range. The text names it in plain words.
    #[error("{0}")]
    InvalidSetting(String),
    /// The chosen text file cannot be used. The text says why in plain words.
    #[error("{0}")]
    InvalidFile(String),
    #[error("This model file is damaged or from an unsupported version. Nothing was changed.")]
    InvalidModel,
    #[error(
        "The saved training file is damaged or from an unsupported version. Your file has been kept."
    )]
    InvalidCheckpoint,
    #[error("Training could not finish this step. The last saved step is kept.")]
    Step,
}

/// Constructed in the composition root; no Burn dependency crosses this boundary.
pub trait TrainingEngine: Send + Sync {
    /// Facts about this computer. May take a moment: call it off the UI thread.
    fn hardware(&self) -> HardwareInfo;
    /// Starts a run on `text` (already read and checked). `base` is the library
    /// model file when the plan continues one, and must be `None` for a fresh
    /// model. `run` names this run and must be new for every run.
    fn create(
        &self,
        plan: &TrainingPlan,
        text: &str,
        base: Option<&[u8]>,
        run: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError>;
    fn restore(&self, checkpoint: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError>;
    /// Does one packet of someone else's run. The packet is checked with
    /// [`TrainingTask::parameters`] before any work starts.
    fn work(&self, task: TrainingTask, worker: DeviceId) -> Result<TrainingResult, TrainingError>;
    /// Checks a model file completely and says what it is. A received file is
    /// stored only after this succeeds and agrees with its card.
    fn inspect(&self, model: &[u8]) -> Result<ModelFacts, TrainingError>;
    /// Continues `prompt` with up to `bytes` bytes written by the model. The same
    /// arguments always give the same text.
    fn generate(
        &self,
        model: &[u8],
        prompt: &str,
        bytes: u16,
        temperature_milli: u32,
        seed: u64,
    ) -> Result<String, TrainingError>;
}

/// One bounded round per call. Pause/shutdown wait for the current call and save
/// before reporting success. A process crash replays only what was not yet saved.
pub trait TrainingSession: Send {
    /// Use the same caller-owned monotonic clock for local and remote work.
    fn advance(&mut self, now_ms: u64) -> Result<(), TrainingError>;
    fn progress(&self) -> TrainingProgress;
    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError>;
    fn assign(&mut self, worker: DeviceId, now_ms: u64) -> Result<TrainingTask, TrainingError>;
    fn merge(
        &mut self,
        result: TrainingResult,
        worker: DeviceId,
        now_ms: u64,
    ) -> Result<(), TrainingError>;
    /// Drop this worker's outstanding assignment without merging it. The saved
    /// model stays as it was, and that text interval can be issued again.
    fn abandon(&mut self, worker: DeviceId);
    /// The plan this run was started with, without its text. Also answers after a
    /// restore, so a reopened run shows its name and settings.
    fn plan(&self) -> TrainingPlanSummary;
    /// The latest round's measurements, with the check loss on the rounds it was
    /// evaluated. `None` before the first round.
    fn metrics(&self) -> Option<MetricPoint>;
    /// Text the model wrote at the last check, with the round it was written on.
    fn sample(&self) -> Option<(u32, String)>;
    /// The model as it is now, as a model file.
    fn export(&self) -> Result<Vec<u8>, TrainingError>;
}

#[cfg(test)]
mod tests;
