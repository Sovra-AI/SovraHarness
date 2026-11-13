//! Resumable CPU language-training sessions behind the core learner port.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sovra_core::group::{ContentHash, DeviceId};
use sovra_core::swarm::learn::scaffold::{Algorithm, LocalWork, Objective, ScaffoldClient};
use sovra_core::swarm::learn::scavenger::{GridCheckpoint, GridSettings, GridSyncer, WorkLease};
use sovra_core::training::{
    BYTE_MODEL_PARAMETERS, MAX_CHECKPOINT_BYTES, MAX_WORK_BATCH_BYTES, TrainingEngine,
    TrainingError, TrainingProgress, TrainingRecipe, TrainingResult, TrainingSession,
    TrainingSource, TrainingTask, encode_parameters,
};

use crate::ByteLanguageModel;

const MIN_ROUNDS: u32 = 80;

fn round_limit(text: &str) -> u32 {
    // Each UTF-8 batch advances by at least 508 bytes, including overlap.
    MIN_ROUNDS.max(u32::try_from(text.len().div_ceil(BATCH_BYTES - 4)).expect("bounded text size"))
}
const BATCH_BYTES: usize = MAX_WORK_BATCH_BYTES;

#[derive(Debug, Default)]
pub struct BurnTrainingEngine;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSession {
    version: u32,
    #[serde(default)]
    recipe: Option<ContentHash>,
    text: String,
    cursor: usize,
    grid: GridCheckpoint,
    loss_milli: Option<u32>,
}

struct Session {
    pending: BTreeMap<u64, PendingBatch>,
    text: String,
    cursor: usize,
    grid: GridSyncer,
    loss_milli: Option<u32>,
}

struct PendingBatch {
    lease: WorkLease,
    text: String,
    tokens: u32,
    cursor_before: usize,
    cursor_after: usize,
}

