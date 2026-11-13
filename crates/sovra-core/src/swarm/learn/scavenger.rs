//! Bounded, asynchronous work leases and a staleness-aware outer optimizer.
//!
//! This is a training primitive, not an authenticated transport or a privacy
//! protocol. The runtime must bind workers to authenticated peers, enforce consent,
//! limit decoded message sizes, and durably save a checkpoint before acknowledging
//! a merge. One surviving worker can advance the model; no fleet-wide barrier is
//! required. Time is supplied by the caller in milliseconds from a monotonic clock.
//!
//! The outer rule gates pseudo-gradients **before** Adam's moment buffers, following
//! <https://arxiv.org/abs/2605.09126>. This implementation is not a reproduction of
//! the paper's experiments or a claim of convergence on arbitrary private data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::group::{ContentHash, DeviceId};

/// Maximum concurrently leased workers. Completed leases leave no replay cache:
/// their monotonically increasing IDs are never issued again in the same run.
const MAX_WORKERS: usize = 64;
/// Bounds this prototype's model buffers; larger models will need fragments.
const MAX_PARAMETERS: usize = 16_777_216;

/// Invalid work never partially changes model, moments, counters, or leases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GridError {
    #[error("training settings or model values are invalid")]
    InvalidSettings,
    #[error("this computer already has work, or all work slots are occupied")]
    Busy,
    #[error("the work assignment is unknown, changed, expired, or already used")]
    InvalidLease,
    #[error("this update is too old; request the current model and try again")]
    Stale,
    #[error("the update has invalid values, dimensions, or token counts")]
    InvalidUpdate,
    #[error("training counters have reached their limit")]
    Exhausted,
    #[error("the saved training state is invalid or from an unsupported version")]
    InvalidCheckpoint,
}

/// CGAD settings. Advanced controls can remain hidden behind a simple UI preset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSettings {
    pub learning_rate: f64,
    pub beta1: f64,
    pub beta2: f64,
    pub epsilon: f64,
    pub age_decay: f64,
    pub cutoff_rounds: u32,
    pub clip_norm: f64,
    pub local_steps: u32,
    pub token_budget: u32,
    pub lease_ms: u32,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            learning_rate: 0.01,
            beta1: 0.9,
            beta2: 0.999,
            epsilon: 1e-8,
            age_decay: 0.1,
            cutoff_rounds: 8,
            clip_norm: 1.0,
            local_steps: 20,
            token_budget: 65_536,
            lease_ms: 60_000,
        }
    }
}

impl GridSettings {
    fn validate(self) -> Result<(), GridError> {
        if !(0.0..=1.0).contains(&self.learning_rate)
            || self.learning_rate == 0.0
            || !(0.0..1.0).contains(&self.beta1)
            || !(0.0..1.0).contains(&self.beta2)
            || !(1e-12..=1.0).contains(&self.epsilon)
            || !(0.0..=100.0).contains(&self.age_decay)
            || !(1..=1024).contains(&self.cutoff_rounds)
            || !(1e-6..=1e6).contains(&self.clip_norm)
            || !(1..=1024).contains(&self.local_steps)
            || !(1..=1_048_576).contains(&self.token_budget)
            || !(1..=3_600_000).contains(&self.lease_ms)
        {
            return Err(GridError::InvalidSettings);
        }
        Ok(())
    }

    fn gate(self, age: u64) -> f64 {
        if age >= u64::from(self.cutoff_rounds) {
            return 0.0;
        }
        let age = f64::from(u32::try_from(age).expect("age below bounded u32 cutoff"));
        let cosine = (std::f64::consts::PI * age / f64::from(self.cutoff_rounds)).cos();
        f64::midpoint(1.0, cosine) * (-self.age_decay * age).exp()
    }
}

/// The job hash identifies an immutable model/data/training recipe, and must be
/// newly chosen for a new run. It is not a credential or an authorization token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkLease {
    pub job: ContentHash,
    pub id: u64,
    pub worker: DeviceId,
    pub base_round: u64,
    pub base_model: ContentHash,
    pub steps: u32,
    pub token_budget: u32,
    pub expires_ms: u64,
}

/// Only model changes and counts cross this boundary, never the training text.
/// Pseudo-gradient sign is `base_parameters - locally_trained_parameters`.
/// This alone does NOT make an update private or prevent data reconstruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub lease: WorkLease,
    pub tokens: u32,
    pub pseudo_gradient: Vec<f32>,
}

