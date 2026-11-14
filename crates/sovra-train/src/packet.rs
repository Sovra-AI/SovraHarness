//! One packet of learning: the same code on the computer that owns a run and on
//! every computer helping it.
//!
//! A packet is a piece of text, the model to start from, and a bounded recipe.
//! The result is how far the local steps moved the model. Both ends count the
//! same number of next-byte guesses for it, so a helper cannot claim more work
//! than the packet holds.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock};

use rayon::{ThreadPool, ThreadPoolBuilder};
use sovra_core::training::{TrainingError, WorkRecipe};

use crate::device::{self, Engine, EngineChoice, Graphics};
use crate::net::{Batch, Net};
use crate::optim::Optimizer;
use crate::rng::Rng;

/// Where the tensor work runs. Burn's built-in processor backend is always
/// there, chosen explicitly so no environment variable can redirect (or crash)
/// it, with the threads its matrix products may use. A graphics card does the
/// transformers' work instead when the engine may use one and one works.
#[derive(Clone)]
pub(crate) struct Compute {
    processor: Engine,
    choice: EngineChoice,
    pool: Arc<ThreadPool>,
}

/// Measured on an eight-core processor, these models train within a few percent
/// of the same speed on one, four or sixteen threads: their matrices are small.
/// Four is the most that ever helped, and it leaves the computer pleasant to use
/// while it trains.
const MAX_AUTOMATIC_THREADS: usize = 4;

fn build_pool(threads: usize) -> Result<Arc<ThreadPool>, TrainingError> {
    ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .thread_name(|index| format!("sovra-train-{index}"))
        .build()
        .map(Arc::new)
        .map_err(|_| TrainingError::Step)
}

/// Threads used when the person has not chosen a number: one per processor core
/// (not per hardware thread), within the cap.
pub(crate) fn automatic_threads() -> usize {
    let logical = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let cores = sysinfo::System::physical_core_count().unwrap_or(logical);
    cores.clamp(1, MAX_AUTOMATIC_THREADS).min(logical)
}

impl Compute {
    /// `threads` of zero means automatic. The automatic pool is shared by every
    /// run and helper packet in the process; a chosen number gets its own.
    pub(crate) fn new(threads: u8, choice: EngineChoice) -> Result<Self, TrainingError> {
        static SHARED: OnceLock<Arc<ThreadPool>> = OnceLock::new();
        let pool = if threads == 0 {
            if let Some(pool) = SHARED.get() {
                Arc::clone(pool)
            } else {
                let pool = build_pool(automatic_threads())?;
                Arc::clone(SHARED.get_or_init(|| pool))
            }
        } else {
            build_pool(usize::from(threads))?
        };
        Ok(Self {
            processor: Engine::processor(),
            choice,
            pool,
        })
    }

    /// Runs tensor work on this compute's own threads.
    fn run<T: Send>(&self, work: impl FnOnce() -> T + Send) -> T {
        self.pool.install(work)
    }

    /// The graphics card, when this work should run on it. The Starter table is
    /// one lookup: sending it to a card costs more than doing it.
    fn graphics(&self, net: &Net) -> Option<&'static Graphics> {
        match self.choice {
            EngineChoice::Processor => None,
            EngineChoice::Automatic if net.architecture().is_table() => None,
            EngineChoice::Automatic => device::graphics(),
        }
    }

    /// Runs `work` on the graphics card when one is in use, and on the processor
    /// otherwise. A card that panics (a lost device, a driver fault, no memory
    /// left) is retired and the same work is done on the processor, so a run
    /// never stops because of the card.
    fn on<T: Send>(
        &self,
        net: &Net,
        work: impl Fn(&Engine) -> Result<T, TrainingError> + Sync,
    ) -> Result<T, TrainingError> {
        card_first(
            self.graphics(net).map(|card| &card.engine),
            &work,
            || device::retire("a step on the graphics card failed"),
            || self.run(|| work(&self.processor)),
        )
    }

    /// Mean loss of `params` on `batch`, without learning.
    pub(crate) fn loss(
        &self,
        net: &Net,
        params: &[f32],
        batch: &Batch,
    ) -> Result<f32, TrainingError> {
        self.on(net, |engine| net.loss(params, batch, &engine.plain))
    }

    /// The loss and its gradient for every parameter.
    pub(crate) fn gradient(
        &self,
        net: &Net,
        params: &[f32],
        batch: &Batch,
    ) -> Result<(f32, Vec<f32>), TrainingError> {
        self.on(net, |engine| net.gradient(params, batch, &engine.learning))
    }
}

