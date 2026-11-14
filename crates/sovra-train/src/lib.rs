//! Local training for the sovereign swarm, on the Burn deep-learning framework.
//!
//! Owns the tensor code; decides nothing. It implements two core ports:
//!
//! - [`BurnTrainingEngine`] is the core `TrainingEngine`: byte-level models (the
//!   Starter table and a family of small transformers) that learn from a person's
//!   own text, write text, and report what this computer is. The runtime drives
//!   it; group computers help through bounded work packets.
//! - [`LinearModel`] and [`ByteLanguageModel`] are core `Objective`s, so the
//!   swarm's rules (SCAFFOLD, privacy noise, secure sums, sign-off) only ever see
//!   a flat parameter vector and its gradient.
//!
//! Devices are picked explicitly: no environment variable chooses one, nothing
//! is downloaded, and no external program is involved. Burn's pure-Rust
//! processor backend is always there. With the `gpu` feature, transformers train
//! on a graphics card instead when one passes the checks in `device`: a real
//! card, the same answers as the processor, and clearly faster. A card that
//! fails mid-run hands its work back to the processor. Gradients come from
//! Burn's autodiff on either.
//!
//! [`reference`] is a textbook trainer for the `training_benchmark` example to
//! compare real runs against.

mod device;
mod hardware;
mod init;
mod language;
mod net;
mod optim;
mod packet;
pub mod reference;
mod rng;
mod saved;
mod session;
mod writer;
pub use device::{EngineChoice, GraphicsCard};
pub use session::BurnTrainingEngine;

pub use language::ByteLanguageModel;

use burn::tensor::{Device, Tensor, TensorData};
use sovra_core::swarm::learn::scaffold::Objective;

/// Why a set of examples was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrainError {
    #[error("there are no examples to learn from")]
    NoExamples,
    #[error("choose a text batch containing 2 to 4097 bytes")]
    InvalidTextBatch,
    #[error("every example needs the same number of features")]
    RaggedExamples,
}

/// Approved examples: one row of features and one target number each.
#[derive(Debug, Clone, PartialEq)]
pub struct Examples {
    features: Vec<f32>,
    targets: Vec<f32>,
    rows: usize,
    width: usize,
}

impl Examples {
    pub fn new(rows: Vec<Vec<f32>>, targets: Vec<f32>) -> Result<Self, TrainError> {
        let width = rows.first().map(Vec::len).ok_or(TrainError::NoExamples)?;
        if width == 0 || rows.len() != targets.len() || rows.iter().any(|r| r.len() != width) {
            return Err(TrainError::RaggedExamples);
        }
        Ok(Self {
            rows: rows.len(),
            width,
            features: rows.into_iter().flatten().collect(),
            targets,
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn width(&self) -> usize {
        self.width
    }
}

/// `y = w·x + b` fitted by mean squared error. Parameters are laid out as
/// `[w₀, …, w_{d−1}, b]`.
#[derive(Debug, Clone)]
pub struct LinearModel {
    examples: Examples,
    device: Device,
}

fn read(tensor: Tensor<2>) -> Vec<f32> {
    tensor
        .into_data()
        .as_slice::<f32>()
        .expect("these tensors are created as f32")
        .to_vec()
}

impl LinearModel {
    /// Trains on Burn's built-in processor backend, with autodiff.
    pub fn new(examples: Examples) -> Self {
        Self {
            examples,
            device: Device::flex().autodiff(),
        }
    }

    fn inputs(&self) -> (Tensor<2>, Tensor<2>) {
        let e = &self.examples;
        let x = Tensor::<2>::from_data(
            TensorData::new(e.features.clone(), [e.rows, e.width]),
            &self.device,
        );
        let y = Tensor::<2>::from_data(
            TensorData::new(e.targets.clone(), [e.rows, 1]),
            &self.device,
        );
        (x, y)
    }

    fn weights(&self, params: &[f32]) -> (Tensor<2>, Tensor<2>) {
        let width = self.examples.width;
        let w = Tensor::<2>::from_data(
            TensorData::new(params[..width].to_vec(), [width, 1]),
            &self.device,
        );
        let b = Tensor::<2>::from_data(TensorData::new(vec![params[width]], [1, 1]), &self.device);
        (w, b)
    }

    /// Mean squared error as a tensor, for `loss` and `gradient` alike.
    fn mse(&self, w: Tensor<2>, b: Tensor<2>) -> Tensor<1> {
        let (x, y) = self.inputs();
        let error = x.matmul(w).add(b).sub(y);
        error.powf_scalar(2.0).mean()
    }
}

impl Objective for LinearModel {
    fn dimension(&self) -> usize {
        self.examples.width + 1
    }

    fn loss(&self, params: &[f32]) -> f32 {
        let (w, b) = self.weights(params);
        self.mse(w, b)
            .into_data()
            .as_slice::<f32>()
            .expect("these tensors are created as f32")[0]
    }

    fn gradient(&self, params: &[f32]) -> Vec<f32> {
        let (w, b) = self.weights(params);
        let (w, b) = (w.require_grad(), b.require_grad());
        let grads = self.mse(w.clone(), b.clone()).backward();
        let mut out = w
            .grad(&grads)
            .map_or_else(|| vec![0.0; self.examples.width], read);
        out.extend(b.grad(&grads).map_or_else(|| vec![0.0], read));
        out
    }
}

#[cfg(test)]
mod tests;