/// Serializable state; restore through [`GridSyncer::restore`], never use decoded
/// fields directly. In-flight leases intentionally are not restored: monotonic
/// clock origins change on process restart, and returning workers must rebase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridCheckpoint {
    version: u32,
    job: ContentHash,
    settings: GridSettings,
    params: Vec<f32>,
    first: Vec<f64>,
    second: Vec<f64>,
    beta1_power: f64,
    beta2_power: f64,
    round: u64,
    tokens: u64,
    next_lease: u64,
}

/// Pure state machine. The runtime owns synchronization, persistence and IO.
#[derive(Debug, Clone, PartialEq)]
pub struct GridSyncer {
    saved: GridCheckpoint,
    leases: BTreeMap<u64, WorkLease>,
}

impl GridSyncer {
    pub fn new(
        job: ContentHash,
        params: Vec<f32>,
        settings: GridSettings,
    ) -> Result<Self, GridError> {
        settings.validate()?;
        if params.is_empty()
            || params.len() > MAX_PARAMETERS
            || params.iter().any(|x| !x.is_finite())
        {
            return Err(GridError::InvalidSettings);
        }
        Ok(Self {
            saved: GridCheckpoint {
                version: 1,
                job,
                settings,
                first: vec![0.0; params.len()],
                second: vec![0.0; params.len()],
                params,
                beta1_power: 1.0,
                beta2_power: 1.0,
                round: 0,
                tokens: 0,
                next_lease: 1,
            },
            leases: BTreeMap::new(),
        })
    }

    pub fn job(&self) -> ContentHash {
        self.saved.job
    }

    pub fn params(&self) -> &[f32] {
        &self.saved.params
    }

    pub fn round(&self) -> u64 {
        self.saved.round
    }

    pub fn tokens(&self) -> u64 {
        self.saved.tokens
    }

    pub fn checkpoint(&self) -> GridCheckpoint {
        self.saved.clone()
    }

