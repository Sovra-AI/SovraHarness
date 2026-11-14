//! Writing text with a trained model, one byte at a time, in plain Rust.
//!
//! Training needs gradients and whole batches, so it runs on Burn. Writing needs
//! neither: it adds one byte and asks what comes next. Doing that with tensors
//! would re-read the whole window for every byte. This reader keeps each layer's
//! keys and values for the bytes already read, so a new byte costs about one pass
//! over the model's weights, and the same model, beginning and seed give the same
//! text on every computer and with any number of threads.

use sovra_core::training::{Architecture, TrainingError, VOCABULARY};

use crate::net::NORM_EPSILON;
use crate::rng::Rng;

/// The byte a model is shown first when it is given no beginning.
const START: u8 = b'\n';

/// Reads bytes with one model and gives the scores for the byte that follows.
pub(crate) struct Writer<'a> {
    architecture: Architecture,
    params: &'a [f32],
    /// Start of each block in `params`, in layout order.
    offsets: Vec<usize>,
    /// Per layer: every read byte's key and value rows, `width` each.
    keys: Vec<Vec<f32>>,
    values: Vec<Vec<f32>>,
    /// The bytes currently inside the window.
    window: Vec<u8>,
    scores: Vec<f32>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(
        architecture: Architecture,
        params: &'a [f32],
    ) -> Result<Self, TrainingError> {
        architecture.validate()?;
        if params.len() != architecture.parameters() {
            return Err(TrainingError::InvalidModel);
        }
        let layers = usize::from(architecture.layers);
        Ok(Self {
            architecture,
            params,
            offsets: architecture
                .blocks()
                .iter()
                .map(|block| block.offset)
                .collect(),
            keys: vec![Vec::new(); layers],
            values: vec![Vec::new(); layers],
            window: Vec::new(),
            scores: vec![0.0; VOCABULARY],
        })
    }

    fn part(&self, index: usize, len: usize) -> &'a [f32] {
        &self.params[self.offsets[index]..self.offsets[index] + len]
    }

    /// Reads one more byte. Afterwards [`Self::scores`] is about the byte after it.
    pub(crate) fn read(&mut self, byte: u8) {
        if self.architecture.is_table() {
            let row = usize::from(byte) * VOCABULARY;
            self.scores
                .copy_from_slice(&self.params[row..row + VOCABULARY]);
            return;
        }
        let context = usize::from(self.architecture.context);
        if self.window.len() == context {
            // Positions are learned for 0..context only. When the window is full,
            // keep its newer half and read that again from position zero.
            let kept = self.window.split_off(context - context / 2);
            self.window.clear();
            for layer in 0..self.keys.len() {
                self.keys[layer].clear();
                self.values[layer].clear();
            }
            for old in kept {
                self.step(old);
            }
        }
        self.step(byte);
    }

    /// Scores (unnormalised log-probabilities) for the next byte.
    pub(crate) fn scores(&self) -> &[f32] {
        &self.scores
    }

    fn step(&mut self, byte: u8) {
        let width = usize::from(self.architecture.width);
        let heads = usize::from(self.architecture.heads);
        let head_width = width / heads;
        let position = self.window.len();
        self.window.push(byte);
        let embedding = self.part(0, VOCABULARY * width);
        let positions = self.part(1, usize::from(self.architecture.context) * width);
        let mut x: Vec<f32> = embedding[usize::from(byte) * width..][..width]
            .iter()
            .zip(&positions[position * width..][..width])
            .map(|(a, b)| a + b)
            .collect();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a head is at most 512 values wide"
        )]
        let scale = 1.0 / (head_width as f32).sqrt();
        for layer in 0..usize::from(self.architecture.layers) {
            let at = 2 + layer * 12;
            let normed = norm(&x, self.part(at, width), self.part(at + 1, width));
            let qkv = affine(
                &normed,
                self.part(at + 2, width * 3 * width),
                self.part(at + 3, 3 * width),
            );
            self.keys[layer].extend_from_slice(&qkv[width..2 * width]);
            self.values[layer].extend_from_slice(&qkv[2 * width..]);
            let (keys, values) = (&self.keys[layer], &self.values[layer]);
            let mut mixed = vec![0.0_f32; width];
            let mut weights = vec![0.0_f32; position + 1];
            for head in 0..heads {
                let span = head * head_width..(head + 1) * head_width;
                let query = &qkv[span.clone()];
                let mut top = f32::NEG_INFINITY;
                for (seen, weight) in weights.iter_mut().enumerate() {
                    let key = &keys[seen * width..][span.clone()];
                    *weight = dot(query, key) * scale;
                    top = top.max(*weight);
                }
                let mut total = 0.0;
                for weight in &mut weights {
                    *weight = (*weight - top).exp();
                    total += *weight;
                }
                for (seen, weight) in weights.iter().enumerate() {
                    let share = weight / total;
                    let value = &values[seen * width..][span.clone()];
                    for (sum, v) in mixed[span.clone()].iter_mut().zip(value) {
                        *sum += share * v;
                    }
                }
            }
            let attended = affine(
                &mixed,
                self.part(at + 4, width * width),
                self.part(at + 5, width),
            );
            add(&mut x, &attended);
            let normed = norm(&x, self.part(at + 6, width), self.part(at + 7, width));
            let mut hidden = affine(
                &normed,
                self.part(at + 8, width * 4 * width),
                self.part(at + 9, 4 * width),
            );
            for value in &mut hidden {
                *value = gelu(*value);
            }
            let fed = affine(
                &hidden,
                self.part(at + 10, 4 * width * width),
                self.part(at + 11, width),
            );
            add(&mut x, &fed);
        }
        let at = 2 + usize::from(self.architecture.layers) * 12;
        let normed = norm(&x, self.part(at, width), self.part(at + 1, width));
        self.scores = affine(
            &normed,
            self.part(at + 2, width * VOCABULARY),
            self.part(at + 3, VOCABULARY),
        );
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn add(into: &mut [f32], from: &[f32]) {
    for (a, b) in into.iter_mut().zip(from) {
        *a += b;
    }
}

