//! Lane encryption: shared content is sealed with a lane key, and lane keys are
//! handed to member computers one by one.
//!
//! - A **lane key** is 32 random bytes with a random id. A `KeyGrant` operation seals
//!   it to each member computer's device key (only that computer can open its copy).
//! - Content operations are `Sealed`: the inner operation, encrypted with a lane key
//!   and bound (as associated data) to the group, lane, key and author.
//! - A key may encrypt **new** content only while no computer that was ever given it
//!   has since been removed. So removing someone retires every key they held, and the
//!   next share makes a fresh key for the computers still in the group, which is the
//!   rotation. History stays readable to current members and to newcomers, who are
//!   granted the older keys when they join; removed computers keep only what they
//!   already had, as the Groups screen says.
//!
//! Core decides; it never touches a crypto library. The [`GroupCipher`] port is
//! implemented by the mesh adapter, which owns the device key.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::fold::GroupState;
use super::keys::{DeviceId, GroupId, OpHash};
use super::knowledge::LaneId;
use crate::Timestamp;

/// Sealing and opening, done by whoever holds this computer's device key.
pub trait GroupCipher: Send + Sync {
    /// Seals `plaintext` so only `device` can open it. `None` if the key is unusable.
    fn seal_for(&self, device: &DeviceId, plaintext: &[u8]) -> Option<Vec<u8>>;
    /// Opens something sealed for this computer.
    fn open_sealed(&self, sealed: &[u8]) -> Option<Vec<u8>>;
    /// Authenticated encryption with a lane key; the output carries its own nonce.
    fn encrypt(&self, key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>>;
    fn decrypt(&self, key: &[u8; 32], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>>;
    /// 32 bytes from the operating system's secure random source.
    fn random_key(&self) -> Option<[u8; 32]>;
}

/// The associated data a sealed operation is bound to, so ciphertext cannot be moved
/// to another group, lane, key or author.
pub fn sealed_aad(group: &GroupId, lane: &LaneId, key: &OpHash, author: &DeviceId) -> Vec<u8> {
    let mut aad = b"sovra-sealed-v1".to_vec();
    aad.extend_from_slice(group.as_bytes());
    aad.extend_from_slice(lane.0.as_bytes());
    aad.extend_from_slice(key.as_bytes());
    aad.extend_from_slice(author.as_bytes());
    aad
}

/// One lane key as the fold sees it: who was given it, and since when it exists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaneKey {
    /// Each computer's sealed copy.
    pub grants: BTreeMap<DeviceId, Bytes>,
    /// When it was first granted (its creation), for choosing the newest usable key.
    pub created_at: Option<Timestamp>,
}

impl LaneKey {
    /// Whether it may encrypt new content: every computer that holds it is still in
    /// the space's `audience` (nobody removed, nobody outside a two-person space).
    pub fn usable(&self, audience: &BTreeSet<DeviceId>) -> bool {
        self.grants.keys().all(|device| audience.contains(device))
    }

    /// Computers in `audience` that still need a copy.
    pub fn missing(&self, audience: &BTreeSet<DeviceId>) -> BTreeSet<DeviceId> {
        audience
            .iter()
            .copied()
            .filter(|device| !self.grants.contains_key(device))
            .collect()
    }
}

/// Every computer in the roster now.
pub fn current_devices(roster: &GroupState) -> BTreeSet<DeviceId> {
    roster
        .people
        .values()
        .flat_map(|member| member.devices.keys().copied())
        .collect()
}

/// The newest key that may encrypt new content for `audience`, if any.
pub fn usable_key(
    keys: &BTreeMap<OpHash, LaneKey>,
    audience: &BTreeSet<DeviceId>,
) -> Option<OpHash> {
    keys.iter()
        .filter(|(_, key)| key.usable(audience))
        .max_by_key(|(id, key)| (key.created_at, **id))
        .map(|(id, _)| *id)
}

/// Bytes carried as lowercase hex in JSON: half the size of a number array, and the
/// same on every computer so hashes and signatures agree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bytes(pub Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = Vec::with_capacity(self.0.len() * 2);
        for byte in &self.0 {
            text.push(HEX[usize::from(byte >> 4)]);
            text.push(HEX[usize::from(byte & 0x0f)]);
        }
        serializer.serialize_str(std::str::from_utf8(&text).expect("hex digits are ASCII"))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() % 2 != 0 {
            return Err(serde::de::Error::custom("hex has an odd length"));
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        text.as_bytes()
            .chunks(2)
            .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
            .collect::<Option<Vec<u8>>>()
            .map(Bytes)
            .ok_or_else(|| serde::de::Error::custom("not lowercase hex"))
    }
}
