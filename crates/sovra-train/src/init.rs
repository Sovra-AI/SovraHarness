//! The values a fresh model starts from, decided by the seed alone.

use sovra_core::training::{Architecture, BlockKind};

use crate::rng::Rng;

/// Spread of freshly drawn weights, as in the small GPT family.
const SPREAD: f64 = 0.02;

/// A fresh model's flat parameter vector, laid out as [`Architecture::blocks`].
/// The Starter table starts at zero (every next byte equally likely); a
/// transformer draws its weights from the seed, block by block, so the same seed
/// gives the same model everywhere.
pub(crate) fn fresh(architecture: Architecture, seed: u64) -> Vec<f32> {
    let mut values = vec![0.0; architecture.parameters()];
    // Weights that write back into the running sum are drawn smaller the deeper
    // the model, so the sum starts at a similar size whatever the depth.
    let residual = SPREAD / (2.0 * f64::from(architecture.layers.max(1))).sqrt();
    for (index, block) in architecture.blocks().into_iter().enumerate() {
        let spread = match block.kind {
            BlockKind::Table
            | BlockKind::AttentionNormBias
            | BlockKind::QueryKeyValueBias
            | BlockKind::AttentionOutBias
            | BlockKind::FeedForwardNormBias
            | BlockKind::FeedForwardInBias
            | BlockKind::FeedForwardOutBias
            | BlockKind::FinalNormBias
            | BlockKind::OutputBias => continue,
            BlockKind::AttentionNormGain
            | BlockKind::FeedForwardNormGain
            | BlockKind::FinalNormGain => {
                values[block.range()].fill(1.0);
                continue;
            }
            BlockKind::ByteEmbedding
            | BlockKind::PositionEmbedding
            | BlockKind::QueryKeyValueWeight
            | BlockKind::FeedForwardInWeight
            | BlockKind::OutputWeight => SPREAD,
            BlockKind::AttentionOutWeight | BlockKind::FeedForwardOutWeight => residual,
        };
        let mut rng = Rng::stream(seed, index as u64);
        for value in &mut values[block.range()] {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a small normal draw is far inside the f32 range"
            )]
            {
                *value = (rng.normal() * spread) as f32;
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use sovra_core::training::BASE_MODELS;

    use super::*;

    #[test]
    fn every_base_model_starts_with_exactly_its_parameter_count() {
        for row in BASE_MODELS {
            let values = fresh(row.architecture, 1);
            assert_eq!(values.len(), row.architecture.parameters(), "{}", row.title);
            assert!(values.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn the_seed_alone_decides_a_fresh_model() {
        let shape = Architecture {
            context: 8,
            width: 8,
            layers: 1,
            heads: 2,
        };
        assert_eq!(fresh(shape, 5), fresh(shape, 5));
        assert_ne!(fresh(shape, 5), fresh(shape, 6));
        assert!(fresh(Architecture::STARTER, 5).iter().all(|x| *x == 0.0));
        let values = fresh(shape, 5);
        for block in shape.blocks() {
            let part = &values[block.range()];
            if block.kind == BlockKind::FinalNormGain {
                assert!(part.iter().all(|x| (*x - 1.0).abs() < 1e-9));
            }
            if block.kind == BlockKind::OutputBias {
                assert!(part.iter().all(|x| *x == 0.0));
            }
            if block.kind == BlockKind::ByteEmbedding {
                assert!(part.iter().any(|x| *x != 0.0));
                assert!(part.iter().all(|x| x.abs() < 0.2));
            }
        }
    }
}
