//! Resumable training runs behind the core training ports.
//!
//! A run is the merge engine (`GridSyncer`) plus the text, a reading position and
//! what the page shows. Each round is one packet of text: this computer learns it
//! itself in [`TrainingSession::advance`], or a helper learns it and the result
//! comes back through [`TrainingSession::merge`]. Either way the same packet code
//! runs, and the same counts are checked.

use std::collections::BTreeMap;
use std::time::Instant;

use sovra_core::group::{ContentHash, DeviceId};
use sovra_core::swarm::learn::scavenger::{Contribution, GridSyncer, WorkLease};
use sovra_core::training::{
    BaseModel, HardwareInfo, MAX_SAMPLE_BYTES, MetricPoint, ModelFacts, ModelHeader, TrainingBase,
    TrainingEngine, TrainingError, TrainingPlan, TrainingPlanSummary, TrainingProgress,
    TrainingRecipe, TrainingResult, TrainingSession, TrainingTask, check_text, check_trial_prompt,
    decode_model, encode_model, encode_parameters, inspect_model, model_id, sample_text,
};

use crate::net::{Batch, Net};
use crate::packet::{self, Compute};
use crate::saved::{self, Header};
use crate::{hardware, init, writer};

/// Stands for this computer in the merge engine's list of workers. No real
/// device has this key: a device key is a public key, and this is not one.
const LOCAL: DeviceId = DeviceId([0; 32]);
/// Most windows of held-back text read for one check, so a check stays quick on
/// long texts and large models.
const MAX_CHECK_WINDOWS: usize = 16;
/// Most held-back text the Starter table reads for one check.
const MAX_TABLE_CHECK_BYTES: usize = 4097;
/// How much of the newest round's speed enters the shown speed.
const SPEED_BLEND: f64 = 0.3;

/// The training engine: Burn's built-in processor backend, with gradients from
/// Burn's autodiff. Holds nothing; build it with `BurnTrainingEngine` or
/// `BurnTrainingEngine::default()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BurnTrainingEngine;

struct Pending {
    lease: WorkLease,
    recipe: TrainingRecipe,
    /// The packet's place in the text.
    start: usize,
    end: usize,
    tokens: u32,
    cursor_before: usize,
    cursor_after: usize,
}

struct Session {
    compute: Compute,
    net: Net,
    plan: TrainingPlanSummary,
    family: BaseModel,
    base_rounds: u32,
    base_tokens: u64,
    text: String,
    text_hash: ContentHash,
    /// `text[..split]` is learned from; the rest is held back for checks.
    split: usize,
    cursor: usize,
    grid: GridSyncer,
    pending: BTreeMap<u64, Pending>,
    loss_milli: Option<u32>,
    check_loss_milli: Option<u32>,
    best_check: Option<(u32, u32)>,
    stalled: bool,
    sample: Option<(u32, String)>,
    point: Option<MetricPoint>,
    /// Next-byte guesses per second on this computer; zero until measured.
    speed: f64,
}

fn milli(loss: f32) -> Option<u32> {
    if !loss.is_finite() || loss < 0.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite, nonnegative and clamped below the u32 limit"
    )]
    let value = (f64::from(loss) * 1000.0).round().min(4_000_000_000.0) as u32;
    Some(value)
}

impl TrainingEngine for BurnTrainingEngine {
    fn hardware(&self) -> HardwareInfo {
        hardware::probe()
    }

