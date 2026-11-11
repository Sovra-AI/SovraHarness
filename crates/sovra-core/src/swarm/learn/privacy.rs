//! Differential privacy for model updates, before they leave a computer.
//!
//! - **Clip** each update to an L2 norm `C`, so no single computer can move the
//!   model much, then **quantise** it to integers so it can be masked and summed
//!   exactly ([`super::secagg`]).
//! - Add **discrete Gaussian** noise, sampled *exactly* with integer arithmetic
//!   (Canonne, Kamath & Steinke, `NeurIPS` 2020, <https://arxiv.org/abs/2004.00010>).
//!   Floating-point Gaussian samplers leak through their rounding (Mironov 2012);
//!   `OpenDP` uses exact samplers for this reason, and so does this module.
//! - Noise is **distributed**: each of n computers adds a share, so only the secure
//!   sum carries the full noise (Kairouz, Liu & Steinke, ICML 2021,
//!   <https://arxiv.org/abs/2102.06387>).
//! - A **zCDP accountant** adds up the cost of every round and refuses a round that
//!   would exceed the budget; [`epsilon_for`] reports the familiar (ε, δ).

use super::RandomSource;

// ---------------------------------------------------------------------------
// Exact samplers (integer arithmetic only)
// ---------------------------------------------------------------------------

fn uniform_below(n: u128, rng: &mut dyn RandomSource) -> u128 {
    debug_assert!(n > 0, "the range must not be empty");
    let draw = |rng: &mut dyn RandomSource| {
        (u128::from(rng.next_u64()) << 64) | u128::from(rng.next_u64())
    };
    // Rejection sampling: unbiased.
    let zone = u128::MAX - (u128::MAX % n);
    loop {
        let x = draw(rng);
        if x < zone {
            return x % n;
        }
    }
}

/// `true` with probability `num / den`.
fn bernoulli(num: u128, den: u128, rng: &mut dyn RandomSource) -> bool {
    uniform_below(den, rng) < num
}

/// `true` with probability `exp(−num / den)` (CKS Algorithm 1).
fn bernoulli_exp(num: u128, den: u128, rng: &mut dyn RandomSource) -> bool {
    if num <= den {
        let mut k: u128 = 1;
        loop {
            let Some(scaled) = den.checked_mul(k) else {
                // exp(−γ)'s series is past any representable term: stop here.
                return k % 2 == 1;
            };
            if !bernoulli(num, scaled, rng) {
                return k % 2 == 1;
            }
            k += 1;
        }
    }
    for _ in 0..num / den {
        if !bernoulli_exp(1, 1, rng) {
            return false;
        }
    }
    bernoulli_exp(num % den, den, rng)
}

/// A discrete Laplace sample with integer scale `t ≥ 1` (CKS Algorithm 2).
fn discrete_laplace(t: u128, rng: &mut dyn RandomSource) -> i128 {
    loop {
        let u = uniform_below(t, rng);
        if !bernoulli_exp(u, t, rng) {
            continue;
        }
        let mut v: u128 = 0;
        while bernoulli_exp(1, 1, rng) {
            v += 1;
        }
        let x = i128::try_from(u + t * v).unwrap_or(i128::MAX);
        let negative = bernoulli(1, 2, rng);
        if negative && x == 0 {
            continue;
        }
        return if negative { -x } else { x };
    }
}

fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = u128::midpoint(x, n / x);
    }
    x
}

/// An exact sample from the discrete Gaussian `N_ℤ(0, σ²)` with integer variance
/// `sigma_sq ≥ 1` (CKS Algorithm 3).
pub fn discrete_gaussian(sigma_sq: u64, rng: &mut dyn RandomSource) -> i64 {
    let s2 = u128::from(sigma_sq.max(1));
    let t = isqrt(s2) + 1;
    loop {
        let y = discrete_laplace(t, rng);
        // Accept with probability exp(−(|y| − σ²/t)² / 2σ²)
        //   = exp(−(|y|·t − σ²)² / (2·σ²·t²)).
        let diff = (y.unsigned_abs() * t).abs_diff(s2);
        let (Some(num), Some(den)) = (diff.checked_mul(diff), (2 * s2).checked_mul(t * t)) else {
            continue;
        };
        if bernoulli_exp(num, den, rng) {
            return i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX });
        }
    }
}

