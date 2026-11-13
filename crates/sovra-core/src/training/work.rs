//! Bounded, versioned learner work carried over an authenticated group connection.
//!
//! Only the selected batch and model weights travel. A task never names a file,
//! executable, model download or operation to perform outside the learner.

use serde::{Deserialize, Serialize};

use crate::{
    group::{Bytes, ContentHash, DeviceId},
    swarm::learn::scavenger::{Contribution, WorkLease},
};

use super::TrainingError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TrainingPeerRequest {
    Work,
    Submit(Box<TrainingResult>),
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
    Unavailable,
    Rejected,
}

pub const BYTE_MODEL_PARAMETERS: usize = 65_536;
pub const MAX_WORK_BATCH_BYTES: usize = 512;
pub const MAX_LOCAL_STEPS: u32 = 16;

/// An exact recipe version, rather than peer-supplied executable instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingRecipe {
    ByteBigramV1,
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
}

impl TrainingTask {
    /// Validate before allocating tensors or starting any learner work.
    pub fn parameters(&self, worker: DeviceId) -> Result<Vec<f32>, TrainingError> {
        if self.lease.worker != worker
            || self.model.0.len() != BYTE_MODEL_PARAMETERS * 4
            || !(1..=MAX_LOCAL_STEPS).contains(&self.lease.steps)
            || !(2..=MAX_WORK_BATCH_BYTES).contains(&self.text.len())
            || ContentHash::of(&self.model.0) != self.lease.base_model
            || self.expected_tokens()? > self.lease.token_budget
        {
            return Err(TrainingError::InvalidWork);
        }
        match self.recipe {
            TrainingRecipe::ByteBigramV1 => decode_parameters(&self.model),
        }
    }

    pub fn expected_tokens(&self) -> Result<u32, TrainingError> {
        u32::try_from(self.text.len().saturating_sub(1))
            .ok()
            .and_then(|tokens| tokens.checked_mul(self.lease.steps))
            .filter(|tokens| *tokens > 0)
            .ok_or(TrainingError::InvalidWork)
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
    pub fn contribution(&self, worker: DeviceId) -> Result<Contribution, TrainingError> {
        if self.lease.worker != worker || self.tokens == 0 || self.tokens > self.lease.token_budget
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(Contribution {
            lease: self.lease.clone(),
            tokens: self.tokens,
            pseudo_gradient: decode_parameters(&self.pseudo_gradient)?,
        })
    }
}

pub fn encode_parameters(parameters: &[f32]) -> Result<Bytes, TrainingError> {
    if parameters.len() != BYTE_MODEL_PARAMETERS || parameters.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::InvalidWork);
    }
    Ok(Bytes(
        parameters.iter().flat_map(|x| x.to_le_bytes()).collect(),
    ))
}

fn decode_parameters(bytes: &Bytes) -> Result<Vec<f32>, TrainingError> {
    if bytes.0.len() != BYTE_MODEL_PARAMETERS * 4 {
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
mod tests {
    use super::*;
    use crate::swarm::learn::scavenger::{GridSettings, GridSyncer};

    fn task() -> TrainingTask {
        let parameters = vec![0.0; BYTE_MODEL_PARAMETERS];
        let mut grid = GridSyncer::new(
            ContentHash::of(b"run"),
            parameters.clone(),
            GridSettings {
                local_steps: 4,
                ..GridSettings::default()
            },
        )
        .unwrap();
        TrainingTask {
            recipe: TrainingRecipe::ByteBigramV1,
            lease: grid.issue(DeviceId([1; 32]), 0).unwrap(),
            model: encode_parameters(&parameters).unwrap(),
            text: "a garden".into(),
        }
    }

    #[test]
    fn tasks_reject_changed_identity_model_recipe_cost_or_batch_before_training() {
        let original = task();
        assert_eq!(original.expected_tokens().unwrap(), 28);
        assert_eq!(
            original.parameters(DeviceId([1; 32])).unwrap().len(),
            BYTE_MODEL_PARAMETERS
        );
        assert!(original.parameters(DeviceId([2; 32])).is_err());
        for change in 0..6 {
            let mut altered = original.clone();
            match change {
                0 => altered.model.0[0] = 1,
                1 => altered.lease.steps = MAX_LOCAL_STEPS + 1,
                2 => altered.text = "x".repeat(MAX_WORK_BATCH_BYTES + 1),
                3 => altered.text.clear(),
                4 => altered.lease.token_budget = 1,
                _ => {
                    altered.model.0[..4].copy_from_slice(&f32::NAN.to_le_bytes());
                    altered.lease.base_model = ContentHash::of(&altered.model.0);
                }
            }
            assert!(
                altered.parameters(DeviceId([1; 32])).is_err(),
                "change {change}"
            );
        }
        let mut wire = serde_json::to_value(original).unwrap();
        wire["recipe"] = "run-a-command".into();
        assert!(serde_json::from_value::<TrainingTask>(wire).is_err());
    }

    #[test]
    fn returned_updates_have_exact_bounded_finite_parameters_and_owned_identity() {
        let assignment = task();
        let mut result = TrainingResult {
            lease: assignment.lease,
            tokens: 28,
            pseudo_gradient: assignment.model,
        };
        assert!(result.contribution(DeviceId([1; 32])).is_ok());
        assert!(result.contribution(DeviceId([2; 32])).is_err());
        result.tokens = 0;
        assert!(result.contribution(DeviceId([1; 32])).is_err());
        result.tokens = 28;
        result.pseudo_gradient.0.pop();
        assert!(result.contribution(DeviceId([1; 32])).is_err());
        assert!(encode_parameters(&[0.0]).is_err());
        assert!(encode_parameters(&vec![f32::INFINITY; BYTE_MODEL_PARAMETERS]).is_err());
    }
}