    fn create(
        &self,
        plan: &TrainingPlan,
        text: &str,
        base: Option<&[u8]>,
        run: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError> {
        check_text(text)?;
        let (family, architecture, values, base_rounds, base_tokens) = match (plan.base, base) {
            (TrainingBase::Fresh(model), None) => {
                let architecture = model.architecture();
                architecture.validate()?;
                let values = init::fresh(architecture, plan.settings.seed);
                (model, architecture, values, 0, 0)
            }
            (TrainingBase::Continue(id), Some(file)) => {
                if model_id(file) != id {
                    return Err(TrainingError::InvalidModel);
                }
                let model = decode_model(file)?;
                let header = model.header;
                (
                    header.base,
                    header.architecture,
                    model.parameters,
                    header.rounds,
                    header.tokens,
                )
            }
            (TrainingBase::Fresh(_), Some(_)) | (TrainingBase::Continue(_), None) => {
                return Err(TrainingError::InvalidModel);
            }
        };
        plan.validate(architecture)?;
        let grid = GridSyncer::new(run, values, plan.settings.grid_settings(architecture))
            .map_err(|_| TrainingError::Step)?;
        let summary = plan.summary(architecture, text.len());
        Ok(Box::new(Session {
            compute: Compute::new(summary.settings.threads)?,
            net: Net::new(architecture)?,
            split: summary.settings.check_split(text),
            plan: summary,
            family,
            base_rounds,
            base_tokens,
            text_hash: ContentHash::of(text.as_bytes()),
            text: text.into(),
            cursor: 0,
            grid,
            pending: BTreeMap::new(),
            loss_milli: None,
            check_loss_milli: None,
            best_check: None,
            stalled: false,
            sample: None,
            point: None,
            speed: 0.0,
        }))
    }

    fn restore(&self, checkpoint: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError> {
        let invalid = TrainingError::InvalidCheckpoint;
        let saved = saved::decode(checkpoint)?;
        let header = saved.header;
        let plan = header.plan;
        let architecture = plan.architecture;
        plan.validate().map_err(|_| invalid.clone())?;
        check_text(&saved.text).map_err(|_| invalid.clone())?;
        let grid = GridSyncer::restore(saved.grid).map_err(|_| invalid.clone())?;
        let split = plan.settings.check_split(&saved.text);
        let cursor = usize::try_from(header.cursor).map_err(|_| invalid.clone())?;
        let round = u32::try_from(grid.round()).map_err(|_| invalid.clone())?;
        let needed = plan
            .settings
            .work_tokens(architecture, plan.settings.batch_bytes as usize);
        if plan.source.bytes != saved.text.len() as u64
            || (header.family == BaseModel::Starter) != architecture.is_table()
            || saved.job.is_some_and(|job| grid.job() != job)
            || grid.params().len() != architecture.parameters()
            || round > plan.settings.rounds
            || grid.settings().local_steps != plan.settings.local_steps
            || u64::from(grid.settings().token_budget) < needed
            || cursor >= split
            || !saved.text.is_char_boundary(cursor)
            || header.point.is_some_and(|point| point.round > round)
            || header.sample.as_ref().is_some_and(|(at, _)| *at > round)
            || header.best_check.is_some_and(|(_, at)| at > round)
        {
            return Err(invalid);
        }
        Ok(Box::new(Session {
            compute: Compute::new(plan.settings.threads).map_err(|_| TrainingError::Step)?,
            net: Net::new(architecture).map_err(|_| invalid)?,
            plan,
            family: header.family,
            base_rounds: header.base_rounds,
            base_tokens: header.base_tokens,
            text_hash: header.text,
            text: saved.text,
            split,
            cursor,
            grid,
            pending: BTreeMap::new(),
            loss_milli: header.loss_milli,
            check_loss_milli: header.check_loss_milli,
            best_check: header.best_check,
            stalled: header.stalled,
            sample: header.sample,
            point: header.point,
            speed: 0.0,
        }))
    }

    fn work(&self, task: TrainingTask, worker: DeviceId) -> Result<TrainingResult, TrainingError> {
        // Everything about the packet is checked before any tensor exists.
        let parameters = task.parameters(worker)?;
        let tokens = task.expected_tokens()?;
        let recipe = task.recipe.work();
        let net = Net::new(recipe.architecture).map_err(|_| TrainingError::InvalidWork)?;
        let learned = packet::learn(
            &Compute::new(0)?,
            &net,
            &recipe,
            &parameters,
            task.text.as_bytes(),
            task.lease.steps,
        )?;
        Ok(TrainingResult {
            pseudo_gradient: encode_parameters(&learned.pseudo_gradient, recipe.architecture)?,
            lease: task.lease,
            tokens,
        })
    }

    fn inspect(&self, model: &[u8]) -> Result<ModelFacts, TrainingError> {
        inspect_model(model)
    }

    fn generate(
        &self,
        model: &[u8],
        prompt: &str,
        bytes: u16,
        temperature_milli: u32,
        seed: u64,
    ) -> Result<String, TrainingError> {
        check_trial_prompt(prompt)?;
        let model = decode_model(model)?;
        let written = writer::write(
            model.header.architecture,
            &model.parameters,
            prompt.as_bytes(),
            usize::from(bytes.min(MAX_SAMPLE_BYTES)),
            temperature_milli,
            seed,
        )?;
        Ok(sample_text(&written))
    }
}

impl Session {
    fn round(&self) -> u32 {
        u32::try_from(self.grid.round()).unwrap_or(u32::MAX)
    }

    /// Reserves the next packet of text for `worker` and returns its lease id.
    fn take(&mut self, worker: DeviceId, now_ms: u64) -> Result<u64, TrainingError> {
        if self.progress().is_complete() {
            return Err(TrainingError::InvalidWork);
        }
        // A worker that asks again replaces its own unfinished lease. The model
        // is unchanged, and the same text interval is issued when the cursor has
        // not moved on for someone else.
        self.abandon(worker);
        let settings = &self.plan.settings;
        let learn = &self.text[..self.split];
        let batch = settings.batch_bytes as usize;
        let cursor_before = self.cursor;
        let mut end = (self.cursor + batch).min(learn.len());
        while !learn.is_char_boundary(end) {
            end -= 1;
        }
        // The last packet reaches back so it is as full as any other: a short
        // tail on its own would teach little.
        let start = if end == learn.len() {
            let mut start = learn.len().saturating_sub(batch);
            while !learn.is_char_boundary(start) {
                start += 1;
            }
            start.min(self.cursor)
        } else {
            self.cursor
        };
        let recipe = TrainingRecipe::for_round(self.net.architecture(), settings, self.round());
        let tokens = recipe.expected_tokens(end - start, settings.local_steps)?;
        let lease = self
            .grid
            .issue(worker, now_ms)
            .map_err(|_| TrainingError::InvalidWork)?;
        self.pending
            .retain(|_, packet| self.grid.has_lease(&packet.lease));
        // The next packet overlaps the last character, so the byte after it is
        // still taught. The position is saved before work is sent: a missing
        // worker's packet is read again when the cursor comes round.
        let cursor_after = if end == learn.len() {
            0
        } else {
            learn[..end]
                .char_indices()
                .next_back()
                .map_or(end, |(i, _)| i)
        };
        self.cursor = cursor_after;
        let id = lease.id;
        self.pending.insert(
            id,
            Pending {
                lease,
                recipe,
                start,
                end,
                tokens,
                cursor_before,
                cursor_after,
            },
        );
        Ok(id)
    }

    /// Folds one finished packet into the model and refreshes what the page shows.
    /// `measured` is the loss the local steps saw; a helper's packet is measured
    /// here instead.
    fn fold(
        &mut self,
        contribution: Contribution,
        now_ms: u64,
        measured: Option<f32>,
    ) -> Result<(), TrainingError> {
        let id = contribution.lease.id;
        // The schedule moves the merge engine's step size, which is what sets how
        // far the model travels each round. A refused merge changes nothing.
        let previous = self.grid.settings().learning_rate;
        self.grid
            .set_learning_rate(self.plan.settings.outer_learning_rate_at(self.round()))
            .map_err(|_| TrainingError::Step)?;
        if self.grid.merge(&[contribution], now_ms).is_err() {
            self.grid
                .set_learning_rate(previous)
                .map_err(|_| TrainingError::Step)?;
            return Err(TrainingError::InvalidWork);
        }
        let packet = self.pending.remove(&id);
        let loss = measured.or_else(|| {
            let packet = packet.as_ref()?;
            packet::packet_loss(
                &self.compute,
                &self.net,
                &packet.recipe.work(),
                self.grid.params(),
                &self.text.as_bytes()[packet.start..packet.end],
            )
            .ok()
        });
        if let Some(loss) = loss.and_then(milli) {
            self.loss_milli = Some(loss);
        }
        self.after_round();
        Ok(())
    }

    fn after_round(&mut self) {
        let round = self.round();
        let settings = &self.plan.settings;
        let due =
            round == 1 || round.is_multiple_of(settings.check_every) || round >= settings.rounds;
        let mut checked = None;
        if due {
            checked = self.check();
            if let Some(loss) = checked {
                self.check_loss_milli = Some(loss);
                if self.best_check.is_none_or(|(best, _)| loss < best) {
                    self.best_check = Some((loss, round));
                }
                let limit = self.plan.settings.stop_when_stalled;
                if limit > 0
                    && self
                        .best_check
                        .is_some_and(|(_, at)| round.saturating_sub(at) >= limit)
                {
                    self.stalled = true;
                }
            }
            self.sample = self.write_sample(round);
        }
        if let Some(loss_milli) = self.loss_milli {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a nonnegative speed clamped to the u32 range"
            )]
            let tokens_per_second = self.speed.round().clamp(0.0, f64::from(u32::MAX)) as u32;
            self.point = Some(MetricPoint {
                round,
                loss_milli,
                check_loss_milli: checked,
                lr_micro: self
                    .plan
                    .settings
                    .outer_learning_rate_micro_at(round.saturating_sub(1)),
                tokens_per_second,
            });
        }
    }