/// `input · weight + bias`, with `weight` row-major `[inputs, outputs]`.
fn affine(input: &[f32], weight: &[f32], bias: &[f32]) -> Vec<f32> {
    let mut out = bias.to_vec();
    for (value, row) in input.iter().zip(weight.chunks_exact(bias.len())) {
        for (sum, w) in out.iter_mut().zip(row) {
            *sum += value * w;
        }
    }
    out
}

/// Layer norm over one vector, matching the tensor code: biased variance, epsilon
/// inside the square root.
fn norm(input: &[f32], gain: &[f32], bias: &[f32]) -> Vec<f32> {
    #[expect(clippy::cast_precision_loss, reason = "a width is at most 512")]
    let count = input.len() as f32;
    let mean = input.iter().sum::<f32>() / count;
    let variance = input.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / count;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the epsilon is a small constant"
    )]
    let scale = 1.0 / (variance + NORM_EPSILON as f32).sqrt();
    input
        .iter()
        .zip(gain.iter().zip(bias))
        .map(|(x, (g, b))| (x - mean) * scale * g + b)
        .collect()
}

/// The exact GELU, `x · Φ(x)`, with the error function from Abramowitz and Stegun
/// 7.1.26 (error below 1.5e-7, under what an `f32` can tell apart here).
fn gelu(x: f32) -> f32 {
    let z = f64::from(x) * std::f64::consts::FRAC_1_SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z.abs());
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = (1.0 - poly * (-z * z).exp()).copysign(z);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the result is no larger in size than the f32 input"
    )]
    {
        (0.5 * f64::from(x) * (1.0 + erf)) as f32
    }
}

/// Picks the next byte from its scores. A temperature of zero always takes the
/// likeliest byte; higher values are more adventurous.
fn pick(scores: &[f32], temperature_milli: u32, rng: &mut Rng) -> u8 {
    let likeliest = || {
        let mut best = 0;
        for (index, score) in scores.iter().enumerate() {
            if *score > scores[best] {
                best = index;
            }
        }
        best
    };
    let index = if temperature_milli == 0 {
        likeliest()
    } else {
        let temperature = f64::from(temperature_milli) / 1000.0;
        let top = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let weights: Vec<f64> = scores
            .iter()
            .map(|score| (f64::from(score - top) / temperature).exp())
            .collect();
        let total: f64 = weights.iter().sum();
        let mut left = rng.unit() * total;
        let mut chosen = None;
        for (index, weight) in weights.iter().enumerate() {
            if left < *weight {
                chosen = Some(index);
                break;
            }
            left -= weight;
        }
        // Rounding can leave a sliver past the last weight.
        chosen.unwrap_or_else(likeliest)
    };
    u8::try_from(index).unwrap_or(u8::MAX)
}

