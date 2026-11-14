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
    /// Whether every value is inside the range the grid accepts.
    pub fn validate(self) -> Result<(), GridError> {
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

/// Marks the compact binary form of a [`GridCheckpoint`].
const CHECKPOINT_MAGIC: &[u8; 8] = b"SOVRAGRD";
/// Fixed bytes before the three value arrays in the binary form.
const CHECKPOINT_HEADER_BYTES: usize = 8 + 4 + 32 + 5 * 8 + 4 + 8 + 3 * 4 + 2 * 8 + 4 * 8;
/// Bytes per parameter in the binary form: one f32 value and two f64 moments.
const CHECKPOINT_BYTES_PER_PARAMETER: usize = 4 + 8 + 8;

impl GridCheckpoint {
    /// The exact length of [`Self::to_bytes`] for a model of `parameters` values.
    pub const fn byte_len(parameters: usize) -> usize {
        CHECKPOINT_HEADER_BYTES + parameters * CHECKPOINT_BYTES_PER_PARAMETER
    }

    /// How many model values this state holds.
    pub fn parameters(&self) -> usize {
        self.params.len()
    }

    /// A compact little-endian form: 20 bytes per parameter where JSON needs about
    /// 45. The layout is a fixed header (magic, version, job, settings, counters,
    /// parameter count) followed by the values, then both moment arrays.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::byte_len(self.params.len()));
        out.extend_from_slice(CHECKPOINT_MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(self.job.as_bytes());
        let settings = self.settings;
        for value in [
            settings.learning_rate,
            settings.beta1,
            settings.beta2,
            settings.epsilon,
            settings.age_decay,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&settings.cutoff_rounds.to_le_bytes());
        out.extend_from_slice(&settings.clip_norm.to_le_bytes());
        out.extend_from_slice(&settings.local_steps.to_le_bytes());
        out.extend_from_slice(&settings.token_budget.to_le_bytes());
        out.extend_from_slice(&settings.lease_ms.to_le_bytes());
        out.extend_from_slice(&self.beta1_power.to_le_bytes());
        out.extend_from_slice(&self.beta2_power.to_le_bytes());
        out.extend_from_slice(&self.round.to_le_bytes());
        out.extend_from_slice(&self.tokens.to_le_bytes());
        out.extend_from_slice(&self.next_lease.to_le_bytes());
        out.extend_from_slice(&(self.params.len() as u64).to_le_bytes());
        out.extend(self.params.iter().flat_map(|x| x.to_le_bytes()));
        out.extend(self.first.iter().flat_map(|x| x.to_le_bytes()));
        out.extend(self.second.iter().flat_map(|x| x.to_le_bytes()));
        out
    }

    /// Decodes [`Self::to_bytes`]. The declared size is checked against the real
    /// length before anything is allocated. The result still has to pass
    /// [`GridSyncer::restore`], which checks every value.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GridError> {
        let mut reader = Reader(bytes);
        if reader.take::<8>()? != *CHECKPOINT_MAGIC {
            return Err(GridError::InvalidCheckpoint);
        }
        let version = reader.u32()?;
        let job = ContentHash(reader.take::<32>()?);
        let settings = GridSettings {
            learning_rate: reader.f64()?,
            beta1: reader.f64()?,
            beta2: reader.f64()?,
            epsilon: reader.f64()?,
            age_decay: reader.f64()?,
            cutoff_rounds: reader.u32()?,
            clip_norm: reader.f64()?,
            local_steps: reader.u32()?,
            token_budget: reader.u32()?,
            lease_ms: reader.u32()?,
        };
        let beta1_power = reader.f64()?;
        let beta2_power = reader.f64()?;
        let round = reader.u64()?;
        let tokens = reader.u64()?;
        let next_lease = reader.u64()?;
        let dimension = usize::try_from(reader.u64()?)
            .ok()
            .filter(|n| (1..=MAX_PARAMETERS).contains(n))
            .ok_or(GridError::InvalidCheckpoint)?;
        if reader.0.len() != dimension * CHECKPOINT_BYTES_PER_PARAMETER {
            return Err(GridError::InvalidCheckpoint);
        }
        let (params, moments) = reader.0.split_at(dimension * 4);
        let (first, second) = moments.split_at(dimension * 8);
        let floats = |bytes: &[u8]| -> Vec<f64> {
            bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|x| f64::from_le_bytes(*x))
                .collect()
        };
        Ok(Self {
            version,
            job,
            settings,
            params: params
                .as_chunks::<4>()
                .0
                .iter()
                .map(|x| f32::from_le_bytes(*x))
                .collect(),
            first: floats(first),
            second: floats(second),
            beta1_power,
            beta2_power,
            round,
            tokens,
            next_lease,
        })
    }
}

/// Reads fixed-size little-endian fields, failing instead of panicking when short.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], GridError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(GridError::InvalidCheckpoint)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u32(&mut self) -> Result<u32, GridError> {
        self.take().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, GridError> {
        self.take().map(u64::from_le_bytes)
    }

    fn f64(&mut self) -> Result<f64, GridError> {
        self.take().map(f64::from_le_bytes)
    }
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

    pub fn settings(&self) -> GridSettings {
        self.saved.settings
    }

    /// Changes the outer step size for the merges that follow, so a schedule can
    /// warm it up and wind it down. Leases, moments and counters are untouched.
    pub fn set_learning_rate(&mut self, learning_rate: f64) -> Result<(), GridError> {
        let settings = GridSettings {
            learning_rate,
            ..self.saved.settings
        };
        settings.validate()?;
        self.saved.settings = settings;
        Ok(())
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
        let beta1_power = self.saved.beta1_power * settings.beta1;
        let beta2_power = self.saved.beta2_power * settings.beta2;
        // Every new value is worked out before anything saved changes, so a
        // refused merge leaves the model exactly as it was. Only the new values
        // are held aside: copying the whole saved state each round (the values
        // and both moment arrays) cost more than the arithmetic.
        // The check is gathered instead of stopping at the first bad value, which
        // lets the loop run several values at a time.
        let mut valid = true;
        let params: Vec<f32> = gradient
            .iter()
            .zip(&self.saved.first)
            .zip(&self.saved.second)
            .zip(&self.saved.params)
            .map(|(((value, first), second), param)| {
                let first =
                    (settings.beta1 * first + (1.0 - settings.beta1) * value) / (1.0 - beta1_power);
                let second = (settings.beta2 * second + (1.0 - settings.beta2) * value * value)
                    / (1.0 - beta2_power);
                let param = f64::from(*param)
                    - settings.learning_rate * first / (second.sqrt() + settings.epsilon);
                // A value that is not a number fails this comparison too.
                valid &= param.abs() <= f64::from(f32::MAX);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "values outside the f32 range refuse the whole merge below"
                )]
                {
                    param as f32
                }
            })
            .collect();
        if !valid {
            return Err(GridError::InvalidUpdate);
        }
        // Accepted: the moments move in place, with the same arithmetic as above.
        for ((first, second), value) in self
            .saved
            .first
            .iter_mut()
            .zip(&mut self.saved.second)
            .zip(&gradient)
        {
            *first = settings.beta1 * *first + (1.0 - settings.beta1) * value;
            *second = settings.beta2 * *second + (1.0 - settings.beta2) * value * value;
        }
        self.saved.params = params;
        self.saved.beta1_power = beta1_power;
        self.saved.beta2_power = beta2_power;
        self.saved.round = next_round;
        self.saved.tokens = next_tokens;
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
