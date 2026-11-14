//! The models themselves, as tensor code over one flat parameter vector.
//!
//! The Starter table and every byte transformer share this one entry point. A
//! model is never a long-lived Burn module: each call builds one leaf tensor per
//! [`ParameterBlock`] straight from the caller's slice, runs the forward pass, and
//! (when learning) copies each leaf's gradient back to the same place in a flat
//! vector. The flat vector is what the merge engine, the wire and the model file
//! all carry, so its layout is fixed by [`Architecture::blocks`] alone.

use burn::nn::loss::CrossEntropyLossConfig;
use burn::tensor::activation::gelu;
use burn::tensor::module::{attention, embedding, layer_norm, linear};
use burn::tensor::ops::AttentionModuleOptions;
use burn::tensor::{Device, Int, Tensor, TensorData};
use sovra_core::training::{Architecture, ParameterBlock, TrainingError, VOCABULARY};

/// Added to the variance inside every layer norm.
pub(crate) const NORM_EPSILON: f64 = 1e-5;

/// Equal-length windows of text: for each position the byte seen and the byte
/// that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Batch {
    inputs: Vec<i32>,
    targets: Vec<i32>,
    rows: usize,
    cols: usize,
}

impl Batch {
    /// One window per `start`, each reading `length` bytes and predicting the byte
    /// after each of them. Every window must lie inside `text`.
    pub(crate) fn windows(
        text: &[u8],
        starts: &[usize],
        length: usize,
    ) -> Result<Self, TrainingError> {
        if length == 0
            || starts.is_empty()
            || starts.iter().any(|start| {
                start
                    .checked_add(length)
                    .is_none_or(|end| end >= text.len())
            })
        {
            return Err(TrainingError::InvalidWork);
        }
        let mut inputs = Vec::with_capacity(starts.len() * length);
        let mut targets = Vec::with_capacity(starts.len() * length);
        for start in starts {
            inputs.extend(text[*start..start + length].iter().map(|b| i32::from(*b)));
            targets.extend(
                text[start + 1..=start + length]
                    .iter()
                    .map(|b| i32::from(*b)),
            );
        }
        Ok(Self {
            inputs,
            targets,
            rows: starts.len(),
            cols: length,
        })
    }

    /// Next-byte guesses in this batch.
    pub(crate) fn tokens(&self) -> usize {
        self.rows * self.cols
    }
}

/// A model shape ready to run: its checked layout, and nothing else.
#[derive(Debug, Clone)]
pub(crate) struct Net {
    architecture: Architecture,
    blocks: Vec<ParameterBlock>,
}

/// The leaves of one transformer layer, in layout order.
struct Layer {
    norm1: (Tensor<1>, Tensor<1>),
    qkv: (Tensor<2>, Tensor<1>),
    out: (Tensor<2>, Tensor<1>),
    norm2: (Tensor<1>, Tensor<1>),
    wide: (Tensor<2>, Tensor<1>),
    narrow: (Tensor<2>, Tensor<1>),
}

impl Net {
    pub(crate) fn new(architecture: Architecture) -> Result<Self, TrainingError> {
        architecture.validate()?;
        Ok(Self {
            architecture,
            blocks: architecture.blocks(),
        })
    }

    pub(crate) fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Flat ranges holding weight matrices, the only values weight decay shrinks:
    /// gains and offsets are left alone.
    pub(crate) fn matrices(&self) -> Vec<std::ops::Range<usize>> {
        self.blocks
            .iter()
            .filter(|block| block.rows > 1)
            .map(ParameterBlock::range)
            .collect()
    }

    /// Burn's shape checks panic, so everything a tensor is built from is checked
    /// here first.
    fn check(&self, params: &[f32], batch: &Batch) -> Result<(), TrainingError> {
        let window_fits =
            self.architecture.is_table() || batch.cols <= usize::from(self.architecture.context);
        if params.len() != self.architecture.parameters()
            || batch.rows == 0
            || batch.cols == 0
            || batch.inputs.len() != batch.tokens()
            || batch.targets.len() != batch.tokens()
            || !window_fits
        {
            return Err(TrainingError::InvalidWork);
        }
        Ok(())
    }

    fn leaves(&self, params: &[f32], device: &Device, learn: bool) -> Vec<Tensor<2>> {
        self.blocks
            .iter()
            .map(|block| {
                let leaf = Tensor::<2>::from_data(
                    TensorData::new(params[block.range()].to_vec(), [block.rows, block.cols]),
                    device,
                );
                if learn { leaf.require_grad() } else { leaf }
            })
            .collect()
    }

