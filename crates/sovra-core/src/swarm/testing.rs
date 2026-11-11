//! Test doubles: a keyed-hash "signature" so tests need no Ed25519.

use crate::group::{DeviceId, Signature, SignatureVerifier};

/// Signs as `device` by hashing the device id with the message. Not secure; tests only.
pub(crate) fn fake_sign(device: &DeviceId, message: &[u8]) -> Signature {
    let mut hasher = blake3::Hasher::new_keyed(device.as_bytes());
    hasher.update(message);
    let mut out = [0u8; 64];
    hasher.finalize_xof().fill(&mut out);
    Signature(out)
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FakeVerifier;

impl SignatureVerifier for FakeVerifier {
    fn verify(&self, signer: &DeviceId, message: &[u8], signature: &Signature) -> bool {
        fake_sign(signer, message) == *signature
    }
}

/// Deterministic random words for tests (`SplitMix64`).
#[derive(Debug, Clone)]
pub(crate) struct SeededRandom(pub u64);

impl crate::swarm::learn::RandomSource for SeededRandom {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}