/// Continues `prompt` with `count` bytes written by the model. The same model,
/// prompt, temperature and seed always give the same bytes.
pub(crate) fn write(
    architecture: Architecture,
    params: &[f32],
    prompt: &[u8],
    count: usize,
    temperature_milli: u32,
    seed: u64,
) -> Result<Vec<u8>, TrainingError> {
    let mut writer = Writer::new(architecture, params)?;
    let mut rng = Rng::new(seed);
    if prompt.is_empty() {
        writer.read(START);
    }
    // Only the end of a long beginning can still be seen.
    let seen = prompt.len().min(usize::from(architecture.context.max(1)));
    for byte in &prompt[prompt.len() - seen..] {
        writer.read(*byte);
    }
    let mut written = Vec::with_capacity(count);
    for _ in 0..count {
        let byte = pick(writer.scores(), temperature_milli, &mut rng);
        written.push(byte);
        writer.read(byte);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use burn::tensor::Device;

    use super::*;
    use crate::{init, net::Net};

    const MICRO: Architecture = Architecture {
        context: 8,
        width: 8,
        layers: 2,
        heads: 2,
    };

    fn lively(seed: u64) -> Vec<f32> {
        // Fresh weights are tiny, which would hide a wrong layer: scale them up so
        // every part of the forward pass matters.
        let mut params = init::fresh(MICRO, seed);
        let mut rng = Rng::new(seed);
        for value in &mut params {
            #[expect(clippy::cast_possible_truncation, reason = "test values are small")]
            {
                *value = *value * 20.0 + (rng.normal() * 0.05) as f32;
            }
        }
        params
    }

    #[test]
    fn the_plain_reader_agrees_with_the_tensor_model_at_every_position() {
        let params = lively(3);
        let net = Net::new(MICRO).unwrap();
        let device = Device::flex();
        let text = b"garden!?";
        let mut writer = Writer::new(MICRO, &params).unwrap();
        for end in 1..=text.len() {
            writer.read(text[end - 1]);
            // Against the whole window, not just the bytes read so far: the tensor
            // model must not let a position see what comes after it.
            let expected = net.logits_at(&params, text, end - 1, &device).unwrap();
            for (mine, theirs) in writer.scores().iter().zip(&expected) {
                assert!(
                    (mine - theirs).abs() < 2e-4,
                    "position {end}: {mine} vs {theirs}"
                );
            }
        }
    }

    #[test]
    fn a_full_window_slides_and_keeps_reading() {
        let params = lively(4);
        let net = Net::new(MICRO).unwrap();
        let text = b"abcdefghijk";
        let mut writer = Writer::new(MICRO, &params).unwrap();
        for byte in text {
            writer.read(*byte);
        }
        // Eight bytes filled the window; the ninth kept the newer four and went on.
        let expected = net
            .logits_at(&params, &text[4..], 6, &Device::flex())
            .unwrap();
        for (mine, theirs) in writer.scores().iter().zip(&expected) {
            assert!((mine - theirs).abs() < 2e-4);
        }
    }

    #[test]
    fn the_table_reads_its_row() {
        let mut params = vec![0.0; Architecture::STARTER.parameters()];
        params[usize::from(b'q') * VOCABULARY + usize::from(b'u')] = 9.0;
        let written = write(Architecture::STARTER, &params, b"q", 1, 0, 0).unwrap();
        assert_eq!(written, b"u");
    }

    #[test]
    fn writing_is_decided_by_the_seed_and_respects_temperature() {
        let params = lively(5);
        let one = write(MICRO, &params, b"ab", 40, 900, 11).unwrap();
        assert_eq!(one, write(MICRO, &params, b"ab", 40, 900, 11).unwrap());
        assert_ne!(one, write(MICRO, &params, b"ab", 40, 900, 12).unwrap());
        assert_eq!(one.len(), 40);
        // Greedy writing ignores the seed.
        assert_eq!(
            write(MICRO, &params, b"ab", 40, 0, 1).unwrap(),
            write(MICRO, &params, b"ab", 40, 0, 2).unwrap()
        );
        // No beginning, and one longer than the window, both work.
        assert_eq!(write(MICRO, &params, b"", 5, 500, 1).unwrap().len(), 5);
        assert_eq!(
            write(MICRO, &params, &[b'x'; 100], 5, 500, 1)
                .unwrap()
                .len(),
            5
        );
        assert!(write(MICRO, &params[1..], b"", 5, 500, 1).is_err());
    }

    #[test]
    fn gelu_matches_known_values() {
        assert!(gelu(0.0).abs() < 1e-7);
        assert!((gelu(1.0) - 0.841_344_7).abs() < 1e-5);
        assert!((gelu(-1.0) + 0.158_655_3).abs() < 1e-5);
        assert!((gelu(3.0) - 2.995_950_2).abs() < 1e-5);
    }
}