    /// Scores for the next byte at every position: `[rows, cols, 256]`.
    fn logits(&self, leaves: &[Tensor<2>], batch: &Batch, device: &Device) -> Tensor<3> {
        let (rows, cols) = (batch.rows, batch.cols);
        let tokens = Tensor::<2, Int>::from_data(
            TensorData::new(batch.inputs.clone(), [rows, cols]),
            device,
        );
        if self.architecture.is_table() {
            // Row = the byte seen, column = the byte that follows.
            return embedding(leaves[0].clone(), tokens);
        }
        let width = usize::from(self.architecture.width);
        let heads = usize::from(self.architecture.heads);
        let flat = |index: usize| -> Tensor<1> {
            let block = &self.blocks[index];
            leaves[index].clone().reshape([block.cols])
        };
        let positions = leaves[1].clone().narrow(0, 0, cols).unsqueeze::<3>();
        let mut x = embedding(leaves[0].clone(), tokens) + positions;
        let causal = AttentionModuleOptions {
            scale: None,
            softcap: None,
            is_causal: true,
        };
        for layer in 0..usize::from(self.architecture.layers) {
            let at = 2 + layer * 12;
            let part = Layer {
                norm1: (flat(at), flat(at + 1)),
                qkv: (leaves[at + 2].clone(), flat(at + 3)),
                out: (leaves[at + 4].clone(), flat(at + 5)),
                norm2: (flat(at + 6), flat(at + 7)),
                wide: (leaves[at + 8].clone(), flat(at + 9)),
                narrow: (leaves[at + 10].clone(), flat(at + 11)),
            };
            let normed = layer_norm(x.clone(), part.norm1.0, Some(part.norm1.1), NORM_EPSILON);
            let qkv = linear(normed, part.qkv.0, Some(part.qkv.1));
            let head = |third: usize| -> Tensor<4> {
                qkv.clone()
                    .narrow(2, third * width, width)
                    .reshape([rows, cols, heads, width / heads])
                    .swap_dims(1, 2)
            };
            let mixed = attention(head(0), head(1), head(2), None, None, causal)
                .swap_dims(1, 2)
                .reshape([rows, cols, width]);
            x = x + linear(mixed, part.out.0, Some(part.out.1));
            let normed = layer_norm(x.clone(), part.norm2.0, Some(part.norm2.1), NORM_EPSILON);
            let hidden = gelu(linear(normed, part.wide.0, Some(part.wide.1)));
            x = x + linear(hidden, part.narrow.0, Some(part.narrow.1));
        }
        let at = 2 + usize::from(self.architecture.layers) * 12;
        let normed = layer_norm(x, flat(at), Some(flat(at + 1)), NORM_EPSILON);
        linear(normed, leaves[at + 2].clone(), Some(flat(at + 3)))
    }

    fn cross_entropy(&self, leaves: &[Tensor<2>], batch: &Batch, device: &Device) -> Tensor<1> {
        let count = batch.tokens();
        let targets =
            Tensor::<1, Int>::from_data(TensorData::new(batch.targets.clone(), [count]), device);
        let logits = self
            .logits(leaves, batch, device)
            .reshape([count, VOCABULARY]);
        CrossEntropyLossConfig::new()
            .init(device)
            .forward(logits, targets)
    }

    /// Mean cross entropy (nats per byte) of `params` on `batch`. `device` should
    /// be the plain one: no gradient graph is needed.
    pub(crate) fn loss(
        &self,
        params: &[f32],
        batch: &Batch,
        device: &Device,
    ) -> Result<f32, TrainingError> {
        self.check(params, batch)?;
        let leaves = self.leaves(params, device, false);
        scalar(self.cross_entropy(&leaves, batch, device))
    }

    /// The loss and its gradient with respect to every parameter, in the flat
    /// layout. `device` must be the autodiff one.
    pub(crate) fn gradient(
        &self,
        params: &[f32],
        batch: &Batch,
        device: &Device,
    ) -> Result<(f32, Vec<f32>), TrainingError> {
        self.check(params, batch)?;
        let leaves = self.leaves(params, device, true);
        let loss = self.cross_entropy(&leaves, batch, device);
        let mut grads = loss.backward();
        let mut out = vec![0.0; params.len()];
        for (leaf, block) in leaves.iter().zip(&self.blocks) {
            // A leaf the loss never touched (positions past a short window) has no
            // gradient: its part of the vector stays zero.
            if let Some(grad) = leaf.grad_remove(&mut grads) {
                let data = grad.into_data();
                let values = data.as_slice::<f32>().map_err(|_| TrainingError::Step)?;
                if values.len() != block.len() {
                    return Err(TrainingError::Step);
                }
                out[block.range()].copy_from_slice(values);
            }
        }
        Ok((scalar(loss)?, out))
    }

