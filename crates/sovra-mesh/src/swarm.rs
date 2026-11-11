//! The device-key side of the sovereign swarm: Ed25519 signatures for audit-ledger
//! entries and checkpoints, X25519 agreement for secure-aggregation masks, and the
//! operating system's secure random source for privacy noise.
//!
//! Core decides everything (`sovra_core::swarm`); this module only holds the key
//! and does the cryptography, like [`crate::cipher`] does for lanes.

use iroh::SecretKey;
use sha2::{Digest as _, Sha256};
use sovra_core::group::{DeviceId, Signature};
use sovra_core::swarm::learn::RandomSource;
use sovra_core::swarm::learn::secagg::PairSeeds;

use crate::cipher::MeshCipher;

/// This computer's swarm keys, from its device secret. Works without a network.
pub struct SwarmKeys {
    secret: SecretKey,
    cipher: MeshCipher,
}

impl std::fmt::Debug for SwarmKeys {
    // Never print the secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmKeys")
            .field("device", &self.me())
            .finish_non_exhaustive()
    }
}

impl SwarmKeys {
    pub fn new(device_secret: &[u8; 32]) -> Self {
        Self {
            secret: SecretKey::from_bytes(device_secret),
            cipher: MeshCipher::new(device_secret),
        }
    }

    /// Signs ledger entry or checkpoint bytes from `sovra_core::swarm::ledger`.
    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.secret.sign(message).to_bytes())
    }
}

impl PairSeeds for SwarmKeys {
    fn me(&self) -> DeviceId {
        DeviceId(*self.secret.public().as_bytes())
    }

    fn pair_seed(&self, peer: &DeviceId, round: u64) -> Option<[u8; 32]> {
        let shared = self.cipher.agree(peer)?;
        let me = self.me();
        let (low, high) = if me < *peer { (me, *peer) } else { (*peer, me) };
        let mut hasher = Sha256::new();
        hasher.update(b"sovra-secagg-pair-v1");
        hasher.update(shared);
        hasher.update(low.as_bytes());
        hasher.update(high.as_bytes());
        hasher.update(round.to_le_bytes());
        Some(hasher.finalize().into())
    }

    fn fresh_seed(&self) -> Option<[u8; 32]> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).ok().map(|()| seed)
    }
}

/// The operating system's secure random source, for privacy noise.
#[derive(Debug, Clone, Copy)]
pub struct OsRandom(());

impl OsRandom {
    /// `None` if the operating system cannot give secure randomness right now; a
    /// training round must then not run, because predictable noise protects nobody.
    pub fn new() -> Option<Self> {
        getrandom::u64().ok().map(|_| Self(()))
    }
}

impl RandomSource for OsRandom {
    fn next_u64(&mut self) -> u64 {
        getrandom::u64().expect("the OS random source worked when this was created")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use sovra_core::Timestamp;
    use sovra_core::group::ContentHash;
    use sovra_core::swarm::department::DepartmentId;
    use sovra_core::swarm::learn::secagg::{MaskingClient, Reveal, unmask_sum};
    use sovra_core::swarm::ledger::{
        AuditEvent, AuditLedger, ItemKind, LedgerError, UnsignedEntry,
    };

    use super::*;
    use crate::Ed25519Verifier;

    fn keys(n: u8) -> SwarmKeys {
        SwarmKeys::new(&[n; 32])
    }

    #[test]
    fn pairs_agree_on_a_seed_nobody_else_can_compute() {
        let (a, b, c) = (keys(1), keys(2), keys(3));
        let ab = a.pair_seed(&b.me(), 7).expect("valid key");
        assert_eq!(b.pair_seed(&a.me(), 7), Some(ab));
        assert_ne!(c.pair_seed(&b.me(), 7), Some(ab));
        assert_ne!(a.pair_seed(&b.me(), 8), Some(ab), "fresh per round");
        assert_ne!(a.fresh_seed(), a.fresh_seed());
    }

    #[test]
    fn real_device_keys_sign_the_audit_ledger() {
        let author = keys(9);
        let mut ledger = AuditLedger::new("acme/audit");
        let body = UnsignedEntry {
            origin: "acme/audit".into(),
            author: author.me(),
            seq: 1,
            prev: None,
            at: Timestamp(1),
            event: AuditEvent::Stored {
                item: ContentHash::of(b"contract.pdf"),
                kind: ItemKind::Document,
                department: DepartmentId::new("legal").expect("valid"),
                device: author.me(),
            },
        };
        let forged = body.clone().sign_with(keys(10).sign(&body.signing_bytes()));
        assert_eq!(
            ledger.append(forged, &Ed25519Verifier),
            Err(LedgerError::BadSignature)
        );
        let signature = author.sign(&body.signing_bytes());
        assert!(
            ledger
                .append(body.sign_with(signature), &Ed25519Verifier)
                .is_ok()
        );
    }

    #[test]
    fn masks_from_real_key_agreement_cancel_in_the_sum() {
        let fleet: Vec<SwarmKeys> = (20..24).map(keys).collect();
        let roster: BTreeSet<DeviceId> = fleet.iter().map(PairSeeds::me).collect();
        let mut clients: Vec<MaskingClient> = fleet
            .iter()
            .map(|k| MaskingClient::from_keys(k, 3, roster.clone(), 3).expect("all agree"))
            .collect();
        let uploads: BTreeMap<DeviceId, Vec<u64>> = clients
            .iter()
            .zip([10i64, -4, 7, 1])
            .map(|(c, v)| (c.me, c.mask(&[v, v * 2])))
            .collect();
        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<Reveal> = clients
            .iter_mut()
            .map(|c| c.reveal(&survivors).expect("one list"))
            .collect();
        assert_eq!(unmask_sum(3, &uploads, &reveals, 3), Ok(vec![14, 28]));
    }

    #[test]
    fn the_os_random_source_is_available_and_varies() {
        let mut rng = OsRandom::new().expect("this machine has an OS random source");
        assert_ne!(rng.next_u64(), rng.next_u64());
    }
}
