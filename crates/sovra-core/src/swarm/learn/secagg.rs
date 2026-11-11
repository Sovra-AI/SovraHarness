//! Secure aggregation: whoever adds up a round's updates learns only the sum.
//!
//! The double-masking scheme of Bonawitz et al. (CCS 2017,
//! <https://eprint.iacr.org/2017/281>) over the ring ℤ/2⁶⁴:
//!
//! - Every pair of computers shares a seed `sᵢⱼ` (from an X25519 agreement in the
//!   mesh adapter). The lower id adds the seed's mask, the higher id subtracts it,
//!   so the pair masks cancel in the sum.
//! - Each computer also adds a **self mask** from its own seed `bᵢ`.
//! - After the upload deadline the aggregator names the survivors. A survivor
//!   reveals its own `bᵢ`, and its pair seeds **only for computers that dropped**,
//!   so the aggregator can remove exactly the masks that no longer cancel.
//! - A computer answers **one** survivor list per round. Revealing `bᵢ` and `sᵢⱼ`
//!   for the same peer would unmask a late upload; answering two different lists is
//!   how a dishonest aggregator would try that, so the second is refused. It also
//!   refuses a list shorter than the round's threshold.
//!
//! Masks are expanded from seeds with BLAKE3's keyed XOF: deterministic, so both
//! members of a pair derive the same mask. Values are two's-complement integers, so
//! the noised, quantised updates from [`super::privacy`] sum exactly.

use std::collections::{BTreeMap, BTreeSet};

use crate::group::{ContentHash, DeviceId};

/// A mask stream for `round` and `len` coordinates from a 32-byte seed.
pub fn expand_mask(seed: &[u8; 32], round: u64, len: usize) -> Vec<u64> {
    let mut hasher = blake3::Hasher::new_keyed(seed);
    hasher.update(b"sovra-secagg-mask-v1");
    hasher.update(&round.to_le_bytes());
    let mut reader = hasher.finalize_xof();
    let mut out = Vec::with_capacity(len);
    let mut word = [0u8; 8];
    for _ in 0..len {
        reader.fill(&mut word);
        out.push(u64::from_le_bytes(word));
    }
    out
}

fn add_into(acc: &mut [u64], mask: &[u64], sign: bool) {
    for (a, m) in acc.iter_mut().zip(mask) {
        *a = if sign {
            a.wrapping_add(*m)
        } else {
            a.wrapping_sub(*m)
        };
    }
}

/// The hash a survivor list is known by, so computers can compare lists.
pub fn survivors_hash(round: u64, survivors: &BTreeSet<DeviceId>) -> ContentHash {
    let mut bytes = b"sovra-secagg-survivors-v1".to_vec();
    bytes.extend_from_slice(&round.to_le_bytes());
    for device in survivors {
        bytes.extend_from_slice(device.as_bytes());
    }
    ContentHash::of(&bytes)
}

/// One computer's side of a round.
#[derive(Debug, Clone)]
pub struct MaskingClient {
    pub me: DeviceId,
    pub round: u64,
    /// Everyone who was asked to take part, including this computer.
    pub roster: BTreeSet<DeviceId>,
    /// The fewest survivors this computer will help unmask.
    pub threshold: usize,
    self_seed: [u8; 32],
    pair_seeds: BTreeMap<DeviceId, [u8; 32]>,
    answered: Option<ContentHash>,
}

/// What a survivor reveals so the sum can be unmasked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reveal {
    pub from: DeviceId,
    pub survivors: ContentHash,
    pub self_seed: [u8; 32],
    /// Pair seeds with the computers that dropped.
    pub dropped_pairs: BTreeMap<DeviceId, [u8; 32]>,
}

/// Why masking or unmasking was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecAggError {
    #[error("there's no shared seed with one of the other computers")]
    MissingPairSeed,
    #[error("too few computers are left to keep anyone's update private")]
    TooFewSurvivors,
    #[error("this computer is not on the survivor list")]
    NotASurvivor,
    #[error("this round already got a different survivor list")]
    Equivocation,
    #[error("an update is the wrong length")]
    WrongLength,
    #[error("a survivor's reveal is missing or doesn't match the list")]
    BadReveal,
}

/// Key agreement for masks, implemented by the adapter that holds the device key
/// (X25519 from the Ed25519 device key in `sovra-mesh`). Both computers of a pair
/// must derive the same seed for the same round, and nobody else can.
pub trait PairSeeds {
    fn me(&self) -> DeviceId;
    fn pair_seed(&self, peer: &DeviceId, round: u64) -> Option<[u8; 32]>;
    /// A fresh secret seed from the operating system's secure source.
    fn fresh_seed(&self) -> Option<[u8; 32]>;
}