    /// The loss on text the model has not learned from, or `None` when no text
    /// was held back.
    fn check(&self) -> Option<u32> {
        let tail = &self.text.as_bytes()[self.split..];
        let architecture = self.net.architecture();
        let length = if architecture.is_table() {
            tail.len().min(MAX_TABLE_CHECK_BYTES).checked_sub(1)?
        } else {
            architecture.window_tokens(tail.len())
        };
        if length == 0 {
            return None;
        }
        // Windows side by side across the tail; when there are more than a check
        // may read, evenly spaced ones are taken.
        let available = if architecture.is_table() {
            1
        } else {
            (tail.len() - 1) / length
        };
        let count = available.clamp(1, MAX_CHECK_WINDOWS);
        let starts: Vec<usize> = (0..count)
            .map(|index| index * available / count * length)
            .collect();
        let batch = Batch::windows(tail, &starts, length).ok()?;
        self.compute
            .loss(&self.net, self.grid.params(), &batch)
            .ok()
            .and_then(milli)
    }

    /// Lets the model write from the text's first character.
    fn write_sample(&self, round: u32) -> Option<(u32, String)> {
        let settings = &self.plan.settings;
        let first = self.text.chars().next()?;
        let mut bytes = self.text.as_bytes()[..first.len_utf8()].to_vec();
        let written = writer::write(
            self.net.architecture(),
            self.grid.params(),
            &bytes,
            usize::from(settings.sample_bytes.min(MAX_SAMPLE_BYTES)),
            settings.sample_temperature_milli,
            settings.seed ^ u64::from(round).wrapping_mul(0xA24B_AED4_963E_E407),
        )
        .ok()?;
        bytes.extend_from_slice(&written);
        Some((round, sample_text(&bytes)))
    }