// ---------------------------------------------------------------------------
// Clipping and quantisation
// ---------------------------------------------------------------------------

/// Scales `update` down so its L2 norm is at most `bound`. Returns the norm before.
pub fn clip_l2(update: &mut [f32], bound: f32) -> f32 {
    let norm = update.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > bound && norm > 0.0 {
        let factor = bound / norm;
        for x in update.iter_mut() {
            *x *= factor;
        }
    }
    norm
}

/// How a clipped float update becomes integers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantiser {
    /// The clipping bound `C` (L2).
    pub clip: f32,
    /// Integer units per 1.0.
    pub scale: f32,
}

impl Quantiser {
    /// Clips, then rounds `update · scale` to the nearest integers.
    pub fn quantise(&self, update: &[f32]) -> Vec<i64> {
        let mut clipped = update.to_vec();
        clip_l2(&mut clipped, self.clip);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "clipped values times a bounded scale fit easily in i64"
        )]
        clipped
            .iter()
            .map(|x| f64::from(x * self.scale).round() as i64)
            .collect()
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "integer sums back to f32 weights"
    )]
    pub fn dequantise(&self, sum: &[i64], participants: usize) -> Vec<f32> {
        let divisor = self.scale * participants.max(1) as f32;
        sum.iter().map(|x| *x as f32 / divisor).collect()
    }

    /// The L2 sensitivity of one computer's quantised update: `C·scale` plus at
    /// most ½ per coordinate from rounding.
    #[expect(clippy::cast_precision_loss, reason = "dimensions are far below 2^52")]
    pub fn sensitivity(&self, dimension: usize) -> f64 {
        f64::from(self.clip * self.scale) + (dimension as f64).sqrt() / 2.0
    }
}

// ---------------------------------------------------------------------------
// Accounting (zero-concentrated DP)
// ---------------------------------------------------------------------------

/// The zCDP cost `ρ` of one round in which `shares` computers each add
/// `N_ℤ(0, σ²)` noise to every coordinate of a `dimension`-long sum, for an L2
/// sensitivity `delta` (DDG Theorem 1, first bound).
#[expect(
    clippy::cast_precision_loss,
    reason = "share counts and dimensions are small"
)]
pub fn rho_per_round(delta: f64, sigma_sq: u64, shares: usize, dimension: usize) -> f64 {
    if shares == 0 {
        return f64::INFINITY;
    }
    let s2 = sigma_sq.max(1) as f64;
    let n = shares as f64;
    let tau: f64 = (1..shares)
        .map(|k| {
            let k = k as f64;
            10.0 * (-2.0 * std::f64::consts::PI.powi(2) * s2 * k / (k + 1.0)).exp()
        })
        .sum();
    let eps_sq = delta * delta / (n * s2) + 0.5 * tau * dimension as f64;
    eps_sq / 2.0
}

/// The (ε, δ)-DP guarantee implied by total zCDP `rho` (Bun & Steinke 2016,
/// Proposition 1.3).
pub fn epsilon_for(rho: f64, delta: f64) -> f64 {
    rho + 2.0 * (rho * (1.0 / delta).ln()).sqrt()
}

/// The total zCDP budget that yields `epsilon` at `delta`: the inverse of
/// [`epsilon_for`].
pub fn rho_for(epsilon: f64, delta: f64) -> f64 {
    let l = (1.0 / delta).ln();
    let root = (l + epsilon).sqrt() - l.sqrt();
    root * root
}

/// The per-share integer variance so a round costs at most `rho` with at least
/// `min_shares` honest shares.
pub fn calibrate_sigma_sq(rho: f64, sensitivity: f64, min_shares: usize, dimension: usize) -> u64 {
    let delta = sensitivity;
    #[expect(clippy::cast_precision_loss, reason = "share counts are small")]
    let n = min_shares.max(1) as f64;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive and capped to u64"
    )]
    let mut s2 = (delta * delta / (2.0 * rho * n)).ceil().clamp(1.0, 1e18) as u64;
    while rho_per_round(delta, s2, min_shares, dimension) > rho && s2 < u64::MAX / 2 {
        s2 += s2 / 100 + 1;
    }
    s2
}