impl MaskingClient {
    /// A client whose seeds come from `keys`: one agreement per other computer in
    /// `roster`, and a fresh self seed.
    pub fn from_keys(
        keys: &dyn PairSeeds,
        round: u64,
        roster: BTreeSet<DeviceId>,
        threshold: usize,
    ) -> Result<Self, SecAggError> {
        let me = keys.me();
        let pair_seeds = roster
            .iter()
            .filter(|peer| **peer != me)
            .map(|peer| {
                keys.pair_seed(peer, round)
                    .map(|seed| (*peer, seed))
                    .ok_or(SecAggError::MissingPairSeed)
            })
            .collect::<Result<_, _>>()?;
        let self_seed = keys.fresh_seed().ok_or(SecAggError::MissingPairSeed)?;
        Self::new(me, round, roster, threshold, self_seed, pair_seeds)
    }

    /// `pair_seeds` must hold one seed for every other computer in `roster`.
    pub fn new(
        me: DeviceId,
        round: u64,
        roster: BTreeSet<DeviceId>,
        threshold: usize,
        self_seed: [u8; 32],
        pair_seeds: BTreeMap<DeviceId, [u8; 32]>,
    ) -> Result<Self, SecAggError> {
        if roster
            .iter()
            .any(|d| *d != me && !pair_seeds.contains_key(d))
        {
            return Err(SecAggError::MissingPairSeed);
        }
        Ok(Self {
            me,
            round,
            roster,
            threshold: threshold.max(2),
            self_seed,
            pair_seeds,
            answered: None,
        })
    }

    /// The masked upload for `values` (already clipped, quantised and noised).
    pub fn mask(&self, values: &[i64]) -> Vec<u64> {
        #[expect(
            clippy::cast_sign_loss,
            reason = "two's-complement into the ring ℤ/2⁶⁴"
        )]
        let mut out: Vec<u64> = values.iter().map(|v| *v as u64).collect();
        let len = out.len();
        add_into(
            &mut out,
            &expand_mask(&self.self_seed, self.round, len),
            true,
        );
        for (peer, seed) in &self.pair_seeds {
            if self.roster.contains(peer) {
                let mask = expand_mask(seed, self.round, out.len());
                add_into(&mut out, &mask, self.me < *peer);
            }
        }
        out
    }

    /// Answers the aggregator's survivor list once.
    pub fn reveal(&mut self, survivors: &BTreeSet<DeviceId>) -> Result<Reveal, SecAggError> {
        let hash = survivors_hash(self.round, survivors);
        if self.answered.is_some_and(|seen| seen != hash) {
            return Err(SecAggError::Equivocation);
        }
        if !survivors.contains(&self.me) {
            return Err(SecAggError::NotASurvivor);
        }
        if survivors.len() < self.threshold || !survivors.is_subset(&self.roster) {
            return Err(SecAggError::TooFewSurvivors);
        }
        self.answered = Some(hash);
        let dropped_pairs = self
            .pair_seeds
            .iter()
            .filter(|(peer, _)| self.roster.contains(*peer) && !survivors.contains(*peer))
            .map(|(peer, seed)| (*peer, *seed))
            .collect();
        Ok(Reveal {
            from: self.me,
            survivors: hash,
            self_seed: self.self_seed,
            dropped_pairs,
        })
    }
}

