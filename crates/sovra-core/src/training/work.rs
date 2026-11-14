//! Bounded, versioned training messages carried over an authenticated group
//! connection: work packets, finding a shared run, and delivering finished models.
//!
//! Only the selected batch, model values and model cards travel. A message never
//! names a file, executable, download or operation to perform outside the learner,
//! and every value is checked against fixed bounds before anything is allocated
//! for it. The transport has already checked that the sender is a current member
//! of the named group; who may be served what is the runtime's decision.

use serde::{Deserialize, Serialize};

use crate::{
    group::{Bytes, ContentHash, DeviceId},
    swarm::learn::scavenger::{Contribution, WorkLease},
};

use super::{
    Architecture, BaseModel, LEGACY_WORK_BATCH_BYTES, LocalOptimizer, MAX_LOCAL_STEPS,
    MAX_MODEL_BYTES, MAX_WINDOWS, MAX_WORK_BATCH_BYTES, ModelCard, ModelId, TrainingError,
    TrainingSettings, check_name,
};

// The cap on group-trained shapes leaves room for the rest of the message: each
// value is eight hex characters, beside the assignment and up to 4096 bytes of text.
const _: () =
    assert!(super::MAX_GROUP_PARAMETERS * 8 + 64 * 1024 <= crate::group::MAX_PEER_MESSAGE_BYTES);

/// Values in the Starter table.
pub const BYTE_MODEL_PARAMETERS: usize = 65_536;
/// Most model ids a computer lists as already held when asking for offers.
pub const MAX_OFFER_HAVE: usize = 64;
/// Most model cards in one reply or announcement.
pub const MAX_OFFERED_MODELS: usize = 8;
/// Most model bytes in one part: 2 MiB, which is 4 MiB as hex on the wire.
pub const MAX_MODEL_PART_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TrainingPeerRequest {
    /// "Give me a packet of your shared run to work on."
    Work,
    Submit(Box<TrainingResult>),
    /// "Are you running training that your group may help with?"
    Status,
    /// "Which finished models are you sharing with this group that I lack?"
    Offers {
        have: Vec<ModelId>,
    },
    /// "Send me that model's bytes from `offset` on."
    FetchModel {
        model: ModelId,
        offset: u64,
    },
    /// "I have that model now."
    Received {
        model: ModelId,
    },
    /// "I have just finished these." Only a hint: the receiver answers `Thanks`
    /// and then fetches what it wants itself. Model bytes never travel in a request.
    Announce {
        models: Vec<ModelCard>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TrainingPeerResponse {
    Assigned(Box<TrainingTask>),
    Accepted(super::TrainingProgress),
    /// Nothing to give right now: no shared run, not this group's, busy, or the
    /// model asked for is not on offer. Ask again later.
    Unavailable,
    Rejected,
    /// A run this group may help with is going.
    Running(RunSummary),
    Offered(Vec<ModelCard>),
    ModelPart {
        total: u64,
        bytes: Bytes,
    },
    Thanks,
}

impl TrainingPeerRequest {
    /// Checks the bounded parts of a request before it is acted on.
    pub fn validate(&self) -> Result<(), TrainingError> {
        match self {
            Self::Work | Self::Status | Self::Received { .. } | Self::Submit(_) => Ok(()),
            Self::Offers { have } if have.len() <= MAX_OFFER_HAVE => Ok(()),
            Self::FetchModel { offset, .. } if *offset < MAX_MODEL_BYTES as u64 => Ok(()),
            Self::Announce { models } if models.len() <= MAX_OFFERED_MODELS => valid_cards(models),
            Self::Offers { .. } | Self::FetchModel { .. } | Self::Announce { .. } => {
                Err(TrainingError::InvalidWork)
            }
        }
    }
}

impl TrainingPeerResponse {
    /// Checks the bounded parts of a reply before it is used. A work packet is
    /// checked separately by [`TrainingTask::parameters`], which also needs to
    /// know who it was meant for.
    pub fn validate(&self) -> Result<(), TrainingError> {
        match self {
            Self::Assigned(_)
            | Self::Accepted(_)
            | Self::Unavailable
            | Self::Rejected
            | Self::Thanks => Ok(()),
            Self::Running(run) => run.validate(),
            Self::Offered(models) if models.len() <= MAX_OFFERED_MODELS => valid_cards(models),
            Self::ModelPart { total, bytes }
                if *total <= MAX_MODEL_BYTES as u64
                    && !bytes.0.is_empty()
                    && bytes.0.len() <= MAX_MODEL_PART_BYTES
                    && bytes.0.len() as u64 <= *total =>
            {
                Ok(())
            }
            Self::Offered(_) | Self::ModelPart { .. } => Err(TrainingError::InvalidWork),
        }
    }
}

fn valid_cards(models: &[ModelCard]) -> Result<(), TrainingError> {
    if models.iter().all(|card| card.validate().is_ok()) {
        Ok(())
    } else {
        Err(TrainingError::InvalidWork)
    }
}

/// What a computer says about the run it is sharing, so a helper can show whose
/// run it joined. It carries no text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    pub name: String,
    pub base: BaseModel,
    pub architecture: Architecture,
    pub rounds: u32,
    pub total_rounds: u32,
}

impl RunSummary {
    pub fn validate(&self) -> Result<(), TrainingError> {
        check_name(&self.name).map_err(|_| TrainingError::InvalidWork)?;
        if self.architecture.validate().is_err()
            || !self.architecture.group_capable()
            || (self.base == BaseModel::Starter) != self.architecture.is_table()
            || self.rounds > self.total_rounds
            || self.total_rounds > TrainingSettings::ROUNDS.1
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(())
    }
}

/// How a helper must do one packet's learning steps. Plain bounded numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkRecipe {
    pub architecture: Architecture,
    /// Windows read from the packet in each step; always 1 for the Starter table.
    pub windows: u16,
    /// The step size for the local steps, in millionths. It is the same in
    /// every round: the schedule scales the round step size, which the owner
    /// applies when it folds the result in. Scaling both made the merge engine,
    /// which remembers the size of recent updates, shrink late steps twice.
    pub learning_rate_micro: u32,
    pub optimizer: LocalOptimizer,
    pub weight_decay_micro: u32,
    /// Largest size of one step's correction, in thousandths; 0 = no limit.
    pub clip_milli: u32,
    /// Chooses which windows are read, so both ends can repeat the work exactly.
    pub seed: u64,
}