/// A privacy budget that every round is charged against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrivacyBudget {
    pub limit_rho: f64,
    pub spent_rho: f64,
}

/// The round would cost more privacy than is left.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("this round would use more privacy than is left ({wanted:.4} wanted, {left:.4} left)")]
pub struct BudgetExhausted {
    pub wanted: f64,
    pub left: f64,
}

impl PrivacyBudget {
    /// The budget for a total (ε, δ) guarantee.
    pub fn for_epsilon(epsilon: f64, delta: f64) -> Self {
        Self {
            limit_rho: rho_for(epsilon, delta),
            spent_rho: 0.0,
        }
    }

    pub fn left(&self) -> f64 {
        (self.limit_rho - self.spent_rho).max(0.0)
    }

    pub fn charge(&mut self, rho: f64) -> Result<(), BudgetExhausted> {
        if rho > self.left() + 1e-12 {
            return Err(BudgetExhausted {
                wanted: rho,
                left: self.left(),
            });
        }
        self.spent_rho += rho;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::testing::SeededRandom;

    #[test]
    fn discrete_gaussian_has_the_requested_mean_and_variance() {
        let mut rng = SeededRandom(42);
        let sigma_sq = 400u64; // σ = 20
        let samples: Vec<f64> = (0..20_000)
            .map(|_| {
                f64::from(i32::try_from(discrete_gaussian(sigma_sq, &mut rng)).expect("small"))
            })
            .collect();
        let n = f64::from(u32::try_from(samples.len()).expect("small"));
        let mean = samples.iter().sum::<f64>() / n;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        assert!(mean.abs() < 0.6, "mean {mean}");
        assert!((var - 400.0).abs() < 400.0 * 0.05, "variance {var}");
    }

    #[test]
    fn small_variances_still_sample() {
        let mut rng = SeededRandom(1);
        for _ in 0..1000 {
            assert!(discrete_gaussian(1, &mut rng).abs() < 20);
        }
    }

    #[test]
    fn clipping_bounds_any_single_computer() {
        let mut big = vec![30.0, 40.0];
        assert!((clip_l2(&mut big, 1.0) - 50.0).abs() < 1e-4);
        assert!((big[0] - 0.6).abs() < 1e-6 && (big[1] - 0.8).abs() < 1e-6);
        let q = Quantiser {
            clip: 1.0,
            scale: 1000.0,
        };
        assert_eq!(q.quantise(&[30.0, 40.0]), [600, 800]);
        assert_eq!(q.dequantise(&[1200, 1600], 2), [0.6, 0.8]);
    }

    #[test]
    fn epsilon_and_rho_are_inverse() {
        for (eps, delta) in [(1.0, 1e-6), (4.0, 1e-5), (8.0, 1e-7)] {
            let rho = rho_for(eps, delta);
            assert!((epsilon_for(rho, delta) - eps).abs() < 1e-9);
        }
    }

    #[test]
    fn calibration_meets_the_round_budget_and_the_budget_refuses_overspend() {
        let delta = 1e-6;
        let mut budget = PrivacyBudget::for_epsilon(4.0, delta);
        let rounds = 10;
        let per_round = budget.limit_rho / f64::from(rounds);
        let q = Quantiser {
            clip: 1.0,
            scale: 1024.0,
        };
        let sens = q.sensitivity(64);
        let s2 = calibrate_sigma_sq(per_round, sens, 3, 64);
        let cost_true = rho_per_round(sens, s2, 3, 64);
        assert!(cost_true <= per_round, "{cost_true} > {per_round}");
        // Not wildly over-noised either.
        assert!(cost_true > per_round * 0.9, "{cost_true} << {per_round}");
        for _ in 0..rounds {
            budget.charge(cost_true).expect("within budget");
        }
        assert!(budget.charge(cost_true).is_err());
        assert!(epsilon_for(budget.spent_rho, delta) <= 4.0 + 1e-9);
        // Fewer honest shares means each costs more.
        assert!(rho_per_round(sens, s2, 2, 64) > cost_true);
    }
}
