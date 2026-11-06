//! Identities and hashes for groups: plain bytes, no crypto library.
//!
//! Core never signs or verifies. Adapters (the mesh crate) own Ed25519; core only
//! carries the bytes and asks a [`SignatureVerifier`] whether a signature holds, so
//! every membership rule stays a pure, testable function.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

/// Why a hex identity could not be read back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyParseError {
    #[error("expected {expected} hex characters, found {found}")]
    Length { expected: usize, found: usize },
    #[error("not a hex character")]
    NotHex,
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn nibble(c: u8) -> Result<u8, KeyParseError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(KeyParseError::NotHex),
    }
}

fn from_hex<const N: usize>(text: &str) -> Result<[u8; N], KeyParseError> {
    let raw = text.as_bytes();
    if raw.len() != N * 2 {
        return Err(KeyParseError::Length {
            expected: N * 2,
            found: raw.len(),
        });
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(raw.as_chunks::<2>().0) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(out)
}

/// Fixed-size byte identities serialise as lowercase hex so stored JSON stays readable.
macro_rules! hex_bytes {
    ($(#[$doc:meta])* $name:ident, $len:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; $len]);

        impl $name {
            pub const fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }

            pub fn to_hex(&self) -> String {
                to_hex(&self.0)
            }

            pub fn from_hex(text: &str) -> Result<Self, KeyParseError> {
                from_hex::<$len>(text).map(Self)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.to_hex())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::from_hex(&text).map_err(D::Error::custom)
            }
        }
    };
}

hex_bytes!(
    /// One install of Sovra: an Ed25519 public key. The mesh transport dials this
    /// same key, so a computer's network identity and signing identity are one thing.
    DeviceId,
    32
);

hex_bytes!(
    /// A human. The key is the root of that person's own devices and is recoverable
    /// from their recovery code; devices act for the person, the person is not a device.
    PersonId,
    32
);

hex_bytes!(
    /// A group of people and their computers, bound to the device that founded it
    /// (see [`GroupId::derive`]) so nobody else can mint a rival genesis for it.
    GroupId,
    32
);

hex_bytes!(
    /// SHA-256 of an operation's signed content. Operations reference each other by
    /// this hash, which makes re-delivery idempotent and ordering verifiable.
    OpHash,
    32
);

hex_bytes!(
    /// A detached Ed25519 signature over [`super::MembershipOp::signing_bytes`].
    Signature,
    64
);

impl DeviceId {
    /// Short, human-checkable fingerprint for the roster ("3f2a · 9c01").
    pub fn fingerprint(&self) -> String {
        let hex = self.to_hex();
        format!("{} · {}", &hex[..4], &hex[4..8])
    }
}

impl GroupId {
    /// The id of a group founded by `founder` with a random `nonce`. Genesis must
    /// match it, so only the founding device can start the group's history.
    pub fn derive(founder: &DeviceId, nonce: &[u8; 16]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"sovra-group-v1");
        hasher.update(founder.as_bytes());
        hasher.update(nonce);
        Self(hasher.finalize().into())
    }
}

/// Checks a detached signature. Implemented by the adapter that owns Ed25519; tests
/// use a fake. Core asks, it never computes.
pub trait SignatureVerifier {
    fn verify(&self, signer: &DeviceId, message: &[u8], signature: &Signature) -> bool;
}