impl TrainingEngine for BurnTrainingEngine {
    fn create(
        &self,
        text: &str,
        run: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError> {
        TrainingSource::Text(text.to_owned()).text()?;
        let settings = GridSettings {
            local_steps: 4,
            learning_rate: 0.25,
            ..GridSettings::default()
        };
        let grid = GridSyncer::new(run, vec![0.0; BYTE_MODEL_PARAMETERS], settings)
            .map_err(|_| TrainingError::Step)?;
        Ok(Box::new(Session {
            pending: BTreeMap::new(),
            text: text.into(),
            cursor: 0,
            grid,
            loss_milli: None,
        }))
    }

    fn restore(&self, checkpoint: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError> {
        if checkpoint.len() > MAX_CHECKPOINT_BYTES {
            return Err(TrainingError::InvalidCheckpoint);
        }
        let saved: SavedSession =
            serde_json::from_slice(checkpoint).map_err(|_| TrainingError::InvalidCheckpoint)?;
        TrainingSource::Text(saved.text.clone())
            .text()
            .map_err(|_| TrainingError::InvalidCheckpoint)?;
        let grid = GridSyncer::restore(saved.grid).map_err(|_| TrainingError::InvalidCheckpoint)?;
        let recipe_valid = match saved.version {
            1 => saved.recipe.is_none() && grid.job() == recipe_hash(&saved.text),
            2 => saved.recipe == Some(recipe_hash(&saved.text)),
            _ => false,
        };
        if !recipe_valid
            || saved.cursor >= saved.text.len()
            || !saved.text.is_char_boundary(saved.cursor)
            || grid.params().len() != BYTE_MODEL_PARAMETERS
            || grid.round() > u64::from(round_limit(&saved.text))
        {
            return Err(TrainingError::InvalidCheckpoint);
        }
        Ok(Box::new(Session {
            pending: BTreeMap::new(),
            text: saved.text,
            cursor: saved.cursor,
            grid,
            loss_milli: saved.loss_milli,
        }))
    }

    fn work(&self, task: TrainingTask, worker: DeviceId) -> Result<TrainingResult, TrainingError> {
        let parameters = task.parameters(worker)?;
        let model = ByteLanguageModel::new(&task.text).map_err(|_| TrainingError::InvalidWork)?;
        let delta = ScaffoldClient::new(model.dimension()).train(
            &model,
            &parameters,
            &vec![0.0; model.dimension()],
            LocalWork {
                steps: usize::try_from(task.lease.steps).map_err(|_| TrainingError::InvalidWork)?,
                learning_rate: 0.1,
                algorithm: Algorithm::FedAvg,
            },
        );
        let tokens = task.expected_tokens()?;
        let pseudo_gradient =
            encode_parameters(&delta.into_iter().map(|x| -x).collect::<Vec<_>>())?;
        Ok(TrainingResult {
            lease: task.lease,
            tokens,
            pseudo_gradient,
        })
    }
}

fn recipe_hash(text: &str) -> ContentHash {
    let mut bytes = b"sovra-byte-grid-session-v1\0".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    ContentHash::of(&bytes)
}

impl TrainingSession for Session {
    fn advance(&mut self, now_ms: u64) -> Result<(), TrainingError> {
        if self.progress().is_complete() {
            return Ok(());
        }
        let worker = DeviceId([0; 32]);
        let task = self.assign(worker, now_ms)?;
        let result = BurnTrainingEngine.work(task, worker)?;
        self.merge(result, worker, now_ms)
    }

    fn abandon(&mut self, worker: DeviceId) {
        let id = self
            .pending
            .iter()
            .find(|(_, batch)| batch.lease.worker == worker)
            .map(|(id, _)| *id);
        if let Some(id) = id
            && let Some(batch) = self.pending.remove(&id)
            && self.cursor == batch.cursor_after
        {
            self.cursor = batch.cursor_before;
        }
        self.grid.release(worker);
    }

    fn assign(&mut self, worker: DeviceId, now_ms: u64) -> Result<TrainingTask, TrainingError> {
        if self.progress().is_complete() {
            return Err(TrainingError::InvalidWork);
        }
        // A worker that asks again replaces its own unfinished lease. The model
        // is unchanged, and the same text interval is issued when the cursor has
        // not moved on for someone else.
        self.abandon(worker);
        let cursor_before = self.cursor;
        // Overlap the last character so a one-byte tail has a predecessor.
        let start = if self.text.len() - self.cursor < 2 {
            self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        } else {
            self.cursor
        };
        let mut end = (start + BATCH_BYTES).min(self.text.len());
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        let lease = self
            .grid
            .issue(worker, now_ms)
            .map_err(|_| TrainingError::InvalidWork)?;
        self.pending
            .retain(|_, task| self.grid.has_lease(&task.lease));
        let task = TrainingTask {
            recipe: TrainingRecipe::ByteBigramV1,
            lease,
            model: encode_parameters(self.grid.params())?,
            text: self.text[start..end].into(),
        };
        // Assignment progress is saved before sending work. A missing worker's
        // batch can be visited again when the cursor cycles through the corpus.
        let cursor_after = if end == self.text.len() {
            0
        } else {
            self.text[..end]
                .char_indices()
                .next_back()
                .map_or(end, |(i, _)| i)
        };
        self.cursor = cursor_after;
        self.pending.insert(
            task.lease.id,
            PendingBatch {
                lease: task.lease.clone(),
                text: task.text.clone(),
                tokens: task.expected_tokens()?,
                cursor_before,
                cursor_after,
            },
        );
        Ok(task)
    }

    fn merge(
        &mut self,
        result: TrainingResult,
        worker: DeviceId,
        now_ms: u64,
    ) -> Result<(), TrainingError> {
        if self.progress().is_complete() {
            return Err(TrainingError::InvalidWork);
        }
        let task = self
            .pending
            .get(&result.lease.id)
            .ok_or(TrainingError::InvalidWork)?;
        if task.lease != result.lease || task.tokens != result.tokens {
            return Err(TrainingError::InvalidWork);
        }
        let contribution = result.contribution(worker)?;
        self.grid
            .merge(&[contribution], now_ms)
            .map_err(|_| TrainingError::InvalidWork)?;
        let model = ByteLanguageModel::new(&task.text).map_err(|_| TrainingError::Step)?;
        let loss = model.loss(self.grid.params());
        if !loss.is_finite() || !(0.0..=1000.0).contains(&loss) {
            return Err(TrainingError::Step);
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "finite nonnegative loss is bounded to 1000 above"
        )]
        {
            self.loss_milli = Some((loss * 1000.0).round() as u32);
        }
        self.pending.remove(&result.lease.id);
        Ok(())
    }

    fn progress(&self) -> TrainingProgress {
        TrainingProgress {
            rounds: u32::try_from(self.grid.round()).expect("bounded session rounds"),
            total_rounds: round_limit(&self.text),
            tokens: self.grid.tokens(),
            loss_milli: self.loss_milli,
        }
    }

    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError> {
        let saved = SavedSession {
            version: 2,
            recipe: Some(recipe_hash(&self.text)),
            text: self.text.clone(),
            cursor: self.cursor,
            grid: self.grid.checkpoint(),
            loss_milli: self.loss_milli,
        };
        let bytes = serde_json::to_vec(&saved).map_err(|_| TrainingError::Step)?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(TrainingError::Step);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::training::SAMPLE_TEXT;

    #[test]
    fn saved_session_restores_text_cursor_model_and_optimizer() {
        let engine = BurnTrainingEngine;
        let mut original = engine
            .create(SAMPLE_TEXT, ContentHash::of(b"test run"))
            .unwrap();
        for _ in 0..3 {
            original.advance(0).unwrap();
        }
        let bytes = original.checkpoint().unwrap();
        let mut restored = engine.restore(&bytes).unwrap();
        original.advance(0).unwrap();
        restored.advance(0).unwrap();
        assert_eq!(restored.progress(), original.progress());
        assert_eq!(
            restored.checkpoint().unwrap(),
            original.checkpoint().unwrap()
        );
        assert!(engine.restore(&bytes[..bytes.len() / 2]).is_err());
        let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        changed["text"] = SAMPLE_TEXT.replace("garden", "castle").into();
        assert!(
            engine
                .restore(&serde_json::to_vec(&changed).unwrap())
                .is_err()
        );
    }

    #[test]
    fn unicode_batches_and_last_short_batch_do_not_panic() {
        let mut session = BurnTrainingEngine
            .create(
                &format!("{}x", "🌱".repeat(128)),
                ContentHash::of(b"unicode run"),
            )
            .unwrap();
        session.advance(0).unwrap();
        session.advance(0).unwrap();
        assert_eq!(session.progress().rounds, 2);
    }

    #[test]
    fn independent_learners_handle_dropout_replay_restore_and_distinct_runs() {
        let engine = BurnTrainingEngine;
        let fast = DeviceId([1; 32]);
        let slow = DeviceId([2; 32]);
        let gone = DeviceId([3; 32]);
        let mut session = engine
            .create(SAMPLE_TEXT, ContentHash::of(b"first"))
            .unwrap();
        let slow_task = session.assign(slow, 0).unwrap();
        let abandoned = session.assign(gone, 0).unwrap();
        let initial_loss = ByteLanguageModel::new(SAMPLE_TEXT)
            .unwrap()
            .loss(&slow_task.parameters(slow).unwrap());
        let first = session.assign(fast, 0).unwrap();
        let first = engine.work(first, fast).unwrap();
        let mut forged = first.clone();
        forged.tokens -= 1;
        let before = session.checkpoint().unwrap();
        assert!(session.merge(forged, fast, 1).is_err());
        assert_eq!(before, session.checkpoint().unwrap());
        session.merge(first.clone(), fast, 1).unwrap();
        let saved = session.checkpoint().unwrap();
        assert!(session.merge(first, fast, 1).is_err());
        assert_eq!(saved, session.checkpoint().unwrap());
        session
            .merge(engine.work(slow_task, slow).unwrap(), slow, 2)
            .unwrap();
        for now in 3..12 {
            let task = session.assign(fast, now).unwrap();
            session
                .merge(engine.work(task, fast).unwrap(), fast, now)
                .unwrap();
        }
        assert!(
            f64::from(session.progress().loss_milli.unwrap()) < f64::from(initial_loss) * 1000.0
        );
        assert!(
            session
                .merge(engine.work(abandoned, gone).unwrap(), gone, 12)
                .is_err()
        );
        let outstanding = session.assign(fast, 13).unwrap();
        let checkpoint = session.checkpoint().unwrap();
        let mut restored = engine.restore(&checkpoint).unwrap();
        let result = engine.work(outstanding.clone(), fast).unwrap();
        assert!(restored.merge(result.clone(), fast, 14).is_err());
        let replacement = restored.assign(fast, 14).unwrap();
        assert!(replacement.lease.id > outstanding.lease.id);
        restored
            .merge(engine.work(replacement, fast).unwrap(), fast, 15)
            .unwrap();
        let mut another = engine
            .create(SAMPLE_TEXT, ContentHash::of(b"second"))
            .unwrap();
        let fresh = another.assign(fast, 14).unwrap();
        assert_ne!(fresh.lease.job, outstanding.lease.job);
        assert!(another.merge(result, fast, 15).is_err());
    }

    #[test]
    fn a_dropped_worker_keeps_the_saved_model_and_can_take_the_same_interval_again() {
        let engine = BurnTrainingEngine;
        let worker = DeviceId([9; 32]);
        let mut session = engine
            .create(SAMPLE_TEXT, ContentHash::of(b"dropout"))
            .unwrap();
        let first = session.assign(worker, 0).unwrap();
        let rounds = session.progress().rounds;
        let saved = session.checkpoint().unwrap();
        let again = session.assign(worker, 1).unwrap();
        assert_eq!(again.text, first.text);
        assert!(again.lease.id > first.lease.id);
        assert_eq!(session.progress().rounds, rounds);
        let reassigned = session.checkpoint().unwrap();
        let mut before: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        let after: serde_json::Value = serde_json::from_slice(&reassigned).unwrap();
        assert!(
            after["grid"]["next_lease"].as_u64().unwrap()
                > before["grid"]["next_lease"].as_u64().unwrap()
        );
        before["grid"]["next_lease"] = after["grid"]["next_lease"].clone();
        assert!(
            before == after,
            "reassignment preserves model, optimizer and progress"
        );
        assert!(
            session
                .merge(engine.work(first, worker).unwrap(), worker, 2)
                .is_err()
        );
        assert!(
            session.checkpoint().unwrap() == reassigned,
            "rejected work cannot change the checkpoint"
        );
        session
            .merge(engine.work(again, worker).unwrap(), worker, 2)
            .unwrap();
        assert_eq!(session.progress().rounds, rounds + 1);
    }

    #[test]
    fn legacy_local_checkpoints_upgrade_without_losing_model_state() {
        let mut session = BurnTrainingEngine
            .create(SAMPLE_TEXT, recipe_hash(SAMPLE_TEXT))
            .unwrap();
        session.advance(0).unwrap();
        let mut saved: serde_json::Value =
            serde_json::from_slice(&session.checkpoint().unwrap()).unwrap();
        saved["version"] = 1.into();
        saved.as_object_mut().unwrap().remove("recipe");
        let mut restored = BurnTrainingEngine
            .restore(&serde_json::to_vec(&saved).unwrap())
            .unwrap();
        restored.advance(0).unwrap();
        session.advance(0).unwrap();
        assert_eq!(
            restored.checkpoint().unwrap(),
            session.checkpoint().unwrap()
        );
    }
}