/// Tries `work` on `card` when there is one, and falls back to `on_processor`.
/// A panic on the card calls `retire` first. Work the card refuses as invalid is
/// refused outright, since it is invalid on any engine; any other failure on
/// the card (a result that is not a number, say) is left to the processor to
/// decide.
fn card_first<T>(
    card: Option<&Engine>,
    work: &impl Fn(&Engine) -> Result<T, TrainingError>,
    retire: impl FnOnce(),
    on_processor: impl FnOnce() -> Result<T, TrainingError>,
) -> Result<T, TrainingError> {
    if let Some(card) = card {
        match catch_unwind(AssertUnwindSafe(|| work(card))) {
            Ok(Ok(done)) => return Ok(done),
            Ok(Err(TrainingError::InvalidWork)) => return Err(TrainingError::InvalidWork),
            Ok(Err(_)) => {}
            Err(_) => retire(),
        }
    }
    on_processor()
}

/// The windows step `step` reads. The Starter table reads the whole packet as
/// one window. A transformer reads `recipe.windows` windows of its context
/// length (or the whole packet when that is shorter): the ones `starts` names
/// for this step, or, when the packet names none, ones placed by `rng`.
pub(crate) fn step_batch(
    net: &Net,
    recipe: &WorkRecipe,
    text: &[u8],
    starts: &[usize],
    step: usize,
    rng: &mut Rng,
) -> Result<Batch, TrainingError> {
    let length = net.architecture().window_tokens(text.len());
    if length == 0 {
        return Err(TrainingError::InvalidWork);
    }
    if net.architecture().is_table() {
        return Batch::windows(text, &[0], length);
    }
    let windows = usize::from(recipe.windows);
    if starts.is_empty() {
        let choices = text.len() - length;
        let placed: Vec<usize> = (0..windows).map(|_| rng.below(choices)).collect();
        return Batch::windows(text, &placed, length);
    }
    let named = step
        .checked_mul(windows)
        .and_then(|from| starts.get(from..from.checked_add(windows)?))
        .ok_or(TrainingError::InvalidWork)?;
    Batch::windows(text, named, length)
}

/// What a packet's local steps produced.
pub(crate) struct Learned {
    /// Start minus finish for every parameter: the direction the merge engine
    /// folds in.
    pub(crate) pseudo_gradient: Vec<f32>,
    /// Mean loss over the steps, each measured before its own correction.
    pub(crate) loss: f32,
}

/// Takes `steps` local steps on `text` from `base`, exactly as `recipe` says.
/// `starts` are the packet's window positions, empty when the seed places them.
pub(crate) fn learn(
    compute: &Compute,
    net: &Net,
    recipe: &WorkRecipe,
    base: &[f32],
    text: &[u8],
    starts: &[usize],
    steps: u32,
) -> Result<Learned, TrainingError> {
    if recipe.architecture != net.architecture() || steps == 0 {
        return Err(TrainingError::InvalidWork);
    }
    let matrices = net.matrices();
    let mut params = base.to_vec();
    let mut optimizer = Optimizer::new(recipe, params.len());
    let mut rng = Rng::new(recipe.seed);
    let mut total = 0.0_f64;
    for step in 0..steps as usize {
        let batch = step_batch(net, recipe, text, starts, step, &mut rng)?;
        let (loss, mut gradient) = compute.gradient(net, &params, &batch)?;
        if gradient.iter().any(|x| !x.is_finite()) {
            return Err(TrainingError::Step);
        }
        total += f64::from(loss);
        optimizer.step(&mut params, &mut gradient, &matrices);
    }
    let pseudo_gradient: Vec<f32> = base.iter().zip(&params).map(|(a, b)| a - b).collect();
    if pseudo_gradient.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::Step);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a mean of finite f32 losses"
    )]
    let loss = (total / f64::from(steps)) as f32;
    Ok(Learned {
        pseudo_gradient,
        loss,
    })
}