    fn header(&self) -> Header {
        Header {
            plan: self.plan.clone(),
            family: self.family,
            base_rounds: self.base_rounds,
            base_tokens: self.base_tokens,
            text: self.text_hash,
            cursor: self.cursor as u64,
            loss_milli: self.loss_milli,
            check_loss_milli: self.check_loss_milli,
            best_check: self.best_check,
            stalled: self.stalled,
            sample: self.sample.clone(),
            point: self.point,
        }
    }
}

impl TrainingSession for Session {
    fn advance(&mut self, now_ms: u64) -> Result<(), TrainingError> {
        if self.progress().is_complete() {
            return Ok(());
        }
        let id = self.take(LOCAL, now_ms)?;
        let packet = self.pending.get(&id).ok_or(TrainingError::Step)?;
        let lease = packet.lease.clone();
        let tokens = packet.tokens;
        let started = Instant::now();
        // The same packet code a helper runs, straight on the model's values:
        // nothing is encoded for the wire when the work stays here.
        let learned = packet::learn(
            &self.compute,
            &self.net,
            &packet.recipe.work(),
            self.grid.params(),
            &self.text.as_bytes()[packet.start..packet.end],
            lease.steps,
        );
        let Ok(learned) = learned else {
            self.abandon(LOCAL);
            return Err(TrainingError::Step);
        };
        let seconds = started.elapsed().as_secs_f64();
        if seconds > 0.0 {
            let speed = f64::from(tokens) / seconds;
            self.speed = if self.speed > 0.0 {
                self.speed * (1.0 - SPEED_BLEND) + speed * SPEED_BLEND
            } else {
                speed
            };
        }
        let contribution = Contribution {
            lease,
            tokens,
            pseudo_gradient: learned.pseudo_gradient,
        };
        if self.fold(contribution, now_ms, Some(learned.loss)).is_err() {
            self.abandon(LOCAL);
            return Err(TrainingError::Step);
        }
        Ok(())
    }

