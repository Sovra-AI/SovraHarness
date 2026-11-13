//! Training controls and backend-neutral synchronous learner ports.
//!
//! Runtime executes the port on owned blocking tasks. UI receives only progress,
//! never tensors or checkpoint contents. Selected text is explicit opt-in input.

use serde::{Deserialize, Serialize};

mod work;
use crate::group::{ContentHash, DeviceId, GroupId};
pub use work::{
    BYTE_MODEL_PARAMETERS, MAX_LOCAL_STEPS, MAX_WORK_BATCH_BYTES, TrainingPeerRequest,
    TrainingPeerResponse, TrainingRecipe, TrainingResult, TrainingTask, encode_parameters,
};

pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
pub const SAMPLE_TEXT: &str = "The sun warms the garden. The rain waters the garden. We plant seeds in the garden. The seeds grow into flowers. The flowers bring bees. We care for the garden together. ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingCommand {
    Load,
    Start(TrainingSource),
    StartShared(TrainingSource),
    StopSharing,
    Help(DeviceId),
    Pause,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingSource {
    Sample,
    Text(String),
}

impl TrainingSource {
    /// No automatic ingestion of chat history, memories, folders, or shared data.
    pub fn text(&self) -> Result<&str, TrainingError> {
        match self {
            Self::Sample => Ok(SAMPLE_TEXT),
            Self::Text(text)
                if (32..=MAX_TEXT_BYTES).contains(&text.len()) && !text.trim().is_empty() =>
            {
                Ok(text)
            }
            Self::Text(_) => Err(TrainingError::InvalidInput),
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrainingState {
    pub phase: TrainingPhase,
    pub progress: TrainingProgress,
    pub saved_at: Option<String>,
    pub error: Option<String>,
    pub group: Option<TrainingGroup>,
    pub sharing: Option<GroupId>,
    pub helped_rounds: u32,
    pub helping: Option<TrainingHelp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPeer {
    pub device: DeviceId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingGroup {
    pub id: GroupId,
    pub name: String,
    pub this_device: DeviceId,
    pub peers: Vec<TrainingPeer>,
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
    pub notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingEvent(pub TrainingState);

impl TrainingState {
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
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrainingError {
    #[error("This training assignment is invalid, expired, or no longer available.")]
    InvalidWork,
    #[error("Paste between 32 bytes and 64 KB of text you want to use for training.")]
    InvalidInput,
    #[error(
        "The saved training file is damaged or from an unsupported version. Your file has been kept."
    )]
    InvalidCheckpoint,
    #[error("Training could not finish this step. The last saved step is kept.")]
    Step,
}

/// Constructed in the composition root; no Burn dependency crosses this boundary.
pub trait TrainingEngine: Send + Sync {
    fn create(
        &self,
        text: &str,
        run: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError>;
    fn restore(&self, checkpoint: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError>;
    fn work(&self, task: TrainingTask, worker: DeviceId) -> Result<TrainingResult, TrainingError>;
}

/// One bounded round per call. Pause/shutdown wait for the current call and save
/// before reporting success. A process crash replays at most one unsaved round.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_bounded_nonempty_text_is_accepted() {
        assert_eq!(TrainingSource::Sample.text(), Ok(SAMPLE_TEXT));
        for value in [
            String::new(),
            "x".repeat(31),
            " ".repeat(32),
            "x".repeat(MAX_TEXT_BYTES + 1),
        ] {
            assert_eq!(
                TrainingSource::Text(value).text(),
                Err(TrainingError::InvalidInput)
            );
        }
        assert!(TrainingSource::Text("x".repeat(32)).text().is_ok());
    }

    #[test]
    fn helping_keeps_local_start_blocked_until_stopped_or_finished() {
        let mut state = TrainingState {
            phase: TrainingPhase::Ready,
            ..TrainingState::default()
        };
        assert!(!state.busy());
        assert!(!state.progress.is_complete());
        for phase in [
            HelpingPhase::Working,
            HelpingPhase::Waiting,
            HelpingPhase::Stopping,
            HelpingPhase::Stopped,
            HelpingPhase::Finished,
        ] {
            state.helping = Some(TrainingHelp {
                group: GroupId([1; 32]),
                peer: TrainingPeer {
                    device: DeviceId([2; 32]),
                    name: "Family computer".into(),
                },
                phase,
                accepted_rounds: 0,
                progress: None,
                notice: None,
            });
            assert_eq!(
                state.busy(),
                matches!(
                    phase,
                    HelpingPhase::Working | HelpingPhase::Waiting | HelpingPhase::Stopping
                )
            );
        }
        state.progress.rounds = 1;
        state.progress.total_rounds = 1;
        assert!(state.progress.is_complete());
    }

    #[test]
    fn state_folds_progress_errors_and_pause_acknowledgement() {
        let mut state = TrainingState::default();
        assert!(state.phase.busy());
        for phase in [
            TrainingPhase::Ready,
            TrainingPhase::Preparing,
            TrainingPhase::Running,
            TrainingPhase::Pausing,
            TrainingPhase::Paused,
            TrainingPhase::Complete,
            TrainingPhase::Unavailable,
        ] {
            let event = TrainingEvent(TrainingState {
                phase,
                error: Some("test".into()),
                ..TrainingState::default()
            });
            state.apply(&event);
            assert_eq!(state, event.0);
            assert_eq!(
                phase.busy(),
                matches!(
                    phase,
                    TrainingPhase::Preparing | TrainingPhase::Running | TrainingPhase::Pausing
                )
            );
        }
    }
}