    /// Scores for the byte after position `at` of a single window. Used to check
    /// the plain-Rust writer against the tensor code.
    #[cfg(test)]
    pub(crate) fn logits_at(
        &self,
        params: &[f32],
        window: &[u8],
        at: usize,
        device: &Device,
    ) -> Result<Vec<f32>, TrainingError> {
        let mut padded = window.to_vec();
        padded.push(0);
        let batch = Batch::windows(&padded, &[0], window.len())?;
        self.check(params, &batch)?;
        if at >= window.len() {
            return Err(TrainingError::InvalidWork);
        }
        let leaves = self.leaves(params, device, false);
        let data = self
            .logits(&leaves, &batch, device)
            .narrow(1, at, 1)
            .into_data();
        Ok(data
            .as_slice::<f32>()
            .map_err(|_| TrainingError::Step)?
            .to_vec())
    }
}

fn scalar(loss: Tensor<1>) -> Result<f32, TrainingError> {
    let data = loss.into_data();
    let value = data
        .as_slice::<f32>()
        .ok()
        .and_then(|values| values.first().copied())
        .ok_or(TrainingError::Step)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(TrainingError::Step)
    }
}

#[cfg(test)]
mod tests {
    use sovra_core::swarm::learn::scaffold::Objective;
    use sovra_core::training::{BASE_MODELS, BlockKind};

    use super::*;
    use crate::{ByteLanguageModel, init, rng::Rng};

    const MICRO: Architecture = Architecture {
        context: 8,
        width: 8,
        layers: 1,
        heads: 2,
    };

    fn lively(architecture: Architecture, seed: u64) -> Vec<f32> {
        let mut params = init::fresh(architecture, seed);
        let mut rng = Rng::new(seed);
        for value in &mut params {
            #[expect(clippy::cast_possible_truncation, reason = "test values are small")]
            {
                *value = *value * 10.0 + (rng.normal() * 0.05) as f32;
            }
        }
        params
    }

    #[test]
    fn every_base_model_has_exactly_the_parameters_core_promises() {
        for row in BASE_MODELS {
            let architecture = row.architecture;
            let net = Net::new(architecture).unwrap();
            let total: usize = net.blocks.iter().map(ParameterBlock::len).sum();
            assert_eq!(total, architecture.parameters(), "{}", row.title);
            // Contiguous, in order, nothing shared and nothing left over.
            let mut next = 0;
            for block in &net.blocks {
                assert_eq!(block.offset, next);
                next += block.len();
            }
            assert_eq!(next, architecture.parameters());
            assert_eq!(
                init::fresh(architecture, 1).len(),
                architecture.parameters()
            );
        }
        assert_eq!(BASE_MODELS.len(), 4);
    }

    #[test]
    fn the_forward_pass_reads_the_blocks_in_the_order_core_lays_them_out() {
        use BlockKind as K;
        let per_layer = [
            K::AttentionNormGain,
            K::AttentionNormBias,
            K::QueryKeyValueWeight,
            K::QueryKeyValueBias,
            K::AttentionOutWeight,
            K::AttentionOutBias,
            K::FeedForwardNormGain,
            K::FeedForwardNormBias,
            K::FeedForwardInWeight,
            K::FeedForwardInBias,
            K::FeedForwardOutWeight,
            K::FeedForwardOutBias,
        ];
        for row in BASE_MODELS
            .iter()
            .filter(|row| !row.architecture.is_table())
        {
            let kinds: Vec<K> = row.architecture.blocks().iter().map(|b| b.kind).collect();
            let mut expected = vec![K::ByteEmbedding, K::PositionEmbedding];
            for _ in 0..row.architecture.layers {
                expected.extend(per_layer);
            }
            expected.extend([
                K::FinalNormGain,
                K::FinalNormBias,
                K::OutputWeight,
                K::OutputBias,
            ]);
            assert_eq!(kinds, expected, "{}", row.title);
        }
        let table: Vec<K> = Architecture::STARTER
            .blocks()
            .iter()
            .map(|b| b.kind)
            .collect();
        assert_eq!(table, vec![K::Table]);
    }