    fn abandon(&mut self, worker: DeviceId) {
        let id = self
            .pending
            .iter()
            .find(|(_, packet)| packet.lease.worker == worker)
            .map(|(id, _)| *id);
        if let Some(id) = id
            && let Some(packet) = self.pending.remove(&id)
            && self.cursor == packet.cursor_after
        {
            self.cursor = packet.cursor_before;
        }
        self.grid.release(worker);
    }

    fn assign(&mut self, worker: DeviceId, now_ms: u64) -> Result<TrainingTask, TrainingError> {
        // A shape too large for one group message is trained here only, and no
        // helper may claim this computer's own slot.
        if !self.net.architecture().group_capable() || worker == LOCAL {
            return Err(TrainingError::InvalidWork);
        }
        let id = self.take(worker, now_ms)?;
        let packet = self.pending.get(&id).ok_or(TrainingError::InvalidWork)?;
        let model = encode_parameters(self.grid.params(), self.net.architecture());
        match model {
            Ok(model) => Ok(TrainingTask {
                recipe: packet.recipe,
                lease: packet.lease.clone(),
                model,
                text: self.text[packet.start..packet.end].into(),
            }),
            Err(error) => {
                self.abandon(worker);
                Err(error)
            }
        }
    }

    fn merge(
        &mut self,
        result: TrainingResult,
        worker: DeviceId,
        now_ms: u64,
    ) -> Result<(), TrainingError> {
        if self.progress().is_complete() || worker == LOCAL {
            return Err(TrainingError::InvalidWork);
        }
        let packet = self
            .pending
            .get(&result.lease.id)
            .ok_or(TrainingError::InvalidWork)?;
        if packet.lease != result.lease || packet.tokens != result.tokens {
            return Err(TrainingError::InvalidWork);
        }
        let contribution = result.contribution(worker, self.net.architecture())?;
        self.fold(contribution, now_ms, None)
    }

    fn progress(&self) -> TrainingProgress {
        let rounds = self.round();
        TrainingProgress {
            rounds,
            // A run that stopped improving is finished where it stands.
            total_rounds: if self.stalled {
                rounds
            } else {
                self.plan.settings.rounds
            },
            tokens: self.grid.tokens(),
            loss_milli: self.loss_milli,
        }
    }

    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError> {
        saved::encode(&self.header(), &self.text, &self.grid.checkpoint())
    }

    fn plan(&self) -> TrainingPlanSummary {
        self.plan.clone()
    }

    fn metrics(&self) -> Option<MetricPoint> {
        self.point
    }

    fn sample(&self) -> Option<(u32, String)> {
        self.sample.clone()
    }

    fn export(&self) -> Result<Vec<u8>, TrainingError> {
        let header = ModelHeader {
            base: self.family,
            architecture: self.net.architecture(),
            rounds: self.base_rounds.saturating_add(self.round()),
            tokens: self.base_tokens.saturating_add(self.grid.tokens()),
            loss_milli: self.loss_milli,
            check_loss_milli: self.check_loss_milli,
        };
        encode_model(&header, self.grid.params())
    }
}

#[cfg(test)]
mod tests;
