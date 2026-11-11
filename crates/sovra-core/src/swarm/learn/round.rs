//! One training round from plan to promotion.
//!
//! 1. **Plan**: the coordinator fixes the roster, the departments that consented,
//!    the clipping bound and the per-computer noise, calibrated so the round fits
//!    the privacy budget even if only the threshold of computers survive.
//! 2. **Contribute** (each computer): check its department may train, clip,
//!    quantise, add its discrete-Gaussian share, mask, and hold the upload in the
//!    [`EgressGate`](crate::swarm::egress::EgressGate) for a person's sign-off.
//! 3. **Aggregate** (coordinator): charge the budget for the actual survivors
//!    *before* unmasking, unmask the sum, and return the mean update.
//! 4. **Record**: a `ModelRound` ledger entry with the model hash, its parent, the
//!    departments, contributor count and privacy cost: model lineage.
//! 5. **Promote**: a person who may approve for every contributing department puts
//!    the candidate into use. Rolling back is one more signed entry.

use std::collections::{BTreeMap, BTreeSet};

use super::RandomSource;
use super::privacy::{
    BudgetExhausted, PrivacyBudget, Quantiser, calibrate_sigma_sq, discrete_gaussian, rho_per_round,
};
use super::secagg::{MaskingClient, Reveal, SecAggError, unmask_sum};
use crate::Timestamp;
use crate::group::{ContentHash, DeviceId, PersonId};
use crate::swarm::department::{CustodyError, DepartmentId, Directory};
use crate::swarm::egress::{Destination, Draft};
use crate::swarm::ledger::AuditEvent;

/// Everything every computer must agree on for a round.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundPlan {
    pub round: u64,
    pub base_model: ContentHash,
    pub departments: BTreeSet<DepartmentId>,
    pub roster: BTreeSet<DeviceId>,
    /// The fewest uploads the round will unmask.
    pub threshold: usize,
    /// Computers assumed to collude with the coordinator; their noise is not counted.
    pub assumed_colluders: usize,
    pub quantiser: Quantiser,
    /// Each computer's discrete-Gaussian variance, in quantised units.
    pub sigma_sq: u64,
    /// Length of the update (the parameter change `Δy`).
    pub dimension: usize,
}

/// Why a round step was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RoundError {
    #[error("this department isn't part of the round")]
    NotInRound,
    #[error("the update has {got} numbers, the round expects {want}")]
    WrongDimension { got: usize, want: usize },
    #[error("the round needs at least {0} computers")]
    TooFewComputers(usize),
    #[error(transparent)]
    Custody(#[from] CustodyError),
    #[error(transparent)]
    SecAgg(#[from] SecAggError),
    #[error(transparent)]
    Budget(#[from] BudgetExhausted),
    #[error("that person can't approve a model trained on {0}")]
    NotAnApprover(DepartmentId),
}

/// Choices for a new round.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundSettings {
    pub round: u64,
    pub base_model: ContentHash,
    pub departments: BTreeSet<DepartmentId>,
    pub roster: BTreeSet<DeviceId>,
    pub dimension: usize,
    pub quantiser: Quantiser,
    pub assumed_colluders: usize,
    /// Rounds still planned against the budget, including this one.
    pub rounds_left: u32,
}

impl RoundPlan {
    /// A plan whose noise fits `budget` spread over the rounds left. Every listed
    /// department must have consented to training.
    pub fn new(
        settings: RoundSettings,
        budget: &PrivacyBudget,
        directory: &Directory,
    ) -> Result<Self, RoundError> {
        for department in &settings.departments {
            directory.may_train(department)?;
        }
        let n = settings.roster.len();
        // Two thirds of the roster, at least 2 honest shares beyond the colluders.
        let threshold = (n * 2).div_ceil(3).max(settings.assumed_colluders + 2);
        if n < threshold {
            return Err(RoundError::TooFewComputers(threshold));
        }
        let honest = threshold - settings.assumed_colluders;
        let per_round = budget.left() / f64::from(settings.rounds_left.max(1));
        let sensitivity = settings.quantiser.sensitivity(settings.dimension);
        let sigma_sq = calibrate_sigma_sq(per_round, sensitivity, honest, settings.dimension);
        Ok(Self {
            round: settings.round,
            base_model: settings.base_model,
            departments: settings.departments,
            roster: settings.roster,
            threshold,
            assumed_colluders: settings.assumed_colluders,
            quantiser: settings.quantiser,
            sigma_sq,
            dimension: settings.dimension,
        })
    }

    /// The round's privacy cost if `survivors` uploads are summed.
    pub fn rho(&self, survivors: usize) -> f64 {
        let honest = survivors.saturating_sub(self.assumed_colluders);
        rho_per_round(
            self.quantiser.sensitivity(self.dimension),
            self.sigma_sq,
            honest,
            self.dimension,
        )
    }
}

/// The bytes of a masked upload, as hashed for sign-off and sent.
pub fn upload_bytes(masked: &[u64]) -> Vec<u8> {
    masked.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// One computer's prepared upload, held until a person approves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Contribution {
    pub masked: Vec<u64>,
    pub draft: Draft,
}

/// Step 2: clip, quantise, noise, mask, and draft for sign-off.
pub fn contribute(
    plan: &RoundPlan,
    directory: &Directory,
    department: &DepartmentId,
    delta: &[f32],
    masking: &MaskingClient,
    rng: &mut dyn RandomSource,
    at: Timestamp,
) -> Result<Contribution, RoundError> {
    if !plan.departments.contains(department) {
        return Err(RoundError::NotInRound);
    }
    directory.may_train(department)?;
    if delta.len() != plan.dimension {
        return Err(RoundError::WrongDimension {
            got: delta.len(),
            want: plan.dimension,
        });
    }
    let noised: Vec<i64> = plan
        .quantiser
        .quantise(delta)
        .into_iter()
        .map(|v| v.wrapping_add(discrete_gaussian(plan.sigma_sq, rng)))
        .collect();
    let masked = masking.mask(&noised);
    let draft = Draft {
        content: ContentHash::of(&upload_bytes(&masked)),
        department: department.clone(),
        destination: Destination::TrainingRound { round: plan.round },
        drafted_by: "Shared training".into(),
        summary: format!(
            "This computer's private, noised part of training round {}",
            plan.round
        ),
        created_at: at,
    };
    Ok(Contribution { masked, draft })
}

/// What a finished round produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundOutcome {
    /// The survivors' mean `Δy`, with the round's privacy noise in it.
    pub mean: Vec<f32>,
    pub participants: usize,
    pub rho: f64,
}