    #[test]
    fn autodiff_matches_finite_differences_in_every_block() {
        let net = Net::new(MICRO).unwrap();
        let params = lively(MICRO, 7);
        let text = b"the garden grows, the garden rests";
        let batch = Batch::windows(text, &[0, 9, 20], 8).unwrap();
        let (loss, gradient) = net
            .gradient(&params, &batch, &Device::flex().autodiff())
            .unwrap();
        let plain = Device::flex();
        assert!((loss - net.loss(&params, &batch, &plain).unwrap()).abs() < 1e-5);
        assert_eq!(gradient.len(), params.len());
        let step = 2e-3_f32;
        for block in MICRO.blocks() {
            // The steepest value of each block gives the clearest comparison.
            let index = block
                .range()
                .max_by(|a, b| gradient[*a].abs().total_cmp(&gradient[*b].abs()))
                .unwrap();
            let mut up = params.clone();
            let mut down = params.clone();
            up[index] += step;
            down[index] -= step;
            let numeric = (net.loss(&up, &batch, &plain).unwrap()
                - net.loss(&down, &batch, &plain).unwrap())
                / (2.0 * step);
            assert!(
                (gradient[index] - numeric).abs() < 5e-3 + 0.03 * numeric.abs(),
                "{:?}: autodiff {} vs numeric {numeric}",
                block.kind,
                gradient[index]
            );
            assert!(
                gradient[index].abs() > 1e-6,
                "{:?} takes part in the loss",
                block.kind
            );
        }
    }

    #[test]
    fn a_position_never_sees_the_bytes_after_it() {
        let net = Net::new(MICRO).unwrap();
        let params = lively(MICRO, 8);
        let device = Device::flex();
        let early = net.logits_at(&params, b"abcdefgh", 2, &device).unwrap();
        let changed = net.logits_at(&params, b"abcZZZZZ", 2, &device).unwrap();
        for (a, b) in early.iter().zip(&changed) {
            assert!((a - b).abs() < 1e-5);
        }
        let seen = net.logits_at(&params, b"abZdefgh", 2, &device).unwrap();
        assert!(early.iter().zip(&seen).any(|(a, b)| (a - b).abs() > 1e-4));
    }

    #[test]
    fn the_table_is_the_starter_bigram() {
        let net = Net::new(Architecture::STARTER).unwrap();
        let text = "abab garden";
        let batch = Batch::windows(text.as_bytes(), &[0], text.len() - 1).unwrap();
        let mut params = vec![0.0; Architecture::STARTER.parameters()];
        params[usize::from(b'a') * VOCABULARY + usize::from(b'b')] = 2.0;
        let objective = ByteLanguageModel::new(text).unwrap();
        let (loss, gradient) = net
            .gradient(&params, &batch, &Device::flex().autodiff())
            .unwrap();
        assert!((loss - objective.loss(&params)).abs() < 1e-5);
        for (mine, theirs) in gradient.iter().zip(objective.gradient(&params)) {
            assert!((mine - theirs).abs() < 1e-6);
        }
    }

    #[test]
    fn wrong_shapes_are_refused_before_any_tensor_is_built() {
        let net = Net::new(MICRO).unwrap();
        let params = lively(MICRO, 9);
        let device = Device::flex();
        let text = b"0123456789abcdef";
        let fits = Batch::windows(text, &[0, 4], 8).unwrap();
        assert!(net.loss(&params, &fits, &device).is_ok());
        assert!(net.loss(&params[1..], &fits, &device).is_err());
        // Longer than the model can look back.
        let long = Batch::windows(text, &[0], 9).unwrap();
        assert!(net.loss(&params, &long, &device).is_err());
        assert!(
            net.gradient(&params, &long, &Device::flex().autodiff())
                .is_err()
        );
        // Windows must lie inside the text, with a byte after each to predict.
        assert!(Batch::windows(text, &[9], 8).is_err());
        assert!(Batch::windows(text, &[usize::MAX], 8).is_err());
        assert!(Batch::windows(text, &[], 8).is_err());
        assert!(Batch::windows(text, &[0], 0).is_err());
        assert!(
            Net::new(Architecture {
                context: 8,
                width: 9,
                layers: 1,
                heads: 2
            })
            .is_err()
        );
        // A window shorter than the context is fine: the packet was short.
        let short = Batch::windows(text, &[0], 3).unwrap();
        assert!(net.loss(&params, &short, &device).unwrap().is_finite());
    }
}
