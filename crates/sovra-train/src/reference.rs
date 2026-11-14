//! A textbook trainer to hold real runs up against.
//!
//! One model, one `AdamW` whose memory lasts the whole run, a warm-up and a
//! cosine fall in the step size, and windows drawn from anywhere in the text. It
//! shares the model code, the optimiser arithmetic and the check with real runs,
//! so a difference in the result comes from how a run is organised (packets,
//! rounds, the merge) and from nothing else. The `training_benchmark` example
//! runs it beside a real run; the app never does.

use std::time::Instant;

use sovra_core::training::{
    BaseModel, LocalOptimizer, TrainingError, TrainingSettings, WorkRecipe, check_text,
};

use crate::device::EngineChoice;
use crate::net::{Batch, Net};
use crate::optim::Optimizer;
use crate::packet::Compute;
use crate::rng::Rng;
use crate::{init, session};

/// What the reference trainer measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceResult {
    pub steps: u32,
    /// The loss the last step saw, before its own correction.
    pub loss: f32,
    /// The loss on the held-back end of the text, measured exactly as a real
    /// run measures it; `None` when no text was held back.
    pub check_loss: Option<f32>,
    pub seconds: f64,
}

/// Trains a fresh `model` on `text` for `steps` steps, the textbook way, with
/// the windows per step, peak step size (the round step size), weight decay,
/// step limit, check share and seed that `settings` gives a real run.
pub fn train(
    choice: EngineChoice,
    model: BaseModel,
    settings: &TrainingSettings,
    text: &str,
    steps: u32,
) -> Result<ReferenceResult, TrainingError> {
    check_text(text)?;
    let architecture = model.architecture();
    if architecture.is_table() || steps == 0 {
        return Err(TrainingError::InvalidWork);
    }
    let net = Net::new(architecture)?;
    let compute = Compute::new(settings.threads, choice)?;
    let (learn, held_back) = text.as_bytes().split_at(settings.check_split(text));
    let window = architecture.window_tokens(learn.len());
    if window == 0 {
        return Err(TrainingError::InvalidWork);
    }
    let recipe = WorkRecipe {
        architecture,
        windows: settings.windows,
        learning_rate_micro: settings.outer_learning_rate_micro,
        optimizer: LocalOptimizer::Adam,
        weight_decay_micro: settings.weight_decay_micro,
        clip_milli: settings.clip_milli,
        seed: settings.seed,
    };
    let mut params = init::fresh(architecture, settings.seed);
    let mut optimizer = Optimizer::new(&recipe, params.len());
    let matrices = net.matrices();
    let mut rng = Rng::new(settings.seed);
    let peak = f64::from(settings.outer_learning_rate_micro) / 1e6;
    let warm_up = (steps / 20).max(5).min(steps);
    let started = Instant::now();
    let mut loss = f32::NAN;
    for step in 0..steps {
        let share = if step < warm_up {
            f64::from(step + 1) / f64::from(warm_up)
        } else {
            let progress = f64::from(step - warm_up) / f64::from((steps - warm_up).max(1));
            0.1 + 0.9 * f64::midpoint(1.0, (std::f64::consts::PI * progress).cos())
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a step size of at most one"
        )]
        optimizer.set_learning_rate((peak * share) as f32);
        let starts: Vec<usize> = (0..settings.windows)
            .map(|_| rng.below(learn.len() - window))
            .collect();
        let batch = Batch::windows(learn, &starts, window)?;
        let (seen, mut gradient) = compute.gradient(&net, &params, &batch)?;
        if gradient.iter().any(|value| !value.is_finite()) {
            return Err(TrainingError::Step);
        }
        loss = seen;
        optimizer.step(&mut params, &mut gradient, &matrices);
    }
    Ok(ReferenceResult {
        steps,
        loss,
        check_loss: session::check_loss(&compute, &net, &params, held_back),
        seconds: started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use sovra_core::training::{HardwareInfo, TrainingPreset};

    use super::*;

    #[test]
    fn the_textbook_trainer_learns_and_refuses_what_it_cannot_train() {
        let text = "the cat sat on the mat. ".repeat(40);
        let small = BaseModel::Small;
        let mut settings = TrainingSettings::recommended(
            TrainingPreset::Quick,
            small.architecture(),
            text.len(),
            &HardwareInfo::default(),
        );
        settings.windows = 2;
        let engine = EngineChoice::Processor;
        let start = train(engine, small, &settings, &text, 1).unwrap();
        // A fresh model finds every byte about equally likely: ln 256 = 5.545.
        assert!((start.loss - 5.545).abs() < 0.2, "{start:?}");
        let later = train(engine, small, &settings, &text, 25).unwrap();
        assert_eq!(later.steps, 25);
        assert!(later.loss < start.loss - 1.0, "{later:?}");
        assert!(later.check_loss.unwrap() < start.check_loss.unwrap());
        // The same seed gives the same run.
        assert_eq!(
            train(engine, small, &settings, &text, 25).unwrap().loss,
            later.loss
        );
        assert!(train(engine, BaseModel::Starter, &settings, &text, 5).is_err());
        assert!(train(engine, small, &settings, &text, 0).is_err());
        assert!(train(engine, small, &settings, "too short", 5).is_err());
    }
}