/// Step 3: charge the budget for the real survivors, then unmask the mean.
pub fn aggregate(
    plan: &RoundPlan,
    uploads: &BTreeMap<DeviceId, Vec<u64>>,
    reveals: &[Reveal],
    budget: &mut PrivacyBudget,
) -> Result<RoundOutcome, RoundError> {
    let participants = uploads.len();
    if participants < plan.threshold {
        return Err(RoundError::TooFewComputers(plan.threshold));
    }
    let rho = plan.rho(participants);
    budget.charge(rho)?;
    let sum = unmask_sum(plan.round, uploads, reveals, plan.threshold)?;
    let mean = plan.quantiser.dequantise(&sum, participants);
    Ok(RoundOutcome {
        mean,
        participants,
        rho,
    })
}

/// The content address of a parameter vector.
pub fn model_hash(params: &[f32]) -> ContentHash {
    let bytes: Vec<u8> = params.iter().flat_map(|p| p.to_le_bytes()).collect();
    ContentHash::of(&bytes)
}

/// Step 4: the ledger entry for a round's candidate model.
pub fn round_event(plan: &RoundPlan, outcome: &RoundOutcome, model: ContentHash) -> AuditEvent {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a non-negative cost in millionths fits u64"
    )]
    let rho_micro = (outcome.rho * 1e6).ceil().max(0.0) as u64;
    AuditEvent::ModelRound {
        model,
        parent: Some(plan.base_model),
        round: plan.round,
        departments: plan.departments.clone(),
        contributors: u32::try_from(outcome.participants).unwrap_or(u32::MAX),
        rho_micro,
    }
}

/// Step 5: people promote a candidate. Every department whose knowledge trained it
/// needs one of its own approvers among `approvers`.
pub fn promote(
    model: ContentHash,
    departments: &BTreeSet<DepartmentId>,
    approvers: &BTreeSet<PersonId>,
    directory: &Directory,
) -> Result<AuditEvent, RoundError> {
    for department in departments {
        let department = directory.department(department)?;
        if !approvers
            .iter()
            .any(|person| department.may_approve(person))
        {
            return Err(RoundError::NotAnApprover(department.id.clone()));
        }
    }
    Ok(AuditEvent::ModelPromoted {
        model,
        approvers: approvers.clone(),
    })
}

#[cfg(test)]
mod tests;
