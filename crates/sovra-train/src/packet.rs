//! One packet of learning: the same code on the computer that owns a run and on
//! every computer helping it.
//!
//! A packet is a piece of text, the model to start from, and a bounded recipe.
//! The result is how far the local steps moved the model. Both ends count the
//! same number of next-byte guesses for it, so a helper cannot claim more work
//! than the packet holds.

use std::sync::{Arc, OnceLock};

use burn::tensor::Device;
use rayon::{ThreadPool, ThreadPoolBuilder};
use sovra_core::training::{TrainingError, WorkRecipe};

use crate::net::{Batch, Net};
use crate::optim::Optimizer;
use crate::rng::Rng;

/// Where the tensor work runs: Burn's built-in processor backend, chosen
/// explicitly so no environment variable can redirect (or crash) it, and the
/// threads its matrix products may use.
#[derive(Clone)]
pub(crate) struct Compute {
    plain: Device,
    learning: Device,
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
    pub(crate) fn new(threads: u8) -> Result<Self, TrainingError> {
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
            plain: Device::flex(),
            learning: Device::flex().autodiff(),
            pool,
        })
    }

    /// Runs tensor work on this compute's own threads.
    fn run<T: Send>(&self, work: impl FnOnce() -> T + Send) -> T {
        self.pool.install(work)
    }

    /// Mean loss of `params` on `batch`, without learning.
    pub(crate) fn loss(
        &self,
        net: &Net,
        params: &[f32],
        batch: &Batch,
    ) -> Result<f32, TrainingError> {
        self.run(|| net.loss(params, batch, &self.plain))
    }
}

/// The windows one step reads. The Starter table reads the whole packet as one
/// window; a transformer reads `recipe.windows` windows of its context length (or
/// the whole packet when that is shorter), placed by `rng`.
pub(crate) fn step_batch(
    net: &Net,
    recipe: &WorkRecipe,
    text: &[u8],
    rng: &mut Rng,
) -> Result<Batch, TrainingError> {
    let length = net.architecture().window_tokens(text.len());
    if length == 0 {
        return Err(TrainingError::InvalidWork);
    }
    if net.architecture().is_table() {
        return Batch::windows(text, &[0], length);
    }
    let choices = text.len() - length;
    let starts: Vec<usize> = (0..recipe.windows).map(|_| rng.below(choices)).collect();
    Batch::windows(text, &starts, length)
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
pub(crate) fn learn(
    compute: &Compute,
    net: &Net,
    recipe: &WorkRecipe,
    base: &[f32],
    text: &[u8],
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
    for _ in 0..steps {
        let batch = step_batch(net, recipe, text, &mut rng)?;
        let (loss, mut gradient) =
            compute.run(|| net.gradient(&params, &batch, &compute.learning))?;
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
) -> Result<f32, TrainingError> {
    let batch = step_batch(net, recipe, text, &mut Rng::new(recipe.seed))?;
    compute.loss(net, params, &batch)
}