impl WorkRecipe {
    pub fn validate(&self) -> Result<(), TrainingError> {
        self.architecture
            .validate()
            .map_err(|_| TrainingError::InvalidWork)?;
        let windows = if self.architecture.is_table() {
            1..=1
        } else {
            1..=MAX_WINDOWS
        };
        let rate = TrainingSettings::LEARNING_RATE_MICRO;
        if !self.architecture.group_capable()
            || !windows.contains(&self.windows)
            || !(rate.0..=rate.1).contains(&self.learning_rate_micro)
            || self.weight_decay_micro > TrainingSettings::MAX_WEIGHT_DECAY_MICRO
            || self.clip_milli > TrainingSettings::MAX_CLIP_MILLI
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(())
    }
}

/// An exact recipe version, rather than peer-supplied executable instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingRecipe {
    /// The first wire form, which computers on the first training release
    /// understand: the Starter table, plain steps of size 0.1 over the whole
    /// packet, at most 512 bytes of text.
    ByteBigramV1,
    /// Any supported model shape with the local rule spelled out.
    ByteModelV2(WorkRecipe),
}

impl TrainingRecipe {
    /// The recipe for the round about to run (counted from zero). Starter runs on
    /// the settings an old helper already uses keep the first wire form.
    pub fn for_round(architecture: Architecture, settings: &TrainingSettings, round: u32) -> Self {
        if settings.is_legacy_starter(architecture) {
            return Self::ByteBigramV1;
        }
        Self::ByteModelV2(WorkRecipe {
            architecture,
            windows: settings.windows,
            learning_rate_micro: settings.learning_rate_micro,
            optimizer: settings.optimizer,
            weight_decay_micro: settings.weight_decay_micro,
            clip_milli: settings.clip_milli,
            seed: settings
                .seed
                .wrapping_add(u64::from(round).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        })
    }

    /// The local rule in full, whichever wire form names it.
    pub fn work(&self) -> WorkRecipe {
        match self {
            Self::ByteBigramV1 => WorkRecipe {
                architecture: Architecture::STARTER,
                windows: 1,
                learning_rate_micro: 100_000,
                optimizer: LocalOptimizer::Sgd,
                weight_decay_micro: 0,
                clip_milli: 0,
                seed: 0,
            },
            Self::ByteModelV2(recipe) => *recipe,
        }
    }

    pub fn architecture(&self) -> Architecture {
        self.work().architecture
    }

    pub fn validate(&self) -> Result<(), TrainingError> {
        self.work().validate()
    }

    /// Most text a packet with this recipe may carry.
    pub fn max_text_bytes(&self) -> usize {
        match self {
            Self::ByteBigramV1 => LEGACY_WORK_BATCH_BYTES,
            Self::ByteModelV2(_) => MAX_WORK_BATCH_BYTES,
        }
    }

    /// Next-byte guesses a packet of `text_len` bytes teaches over `steps` steps:
    /// windows × bytes per window × steps. The owner and the helper both compute
    /// this, and a result with any other count is refused.
    pub fn expected_tokens(&self, text_len: usize, steps: u32) -> Result<u32, TrainingError> {
        let recipe = self.work();
        u32::try_from(recipe.architecture.window_tokens(text_len))
            .ok()
            .and_then(|tokens| tokens.checked_mul(u32::from(recipe.windows)))
            .and_then(|tokens| tokens.checked_mul(steps))
            .filter(|tokens| *tokens > 0)
            .ok_or(TrainingError::InvalidWork)
    }
}

/// One independently runnable assignment. The runtime persists the lease counter
/// before sending it, and binds `lease.worker` to the authenticated connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingTask {
    pub recipe: TrainingRecipe,
    pub lease: WorkLease,
    /// Little-endian f32 bytes, hex on the existing group JSON wire.
    pub model: Bytes,
    pub text: String,
    /// Where each window begins in `text`, in the order the steps read them:
    /// the recipe's `windows` for every step. A transformer's packet is made of
    /// short stretches from different places in the text, and this says where
    /// each one starts. Empty means the recipe's seed places the windows (and
    /// the Starter table always reads its whole packet as one window), which is
    /// also the only form the first training release understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<u32>,
}

