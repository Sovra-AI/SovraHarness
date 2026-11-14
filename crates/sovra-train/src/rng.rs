//! A small seeded generator, so a seed always gives the same model, the same
//! reading order and the same sample text on every computer.
//!
//! Burn's own random tensors share one process-wide seed, which any other tensor
//! user could move; nothing here touches it.

/// `SplitMix64`: 64 bits of state, passes the usual statistical tests, and is
/// written out here so the sequence can never change under a dependency update.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A generator for one numbered part of a seeded job, independent of the others.
    pub(crate) fn stream(seed: u64, part: u64) -> Self {
        let mut rng = Self(seed ^ part.wrapping_mul(0xD6E8_FEB8_6659_FD93));
        rng.next();
        rng
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`, from the top 53 bits.
    pub(crate) fn unit(&mut self) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a 53-bit integer is exactly representable as f64"
        )]
        let value = (self.next() >> 11) as f64;
        value / 9_007_199_254_740_992.0
    }

    /// Uniform in `0..bound`; `bound` of zero gives zero.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        // Widening multiply: unbiased enough for choosing windows and far from
        // any overflow on every pointer width.
        let wide = u128::from(self.next()) * bound as u128;
        usize::try_from(wide >> 64).unwrap_or(bound - 1)
    }

    /// Standard normal by Box–Muller.
    pub(crate) fn normal(&mut self) -> f64 {
        let radius = (-2.0 * (1.0 - self.unit()).ln()).sqrt();
        radius * (std::f64::consts::TAU * self.unit()).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_is_fixed_and_streams_differ() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        assert_eq!(a.next(), b.next());
        // The published first output of SplitMix64 seeded with zero.
        assert_eq!(Rng::new(0).next(), 0xE220_A839_7B1D_CDAF);
        assert_ne!(Rng::stream(7, 0).next(), Rng::stream(7, 1).next());
    }

    #[test]
    fn draws_stay_in_range_and_look_normal() {
        let mut rng = Rng::new(9);
        let mut sum = 0.0;
        let mut squares = 0.0;
        for _ in 0..20_000 {
            let unit = rng.unit();
            assert!((0.0..1.0).contains(&unit));
            assert!(rng.below(7) < 7);
            let value = rng.normal();
            assert!(value.is_finite());
            sum += value;
            squares += value * value;
        }
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
        assert!((sum / 20_000.0).abs() < 0.03);
        assert!((squares / 20_000.0 - 1.0).abs() < 0.05);
    }
}