/// Adds up masked uploads from `survivors` and removes the masks using their
/// reveals. Returns the plain integer sum of the survivors' values.
pub fn unmask_sum(
    round: u64,
    uploads: &BTreeMap<DeviceId, Vec<u64>>,
    reveals: &[Reveal],
    threshold: usize,
) -> Result<Vec<i64>, SecAggError> {
    let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
    if survivors.len() < threshold.max(2) {
        return Err(SecAggError::TooFewSurvivors);
    }
    let len = uploads.values().next().map_or(0, Vec::len);
    if uploads.values().any(|u| u.len() != len) {
        return Err(SecAggError::WrongLength);
    }
    let hash = survivors_hash(round, &survivors);
    let by_sender: BTreeMap<DeviceId, &Reveal> = reveals
        .iter()
        .filter(|r| r.survivors == hash)
        .map(|r| (r.from, r))
        .collect();
    if survivors.iter().any(|s| !by_sender.contains_key(s)) {
        return Err(SecAggError::BadReveal);
    }
    let mut sum = vec![0u64; len];
    for upload in uploads.values() {
        add_into(&mut sum, upload, true);
    }
    for (survivor, reveal) in &by_sender {
        if !survivors.contains(survivor) {
            continue;
        }
        add_into(&mut sum, &expand_mask(&reveal.self_seed, round, len), false);
        for (dropped, seed) in &reveal.dropped_pairs {
            if survivors.contains(dropped) {
                return Err(SecAggError::BadReveal);
            }
            // The survivor added (+) when it was lower, so remove with the opposite sign.
            let mask = expand_mask(seed, round, len);
            add_into(&mut sum, &mask, survivor > dropped);
        }
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "back from the ring to two's complement"
    )]
    Ok(sum.into_iter().map(|v| v as i64).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(n: u8) -> DeviceId {
        DeviceId([n; 32])
    }

    fn pair_seed(a: u8, b: u8) -> [u8; 32] {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        *blake3::hash(&[lo, hi]).as_bytes()
    }

    fn clients(n: u8, round: u64) -> Vec<MaskingClient> {
        let roster: BTreeSet<DeviceId> = (1..=n).map(device).collect();
        (1..=n)
            .map(|i| {
                let seeds = (1..=n)
                    .filter(|j| *j != i)
                    .map(|j| (device(j), pair_seed(i, j)))
                    .collect();
                MaskingClient::new(device(i), round, roster.clone(), 2, [i; 32], seeds)
                    .expect("every pair has a seed")
            })
            .collect()
    }

    #[test]
    fn masked_uploads_hide_values_but_sum_exactly() {
        let mut cs = clients(4, 7);
        let values = [
            vec![5, -3, 10],
            vec![1, 1, 1],
            vec![-8, 0, 2],
            vec![100, -100, 0],
        ];
        let uploads: BTreeMap<DeviceId, Vec<u64>> = cs
            .iter()
            .zip(&values)
            .map(|(c, v)| (c.me, c.mask(v)))
            .collect();
        // A single upload looks nothing like its values.
        assert_ne!(uploads[&device(1)][0], 5);
        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<Reveal> = cs
            .iter_mut()
            .map(|c| c.reveal(&survivors).expect("valid list"))
            .collect();
        assert_eq!(unmask_sum(7, &uploads, &reveals, 2), Ok(vec![98, -102, 13]));
    }

    #[test]
    fn a_computer_that_drops_out_is_recovered_without_exposing_anyone() {
        let mut cs = clients(4, 1);
        let values = [vec![1, 2], vec![3, 4], vec![5, 6], vec![7, 8]];
        let mut uploads: BTreeMap<DeviceId, Vec<u64>> = cs
            .iter()
            .zip(&values)
            .map(|(c, v)| (c.me, c.mask(v)))
            .collect();
        uploads.remove(&device(3));
        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<Reveal> = cs
            .iter_mut()
            .filter(|c| survivors.contains(&c.me))
            .map(|c| c.reveal(&survivors).expect("valid list"))
            .collect();
        assert_eq!(unmask_sum(1, &uploads, &reveals, 2), Ok(vec![11, 14]));
    }

    #[test]
    fn a_second_different_survivor_list_is_refused() {
        let mut cs = clients(3, 2);
        let all: BTreeSet<DeviceId> = (1..=3).map(device).collect();
        let fewer: BTreeSet<DeviceId> = [device(1), device(2)].into();
        cs[0].reveal(&all).expect("first answer");
        assert_eq!(cs[0].reveal(&fewer), Err(SecAggError::Equivocation));
        // The same list again is fine (a retry).
        assert!(cs[0].reveal(&all).is_ok());
        assert_eq!(
            cs[1].reveal(&[device(2)].into()),
            Err(SecAggError::TooFewSurvivors)
        );
        assert_eq!(cs[2].reveal(&fewer), Err(SecAggError::NotASurvivor));
    }

    #[test]
    fn missing_reveals_fail_instead_of_returning_garbage() {
        let mut cs = clients(3, 5);
        let uploads: BTreeMap<DeviceId, Vec<u64>> =
            cs.iter().map(|c| (c.me, c.mask(&[1]))).collect();
        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<Reveal> = cs
            .iter_mut()
            .take(2)
            .map(|c| c.reveal(&survivors).expect("valid"))
            .collect();
        assert_eq!(
            unmask_sum(5, &uploads, &reveals, 2),
            Err(SecAggError::BadReveal)
        );
    }
}