impl TrainingTask {
    /// Validate before allocating tensors or starting any learner work: the recipe
    /// and its bounds, who the packet is for, the exact model size its shape
    /// implies, the step and text limits, the cost, and that the model is the one
    /// the lease names.
    pub fn parameters(&self, worker: DeviceId) -> Result<Vec<f32>, TrainingError> {
        self.recipe.validate()?;
        let architecture = self.recipe.architecture();
        if self.lease.worker != worker
            || self.model.0.len() != architecture.parameters() * 4
            || !(1..=MAX_LOCAL_STEPS).contains(&self.lease.steps)
            || !(2..=self.recipe.max_text_bytes()).contains(&self.text.len())
            || self.expected_tokens()? > self.lease.token_budget
            || ContentHash::of(&self.model.0) != self.lease.base_model
        {
            return Err(TrainingError::InvalidWork);
        }
        self.window_starts()?;
        decode_parameters(&self.model, architecture)
    }

    /// Where each window begins, checked against the packet: none for the
    /// Starter table, otherwise either none (the seed places them) or exactly
    /// one for every window of every step, each with a whole window and the byte
    /// after it inside the text.
    pub fn window_starts(&self) -> Result<Vec<usize>, TrainingError> {
        if self.starts.is_empty() {
            return Ok(Vec::new());
        }
        let recipe = self.recipe.work();
        let window = recipe.architecture.window_tokens(self.text.len());
        let wanted = usize::from(recipe.windows) * self.lease.steps as usize;
        let starts: Vec<usize> = self.starts.iter().map(|start| *start as usize).collect();
        if recipe.architecture.is_table()
            || starts.len() != wanted
            || starts.iter().any(|start| {
                start
                    .checked_add(window)
                    .is_none_or(|end| end >= self.text.len())
            })
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(starts)
    }

    pub fn expected_tokens(&self) -> Result<u32, TrainingError> {
        self.recipe
            .expected_tokens(self.text.len(), self.lease.steps)
    }
}

/// A completed local interval. The coordinator checks the original lease and its
/// expected token count; a learner cannot choose another worker or training run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingResult {
    pub lease: WorkLease,
    pub tokens: u32,
    pub pseudo_gradient: Bytes,
}

impl TrainingResult {
    /// Checks a returned update against the run's own model shape.
    pub fn contribution(
        &self,
        worker: DeviceId,
        architecture: Architecture,
    ) -> Result<Contribution, TrainingError> {
        if self.lease.worker != worker || self.tokens == 0 || self.tokens > self.lease.token_budget
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(Contribution {
            lease: self.lease.clone(),
            tokens: self.tokens,
            pseudo_gradient: decode_parameters(&self.pseudo_gradient, architecture)?,
        })
    }
}

/// Model values as wire bytes: exactly the count the shape implies, all finite.
pub fn encode_parameters(
    parameters: &[f32],
    architecture: Architecture,
) -> Result<Bytes, TrainingError> {
    if parameters.len() != architecture.parameters() || parameters.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::InvalidWork);
    }
    Ok(Bytes(
        parameters.iter().flat_map(|x| x.to_le_bytes()).collect(),
    ))
}

/// Wire bytes back to model values, refusing any other length or a value that is
/// not a finite number. The length is checked before anything is allocated.
pub fn decode_parameters(
    bytes: &Bytes,
    architecture: Architecture,
) -> Result<Vec<f32>, TrainingError> {
    if bytes.0.len() != architecture.parameters() * 4 {
        return Err(TrainingError::InvalidWork);
    }
    let parameters: Vec<f32> = bytes
        .0
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if parameters.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::InvalidWork);
    }
    Ok(parameters)
}

#[cfg(test)]
mod tests;