    pub fn restore(saved: GridCheckpoint) -> Result<Self, GridError> {
        let dimension = saved.params.len();
        #[expect(
            clippy::float_cmp,
            reason = "an unstepped checkpoint must preserve the exact initial power of one"
        )]
        let valid_power = |power: f64| {
            power.is_finite()
                && if saved.round == 0 {
                    power == 1.0
                } else {
                    (0.0..1.0).contains(&power)
                }
        };
        if saved.version != 1
            || saved.settings.validate().is_err()
            || dimension == 0
            || dimension > MAX_PARAMETERS
            || saved.params.iter().any(|x| !x.is_finite())
            || saved.first.len() != dimension
            || saved.second.len() != dimension
            || saved.first.iter().any(|x| !x.is_finite())
            || saved.second.iter().any(|x| !x.is_finite() || *x < 0.0)
            || !valid_power(saved.beta1_power)
            || !valid_power(saved.beta2_power)
            || saved.next_lease <= saved.round
            || saved.tokens < saved.round
        {
            return Err(GridError::InvalidCheckpoint);
        }
        Ok(Self {
            saved,
            leases: BTreeMap::new(),
        })
    }

    /// Reclaims timed-out/stale workers without waiting for them to disconnect.
    /// A paused worker can ask again later, receiving the newest model version.
    pub fn issue(&mut self, worker: DeviceId, now_ms: u64) -> Result<WorkLease, GridError> {
        let next = self
            .saved
            .next_lease
            .checked_add(1)
            .ok_or(GridError::Exhausted)?;
        let expires_ms = now_ms
            .checked_add(u64::from(self.saved.settings.lease_ms))
            .ok_or(GridError::Exhausted)?;
        self.leases.retain(|_, lease| {
            lease.expires_ms > now_ms
                && self.saved.round - lease.base_round
                    < u64::from(self.saved.settings.cutoff_rounds)
        });
        if self.leases.len() >= MAX_WORKERS || self.leases.values().any(|l| l.worker == worker) {
            return Err(GridError::Busy);
        }
        let lease = WorkLease {
            job: self.saved.job,
            id: self.saved.next_lease,
            worker,
            base_round: self.saved.round,
            base_model: super::round::model_hash(self.params()),
            steps: self.saved.settings.local_steps,
            token_budget: self.saved.settings.token_budget,
            expires_ms,
        };
        self.leases.insert(lease.id, lease.clone());
        self.saved.next_lease = next;
        Ok(lease)
    }

    /// Allows a coordinator to discard cached batch data when a lease is reclaimed.
    pub fn has_lease(&self, lease: &WorkLease) -> bool {
        self.leases.get(&lease.id) == Some(lease)
    }

    /// Drops this worker's outstanding lease. The model, round and lease counter
    /// stay as they were, so a later submit of that lease is rejected and the same
    /// worker can be assigned again.
    pub fn release(&mut self, worker: DeviceId) {
        self.leases.retain(|_, lease| lease.worker != worker);
    }

    pub fn next_lease(&self) -> u64 {
        self.saved.next_lease
    }

    /// Merge whichever workers are ready (even just one), weighted by their token
    /// counts. Each update is norm-clipped and age-gated before entering Adam.
    /// Reject the entire batch on a malformed, replayed, expired, or stale update.
    pub fn merge(&mut self, updates: &[Contribution], now_ms: u64) -> Result<(), GridError> {
        let tokens = self.validate_updates(updates, now_ms)?;
        let settings = self.saved.settings;
        let next_round = self
            .saved
            .round
            .checked_add(1)
            .ok_or(GridError::Exhausted)?;
        let next_tokens = self
            .saved
            .tokens
            .checked_add(tokens)
            .ok_or(GridError::Exhausted)?;
        let mut gradient = vec![0.0; self.params().len()];
        // At most 64 workers, each with a u32 budget <= 2^20: exactly representable.
        let total = f64::from(u32::try_from(tokens).expect("bounded batch tokens fit u32"));
        for update in updates {
            let norm = update
                .pseudo_gradient
                .iter()
                .map(|x| f64::from(*x).powi(2))
                .sum::<f64>()
                .sqrt();
            let scale = (settings.clip_norm / norm.max(settings.clip_norm))
                * settings.gate(self.saved.round - update.lease.base_round)
                * f64::from(update.tokens)
                / total;
            for (sum, value) in gradient.iter_mut().zip(&update.pseudo_gradient) {
                *sum += f64::from(*value) * scale;
            }
        }
        let mut next = self.saved.clone();
        next.beta1_power *= settings.beta1;
        next.beta2_power *= settings.beta2;
        for (i, value) in gradient.into_iter().enumerate() {
            next.first[i] = settings.beta1 * next.first[i] + (1.0 - settings.beta1) * value;
            next.second[i] =
                settings.beta2 * next.second[i] + (1.0 - settings.beta2) * value * value;
            let first = next.first[i] / (1.0 - next.beta1_power);
            let second = next.second[i] / (1.0 - next.beta2_power);
            let param = f64::from(next.params[i])
                - settings.learning_rate * first / (second.sqrt() + settings.epsilon);
            if !param.is_finite() || param.abs() > f64::from(f32::MAX) {
                return Err(GridError::InvalidUpdate);
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "finite f32 range checked above"
            )]
            {
                next.params[i] = param as f32;
            }
        }
        next.round = next_round;
        next.tokens = next_tokens;
        self.saved = next;
        for update in updates {
            self.leases.remove(&update.lease.id);
        }
        Ok(())
    }

    fn validate_updates(&self, updates: &[Contribution], now_ms: u64) -> Result<u64, GridError> {
        if updates.is_empty() || updates.len() > MAX_WORKERS {
            return Err(GridError::InvalidUpdate);
        }
        let mut seen = BTreeSet::new();
        let mut tokens = 0;
        for update in updates {
            let lease = &update.lease;
            if self.leases.get(&lease.id) != Some(lease)
                || lease.expires_ms <= now_ms
                || !seen.insert(lease.id)
            {
                return Err(GridError::InvalidLease);
            }
            if self.saved.round - lease.base_round >= u64::from(self.saved.settings.cutoff_rounds) {
                return Err(GridError::Stale);
            }
            if update.tokens == 0
                || update.tokens > lease.token_budget
                || update.pseudo_gradient.len() != self.params().len()
                || update.pseudo_gradient.iter().any(|x| !x.is_finite())
            {
                return Err(GridError::InvalidUpdate);
            }
            tokens += u64::from(update.tokens);
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests;