/// The loss of `params` on the windows a packet's first step reads: what the
/// owner shows after folding in a helper's result.
pub(crate) fn packet_loss(
    compute: &Compute,
    net: &Net,
    recipe: &WorkRecipe,
    params: &[f32],
    text: &[u8],
    starts: &[usize],
) -> Result<f32, TrainingError> {
    let batch = step_batch(net, recipe, text, starts, 0, &mut Rng::new(recipe.seed))?;
    compute.loss(net, params, &batch)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// What the pretend card does with a step.
    #[derive(Clone, Copy)]
    enum Card {
        Works,
        Panics,
        Refuses,
        Fails,
    }

    /// Runs one step with a pretend card and reports the answer, whether the
    /// card was retired, and whether the processor was asked.
    fn step(card: Option<Card>) -> (Result<&'static str, TrainingError>, bool, bool) {
        let engine = Engine::processor();
        let retired = Cell::new(false);
        let asked = Cell::new(false);
        let answer = card_first(
            card.is_some().then_some(&engine),
            &|_| match card {
                Some(Card::Works) | None => Ok("card"),
                Some(Card::Panics) => panic!("the pretend card is lost"),
                Some(Card::Refuses) => Err(TrainingError::InvalidWork),
                Some(Card::Fails) => Err(TrainingError::Step),
            },
            || retired.set(true),
            || {
                asked.set(true);
                Ok("processor")
            },
        );
        (answer, retired.get(), asked.get())
    }

    #[test]
    fn a_card_that_fails_hands_the_step_to_the_processor() {
        assert_eq!(step(None), (Ok("processor"), false, true));
        assert_eq!(step(Some(Card::Works)), (Ok("card"), false, false));
        // A lost card is retired and the same step is done on the processor.
        assert_eq!(step(Some(Card::Panics)), (Ok("processor"), true, true));
        // Invalid work is invalid everywhere: nobody is asked again.
        assert_eq!(
            step(Some(Card::Refuses)),
            (Err(TrainingError::InvalidWork), false, false)
        );
        // Any other failure is the processor's to decide; the card stays.
        assert_eq!(step(Some(Card::Fails)), (Ok("processor"), false, true));
    }

    #[test]
    fn the_processor_engine_never_looks_for_a_card() {
        let compute = Compute::new(1, EngineChoice::Processor).unwrap();
        let small = Net::new(sovra_core::training::BaseModel::Small.architecture()).unwrap();
        assert!(compute.graphics(&small).is_none());
        // The Starter table stays on the processor whatever the choice.
        let table = Net::new(sovra_core::training::Architecture::STARTER).unwrap();
        let automatic = Compute::new(1, EngineChoice::Automatic).unwrap();
        assert!(automatic.graphics(&table).is_none());
    }

    /// Only meaningful on a computer with a working card and a build with the
    /// graphics engine; elsewhere there is nothing to compare.
    #[test]
    fn a_card_in_use_learns_what_the_processor_learns() {
        let small = sovra_core::training::BaseModel::Small.architecture();
        let net = Net::new(small).unwrap();
        let on_card = Compute::new(0, EngineChoice::Automatic).unwrap();
        if on_card.graphics(&net).is_none() {
            return;
        }
        let on_processor = Compute::new(0, EngineChoice::Processor).unwrap();
        let recipe = WorkRecipe {
            architecture: small,
            windows: 4,
            learning_rate_micro: 3_000,
            optimizer: sovra_core::training::LocalOptimizer::Sgd,
            weight_decay_micro: 0,
            clip_milli: 0,
            seed: 7,
        };
        let text = "the garden grows, the garden rests, and so do we. ".repeat(8);
        let base = crate::init::fresh(small, 3);
        let learn = |compute: &Compute| {
            learn(compute, &net, &recipe, &base, text.as_bytes(), &[], 2).unwrap()
        };
        let (card, processor) = (learn(&on_card), learn(&on_processor));
        assert!((card.loss - processor.loss).abs() < 1e-3);
        for (a, b) in card.pseudo_gradient.iter().zip(&processor.pseudo_gradient) {
            assert!((a - b).abs() < 1e-5, "{a} against {b}");
        }
        // When a run is over, the memory it held on the card is handed back.
        if let Some(held) = device::reserved_bytes() {
            device::release();
            let left = device::reserved_bytes().unwrap();
            assert!(held > 0 && left < held, "{held} bytes held, {left} left");
        }
    }
}
