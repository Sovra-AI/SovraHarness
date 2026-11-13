//! Collective training without exporting data.
//!
//! The private round protocol below requires consent and masking; the separate
//! [`scavenger`] scheduler/optimizer primitive does not provide those protections.
//!
//! Each computer trains on its own department's approved examples; only a clipped,
//! noised, masked update leaves it, and only after a person signs off. The fleet
//! sees the sum, never one computer's contribution.
//!
//! - [`galore`]: low-rank gradient projection so large models fit consumer GPUs,
//!   and shared projectors for compact federated updates (`FedLore`).
//! - [`scaffold`]: control variates against non-IID drift between departments.
//! - [`privacy`]: exact discrete-Gaussian noise and a zCDP budget.
//! - [`secagg`]: pairwise-masked secure sums with dropout recovery.
//! - [`round`]: one round end to end, producing ledger events.
//!
//! Randomness that protects privacy must come from the operating system's secure
//! source: the runtime passes a [`RandomSource`] backed by it. Tests use a seed.

pub mod galore;
pub mod linalg;
pub mod privacy;
pub mod round;
pub mod scaffold;
pub mod scavenger;
pub mod secagg;

/// 64 random bits at a time. Production implementations must be cryptographically
/// secure (the mesh adapter's OS random source); noise from a predictable source
/// gives no privacy.
pub trait RandomSource {
    fn next_u64(&mut self) -> u64;
}

/// A uniform float in `[0, 1)` with 53 random bits.
#[expect(
    clippy::cast_precision_loss,
    reason = "53 bits fit an f64 mantissa exactly"
)]
pub(crate) fn uniform(rng: &mut dyn RandomSource) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// A standard normal sample (Box–Muller). Used for random projections and test
/// data, never for privacy noise, which is exact and integer ([`privacy`]).
pub(crate) fn standard_normal(rng: &mut dyn RandomSource) -> f64 {
    let u1 = uniform(rng).max(f64::MIN_POSITIVE);
    let u2 = uniform(rng);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}
