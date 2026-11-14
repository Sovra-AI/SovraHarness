//! The local optimiser: the steps one computer takes on one packet of text
//! before its result is folded into the shared model.
//!
//! Plain Rust over the flat parameter vector. Its memory (Adam's running
//! averages) lives for one packet only: nothing about it travels with the work,
//! which would double every message. The merge engine keeps the run's long-lived
//! momentum, so what matters here is the direction the packet pulls in.

use std::ops::Range;

use sovra_core::training::{LocalOptimizer, WorkRecipe};

const BETA1: f32 = 0.9;
const BETA2: f32 = 0.999;
const EPSILON: f32 = 1e-8;

pub(crate) struct Optimizer {
    kind: LocalOptimizer,
    learning_rate: f32,
    weight_decay: f32,
    /// Largest gradient size allowed; zero leaves gradients as they are.
    clip: f32,
    first: Vec<f32>,
    second: Vec<f32>,
    steps: i32,
}

#[expect(
    clippy::cast_precision_loss,
    reason = "bounded settings: millionths and thousandths far below 2^24 matter only approximately"
)]
fn fraction(value: u32, per: f32) -> f32 {
    value as f32 / per
}

impl Optimizer {
    pub(crate) fn new(recipe: &WorkRecipe, parameters: usize) -> Self {
        let moments = match recipe.optimizer {
            LocalOptimizer::Sgd => 0,
            LocalOptimizer::Adam => parameters,
        };
        Self {
            kind: recipe.optimizer,
            learning_rate: fraction(recipe.learning_rate_micro, 1e6),
            weight_decay: fraction(recipe.weight_decay_micro, 1e6),
            clip: fraction(recipe.clip_milli, 1e3),
            first: vec![0.0; moments],
            second: vec![0.0; moments],
            steps: 0,
        }
    }

    /// Changes the step size and keeps the running averages: only the reference
    /// trainer, whose optimiser lives for a whole run, needs it.
    pub(crate) fn set_learning_rate(&mut self, learning_rate: f32) {
        self.learning_rate = learning_rate;
    }

    /// Moves `params` one step against `gradient`. `matrices` are the ranges
    /// weight decay applies to.
    pub(crate) fn step(
        &mut self,
        params: &mut [f32],
        gradient: &mut [f32],
        matrices: &[Range<usize>],
    ) {
        if self.clip > 0.0 {
            let size = gradient
                .iter()
                .map(|x| f64::from(*x).powi(2))
                .sum::<f64>()
                .sqrt();
            if size > f64::from(self.clip) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a ratio between zero and one"
                )]
                let scale = (f64::from(self.clip) / size) as f32;
                for value in gradient.iter_mut() {
                    *value *= scale;
                }
            }
        }
        match self.kind {
            LocalOptimizer::Sgd => {
                // Classic weight decay: part of the gradient.
                if self.weight_decay > 0.0 {
                    for range in matrices {
                        for (g, p) in gradient[range.clone()]
                            .iter_mut()
                            .zip(&params[range.clone()])
                        {
                            *g += self.weight_decay * p;
                        }
                    }
                }
                for (p, g) in params.iter_mut().zip(gradient.iter()) {
                    *p -= self.learning_rate * g;
                }
            }
            LocalOptimizer::Adam => {
                self.steps = self.steps.saturating_add(1);
                let correct1 = 1.0 - BETA1.powi(self.steps);
                let correct2 = 1.0 - BETA2.powi(self.steps);
                // Decoupled weight decay (AdamW): shrink first, then step.
                if self.weight_decay > 0.0 {
                    let keep = 1.0 - self.learning_rate * self.weight_decay;
                    for range in matrices {
                        for p in &mut params[range.clone()] {
                            *p *= keep;
                        }
                    }
                }
                for (((p, g), m), v) in params
                    .iter_mut()
                    .zip(gradient.iter())
                    .zip(&mut self.first)
                    .zip(&mut self.second)
                {
                    *m = BETA1 * *m + (1.0 - BETA1) * g;
                    *v = BETA2 * *v + (1.0 - BETA2) * g * g;
                    *p -= self.learning_rate * (*m / correct1) / ((*v / correct2).sqrt() + EPSILON);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use sovra_core::training::Architecture;

    use super::*;

    fn recipe(optimizer: LocalOptimizer) -> WorkRecipe {
        WorkRecipe {
            architecture: Architecture::STARTER,
            windows: 1,
            learning_rate_micro: 100_000,
            optimizer,
            weight_decay_micro: 0,
            clip_milli: 0,
            seed: 0,
        }
    }

    #[test]
    fn plain_steps_follow_the_gradient_and_decay_only_matrices() {
        let mut params = vec![1.0, 1.0, 1.0];
        let mut optimizer = Optimizer::new(&recipe(LocalOptimizer::Sgd), 3);
        optimizer.step(&mut params, &mut [1.0, -2.0, 0.0], &[]);
        assert_eq!(params, vec![0.9, 1.2, 1.0]);
        let mut decayed = WorkRecipe {
            weight_decay_micro: 500_000,
            ..recipe(LocalOptimizer::Sgd)
        };
        let matrix: Vec<Range<usize>> = std::iter::once(0..1).collect();
        let mut params = vec![1.0, 1.0];
        Optimizer::new(&decayed, 2).step(&mut params, &mut [0.0, 0.0], &matrix);
        assert!((params[0] - 0.95).abs() < 1e-6);
        assert!((params[1] - 1.0).abs() < 1e-6);
        decayed.optimizer = LocalOptimizer::Adam;
        let mut params = vec![1.0, 1.0];
        Optimizer::new(&decayed, 2).step(&mut params, &mut [0.0, 0.0], &matrix);
        assert!((params[0] - 0.95).abs() < 1e-6);
        assert!((params[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn adam_takes_even_first_steps_whatever_the_gradient_size() {
        let mut params = vec![0.0, 0.0];
        let mut optimizer = Optimizer::new(&recipe(LocalOptimizer::Adam), 2);
        optimizer.step(&mut params, &mut [1000.0, -0.001], &[]);
        assert!((params[0] + 0.1).abs() < 1e-4);
        assert!((params[1] - 0.1).abs() < 1e-3);
        optimizer.step(&mut params, &mut [1000.0, -0.001], &[]);
        assert!((params[0] + 0.2).abs() < 1e-3);
    }

    #[test]
    fn clipping_limits_the_size_of_one_correction() {
        let clipped = WorkRecipe {
            clip_milli: 1_000,
            learning_rate_micro: 1_000_000,
            ..recipe(LocalOptimizer::Sgd)
        };
        let mut params = vec![0.0, 0.0];
        Optimizer::new(&clipped, 2).step(&mut params, &mut [3.0, 4.0], &[]);
        assert!((params[0] + 0.6).abs() < 1e-6);
        assert!((params[1] + 0.8).abs() < 1e-6);
        let mut params = vec![0.0, 0.0];
        Optimizer::new(&clipped, 2).step(&mut params, &mut [0.3, 0.4], &[]);
        assert!((params[0] + 0.3).abs() < 1e-6);
    }
}
