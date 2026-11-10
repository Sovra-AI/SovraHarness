//! Lane encryption for groups: the [`GroupCipher`] port, with this computer's device
//! key.
//!
//! - **Sealing to a computer** (lane-key grants): the device's Ed25519 key, mapped to
//!   X25519 (the standard birational map; the secret is the clamped first half of
//!   SHA-512 of the seed, as in RFC 8032). An ephemeral X25519 key agrees a secret
//!   with the recipient, SHA-256 derives the AES-256-GCM key, and the output is
//!   `ephemeral public key ‖ nonce ‖ ciphertext`.
//! - **Content**: AES-256-GCM with a random 96-bit nonce, `nonce ‖ ciphertext`, bound
//!   to the associated data core chose (group, lane, key, author).
//!
//! Both primitives are the versions iroh already builds. Nothing here decides who gets
//! a key; that rule lives in `sovra_core::group::lane_keys`.

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use sha2::{Digest as _, Sha256, Sha512};
use sovra_core::group::{DeviceId, GroupCipher};

const NONCE_BYTES: usize = 12;

/// This computer's half of lane encryption.
pub(crate) struct MeshCipher {
    /// The X25519 secret scalar bytes (clamped on every use).
    secret: [u8; 32],
    public: MontgomeryPoint,
}

impl std::fmt::Debug for MeshCipher {
    // Never print the secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshCipher").finish_non_exhaustive()
    }
}

impl MeshCipher {
    pub(crate) fn new(device_seed: &[u8; 32]) -> Self {
        let digest = Sha512::digest(device_seed);
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&digest[..32]);
        Self {
            secret,
            public: MontgomeryPoint::mul_base_clamped(secret),
        }
    }
}

/// A computer's X25519 key, from its Ed25519 device key (its group address).
fn device_point(device: &DeviceId) -> Option<MontgomeryPoint> {
    CompressedEdwardsY(*device.as_bytes())
        .decompress()
        .map(|point| point.to_montgomery())
}

fn seal_key(
    shared: &MontgomeryPoint,
    ephemeral: &MontgomeryPoint,
    to: &MontgomeryPoint,
) -> Option<[u8; 32]> {
    // A low-order recipient key gives an all-zero secret; refuse it.
    if shared.as_bytes() == &[0u8; 32] {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"sovra-seal-v1");
    hasher.update(shared.as_bytes());
    hasher.update(ephemeral.as_bytes());
    hasher.update(to.as_bytes());
    Some(hasher.finalize().into())
}

fn random<const N: usize>() -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).ok().map(|()| bytes)
}

fn aes_encrypt(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = random::<NONCE_BYTES>()?;
    let mut out = nonce.to_vec();
    out.extend(
        cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .ok()?,
    );
    Some(out)
}

fn aes_decrypt(key: &[u8; 32], aad: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < NONCE_BYTES {
        return None;
    }
    let (nonce, ciphertext) = data.split_at(NONCE_BYTES);
    Aes256Gcm::new_from_slice(key)
        .ok()?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}

impl GroupCipher for MeshCipher {
    fn seal_for(&self, device: &DeviceId, plaintext: &[u8]) -> Option<Vec<u8>> {
        let to = device_point(device)?;
        let ephemeral_secret = random::<32>()?;
        let ephemeral = MontgomeryPoint::mul_base_clamped(ephemeral_secret);
        let key = seal_key(&to.mul_clamped(ephemeral_secret), &ephemeral, &to)?;
        let mut out = ephemeral.to_bytes().to_vec();
        out.extend(aes_encrypt(&key, b"sovra-seal-v1", plaintext)?);
        Some(out)
    }

    fn open_sealed(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < 32 {
            return None;
        }
        let (ephemeral, rest) = sealed.split_at(32);
        let ephemeral = MontgomeryPoint(ephemeral.try_into().ok()?);
        let key = seal_key(
            &ephemeral.mul_clamped(self.secret),
            &ephemeral,
            &self.public,
        )?;
        aes_decrypt(&key, b"sovra-seal-v1", rest)
    }

    fn encrypt(&self, key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
        aes_encrypt(key, aad, plaintext)
    }

    fn decrypt(&self, key: &[u8; 32], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
        aes_decrypt(key, aad, ciphertext)
    }

    fn random_key(&self) -> Option<[u8; 32]> {
        random::<32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mesh;

    #[test]
    fn only_the_computer_a_key_was_sealed_for_can_open_it() {
        let (mum_seed, dad_seed) = ([41; 32], [42; 32]);
        let mum = MeshCipher::new(&mum_seed);
        let dad = MeshCipher::new(&dad_seed);
        let dads_address = Mesh::device_for(&dad_seed);

        let sealed = mum.seal_for(&dads_address, b"lane key").expect("seals");
        assert_eq!(
            dad.open_sealed(&sealed).as_deref(),
            Some(b"lane key".as_slice())
        );
        assert_eq!(mum.open_sealed(&sealed), None, "not even its sender");
        let mut tampered = sealed.clone();
        *tampered.last_mut().expect("not empty") ^= 1;
        assert_eq!(dad.open_sealed(&tampered), None);
        assert_ne!(
            mum.seal_for(&dads_address, b"lane key"),
            Some(sealed),
            "every seal uses a fresh ephemeral key"
        );
    }

    #[test]
    fn content_opens_only_with_its_key_and_its_place() {
        let cipher = MeshCipher::new(&[7; 32]);
        let key = cipher.random_key().expect("random");
        let other = cipher.random_key().expect("random");
        let sealed = cipher
            .encrypt(&key, b"group/lane", b"boiler on 12 March")
            .expect("encrypts");
        assert!(
            !sealed.windows(5).any(|w| w == b"March"),
            "no plaintext on disk"
        );
        assert_eq!(
            cipher.decrypt(&key, b"group/lane", &sealed).as_deref(),
            Some(b"boiler on 12 March".as_slice())
        );
        assert_eq!(cipher.decrypt(&other, b"group/lane", &sealed), None);
        assert_eq!(cipher.decrypt(&key, b"other/lane", &sealed), None);
        assert_eq!(cipher.decrypt(&key, b"group/lane", &sealed[..5]), None);
    }
}
