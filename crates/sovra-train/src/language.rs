//! The Starter table as a core `Objective`: a next-byte model the swarm's own
//! learning rules can drive directly.
//!
//! It predicts each byte from the one before it, with a 256 × 256 table. Every
//! UTF-8 byte uses the same fixed vocabulary on every worker; no tokenizer file,
//! Python runtime, network download, or external training process is required.
//! Training runs use the same table through `net`, beside the transformers.

use burn::nn::loss::CrossEntropyLossConfig;
use burn::tensor::{Device, Int, Tensor, TensorData};
use sovra_core::swarm::learn::scaffold::Objective;

use crate::TrainError;

const VOCABULARY: usize = 256;
const MAX_BATCH_BYTES: usize = 4097;

/// Predicts the next byte from one preceding byte using a 256 × 256 logits table.
/// A caller supplies an explicitly selected, bounded batch, not an entire file.
#[derive(Debug, Clone)]
pub struct ByteLanguageModel {
    inputs: Vec<i32>,
    targets: Vec<i32>,
    device: Device,
}

impl ByteLanguageModel {
    pub fn new(text: &str) -> Result<Self, TrainError> {
        if text.len() < 2 || text.len() > MAX_BATCH_BYTES {
            return Err(TrainError::InvalidTextBatch);
        }
        let bytes = text.as_bytes();
        Ok(Self {
            inputs: bytes[..bytes.len() - 1]
                .iter()
                .map(|b| i32::from(*b))
                .collect(),
            targets: bytes[1..].iter().map(|b| i32::from(*b)).collect(),
            device: Device::flex().autodiff(),
        })
    }

    /// Number of next-token predictions contributing to each gradient.
    pub fn tokens(&self) -> u32 {
        u32::try_from(self.inputs.len()).expect("text batch bounded to 4096 predictions")
    }

    fn weights(&self, params: &[f32]) -> Tensor<2> {
        Tensor::from_data(
            TensorData::new(params.to_vec(), [VOCABULARY, VOCABULARY]),
            &self.device,
        )
    }

    fn cross_entropy(&self, weights: Tensor<2>) -> Tensor<1> {
        let inputs = Tensor::<1, Int>::from_data(
            TensorData::new(self.inputs.clone(), [self.inputs.len()]),
            &self.device,
        );
        let targets = Tensor::<1, Int>::from_data(
            TensorData::new(self.targets.clone(), [self.targets.len()]),
            &self.device,
        );
        CrossEntropyLossConfig::new()
            .init(&self.device)
            .forward(weights.select(0, inputs), targets)
    }
}

impl Objective for ByteLanguageModel {
    fn dimension(&self) -> usize {
        VOCABULARY * VOCABULARY
    }

    fn loss(&self, params: &[f32]) -> f32 {
        self.cross_entropy(self.weights(params))
            .into_data()
            .as_slice::<f32>()
            .expect("f32 logits produce f32 loss")[0]
    }

    fn gradient(&self, params: &[f32]) -> Vec<f32> {
        let weights = self.weights(params).require_grad();
        let grads = self.cross_entropy(weights.clone()).backward();
        crate::read(weights.grad(&grads).expect("loss depends on logits"))
    }
}

#[cfg(test)]
mod tests {
    use sovra_core::group::{ContentHash, DeviceId};
    use sovra_core::swarm::learn::scaffold::{Algorithm, LocalWork, ScaffoldClient};
    use sovra_core::swarm::learn::scavenger::{Contribution, GridSettings, GridSyncer};

    use super::*;

    #[test]
    fn bounded_text_batches_and_fixed_byte_vocabulary() {
        assert!(ByteLanguageModel::new("").is_err());
        assert!(ByteLanguageModel::new("a").is_err());
        assert!(ByteLanguageModel::new(&"a".repeat(MAX_BATCH_BYTES + 1)).is_err());
        let model = ByteLanguageModel::new("مرحبا").unwrap();
        assert_eq!(model.tokens(), 9);
        assert_eq!(model.dimension(), 65_536);
    }

    #[test]
    fn cross_entropy_and_autodiff_match_analytic_bigram_gradient() {
        let model = ByteLanguageModel::new("abab").unwrap();
        let params = vec![0.0; model.dimension()];
        assert!((model.loss(&params) - 256.0_f32.ln()).abs() < 1e-5);
        let grads = model.gradient(&params);
        let ab = usize::from(b'a') * VOCABULARY + usize::from(b'b');
        let aa = usize::from(b'a') * VOCABULARY + usize::from(b'a');
        assert!((grads[ab] - (1.0 / 256.0 - 1.0) * 2.0 / 3.0).abs() < 1e-5);
        assert!((grads[aa] - (1.0 / 256.0) * 2.0 / 3.0).abs() < 1e-5);
        assert_eq!(grads[0], 0.0);
    }

    #[test]
    fn independent_language_workers_lower_loss_despite_worker_loss_and_syncer_restart() {
        let workers = [
            ByteLanguageModel::new("abababababababab").unwrap(),
            ByteLanguageModel::new("babababababababa").unwrap(),
        ];
        let dimension = workers[0].dimension();
        let settings = GridSettings {
            learning_rate: 0.5,
            local_steps: 4,
            ..GridSettings::default()
        };
        let mut grid = GridSyncer::new(
            ContentHash::of(b"sample bigram v1"),
            vec![0.0; dimension],
            settings,
        )
        .unwrap();
        let before = workers[0].loss(grid.params());
        // Worker 9 vanishes. The others never wait for this assignment.
        let lost = grid.issue(DeviceId([9; 32]), 0).unwrap();
        for round in 0..8 {
            // Unequal participation and token counts exercise an asynchronous fleet.
            let active = if round % 2 == 0 { 2 } else { 1 };
            let mut updates = Vec::new();
            for (i, worker) in workers.iter().enumerate().take(active) {
                let lease = grid
                    .issue(DeviceId([u8::try_from(i).unwrap(); 32]), round)
                    .unwrap();
                let delta = ScaffoldClient::new(dimension).train(
                    worker,
                    grid.params(),
                    &vec![0.0; dimension],
                    LocalWork {
                        steps: usize::try_from(lease.steps).unwrap(),
                        learning_rate: 0.1,
                        algorithm: Algorithm::FedAvg,
                    },
                );
                updates.push(Contribution {
                    tokens: worker.tokens() * lease.steps,
                    lease,
                    pseudo_gradient: delta.into_iter().map(|x| -x).collect(),
                });
            }
            grid.merge(&updates, round).unwrap();
            if round == 3 {
                grid = GridSyncer::restore(grid.checkpoint()).unwrap();
            }
        }
        let after = workers[0].loss(grid.params());
        assert!(after < before * 0.25, "cross entropy {before} -> {after}");
        assert_eq!(grid.round(), 8);
        assert_eq!(grid.tokens(), 720);
        assert!(
            grid.merge(
                &[Contribution {
                    lease: lost,
                    tokens: 1,
                    pseudo_gradient: vec![0.0; dimension]
                }],
                9
            )
            .is_err()
        );
    }
}
