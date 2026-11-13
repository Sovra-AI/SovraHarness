//! Phone pairing and access decisions. IO, clocks, randomness and persistence belong
//! to adapters; group membership and Development mode never grant remote access.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::group::DeviceId;

mod session;
mod state;
pub use state::{RemoteCommand, RemoteEvent, RemoteFailure, RemoteState};
mod transport;
mod wire;
pub use session::{MAX_SESSION_REQUESTS, SessionGuard};
pub use transport::{
    IncomingRequest, RemoteConnection, RemoteEndpoint, RemoteHandshake, RemoteReply,
};
pub use wire::{
    ErrorCode, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, PairingTicket, RemoteOperation,
    RemoteRequest, RemoteResponse, WireError,
};

pub const REMOTE_ALPN: &[u8] = b"sovra/remote-control/1";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PHONES: usize = 8;
pub const MAX_PHONE_NAME_BYTES: usize = 80;
pub const PAIRING_LIFETIME_MS: u64 = 300_000;

/// Supplied from OS randomness. Never log or persist the invitation credential.
#[derive(Clone, PartialEq, Eq)]
pub struct InvitationSecret(pub [u8; 32]);

impl Serialize for InvitationSecret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        DeviceId(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InvitationSecret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        DeviceId::deserialize(deserializer).map(|id| Self(id.0))
    }
}

impl std::fmt::Debug for InvitationSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InvitationSecret([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedPhone {
    pub device: DeviceId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    #[error("Start remote control on your computer.")]
    Disabled,
    #[error("Update the phone app and Sovra to compatible versions.")]
    UnsupportedVersion,
    #[error("Pair this phone with your computer first.")]
    NotPaired,
    #[error("Reconnect to this computer before trying again.")]
    StaleGeneration,
    #[error("This invitation is unavailable or expired. Create a new one on your computer.")]
    InvalidInvitation,
    #[error("Forget a phone before pairing another one.")]
    PhoneLimit,
    #[error("Use a phone name of 1 to 80 bytes without control characters.")]
    InvalidName,
    #[error("The saved phone list contains duplicate identities.")]
    DuplicatePhone,
}

#[derive(Clone)]
struct Invitation {
    digest: [u8; 32],
    expires_at_ms: u64,
}

/// Runtime clones this policy before pairing, saves the candidate phone list, then
/// publishes the candidate only after durable success, under one serialized owner.
/// Persist `phones()` only: activation and invitations must not survive restart.
#[derive(Clone, Default)]
pub struct RemoteAccess {
    phones: Vec<PairedPhone>,
    generation: Option<[u8; 32]>,
    invitation: Option<Invitation>,
}

impl std::fmt::Debug for RemoteAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteAccess")
            .field("phone_count", &self.phones.len())
            .field("enabled", &self.generation.is_some())
            .field("pairing_open", &self.invitation.is_some())
            .finish()
    }
}

impl RemoteAccess {
    /// Corrupt storage fails closed. Restored pairs do not activate the listener.
    pub fn restore(phones: Vec<PairedPhone>) -> Result<Self, AccessError> {
        if phones.len() > MAX_PHONES {
            return Err(AccessError::PhoneLimit);
        }
        let mut seen = std::collections::BTreeSet::new();
        for phone in &phones {
            validate_name(&phone.name)?;
            if !seen.insert(phone.device) {
                return Err(AccessError::DuplicatePhone);
            }
        }
        Ok(Self {
            phones,
            ..Self::default()
        })
    }

    pub fn phones(&self) -> &[PairedPhone] {
        &self.phones
    }

    pub fn generation(&self) -> Option<[u8; 32]> {
        self.generation
    }

    /// A fresh unpredictable generation is supplied on each activation. Repeating
    /// the skill while active preserves live requests and the pairing invitation.
    pub fn enable(&mut self, generation: [u8; 32]) {
        if self.generation.is_none() {
            self.generation = Some(generation);
        }
    }

    pub fn disable(&mut self) {
        self.generation = None;
        self.invitation = None;
    }

    /// `now_ms` is elapsed monotonic time from the runtime's fixed process origin.
    /// Only an explicit local setup action may create or replace an invitation.
    pub fn invite(&mut self, secret: &InvitationSecret, now_ms: u64) -> Result<u64, AccessError> {
        if self.generation.is_none() {
            return Err(AccessError::Disabled);
        }
        if self.phones.len() >= MAX_PHONES {
            return Err(AccessError::PhoneLimit);
        }
        let expires_at_ms = now_ms
            .checked_add(PAIRING_LIFETIME_MS)
            .ok_or(AccessError::InvalidInvitation)?;
        self.invitation = Some(Invitation {
            digest: Sha256::digest(secret.0).into(),
            expires_at_ms,
        });
        Ok(expires_at_ms)
    }

    /// `peer` must come from a completed authenticated handshake, never JSON.
    /// Call on a candidate clone; persist its phone list before publishing it.
    pub fn pair(
        &mut self,
        version: u16,
        peer: DeviceId,
        name: &str,
        secret: &InvitationSecret,
        now_ms: u64,
    ) -> Result<(), AccessError> {
        self.check_enabled(version)?;
        validate_name(name)?;
        let invitation = self
            .invitation
            .as_ref()
            .ok_or(AccessError::InvalidInvitation)?;
        let digest: [u8; 32] = Sha256::digest(secret.0).into();
        if now_ms >= invitation.expires_at_ms || digest != invitation.digest {
            return Err(AccessError::InvalidInvitation);
        }
        if self.phones.iter().any(|phone| phone.device == peer) {
            // Already paired (it forgot this computer locally and scanned again): close
            // the code so existing control is not paused until it expires.
            self.invitation = None;
            return Ok(());
        }
        if self.phones.len() >= MAX_PHONES {
            return Err(AccessError::PhoneLimit);
        }
        self.phones.push(PairedPhone {
            device: peer,
            name: name.to_owned(),
        });
        self.invitation = None;
        Ok(())
    }

    /// Recheck at dispatch and reply time, not only when accepting a connection.
    pub fn authorize(
        &self,
        version: u16,
        peer: DeviceId,
        generation: &[u8; 32],
    ) -> Result<(), AccessError> {
        self.check_enabled(version)?;
        if self.generation.as_ref() != Some(generation) {
            return Err(AccessError::StaleGeneration);
        }
        if !self.phones.iter().any(|phone| phone.device == peer) {
            return Err(AccessError::NotPaired);
        }
        Ok(())
    }

    /// The runtime also closes this phone's connections and cancels pending input.
    /// If durable revocation fails, disable access until storage is repaired.
    pub fn forget(&mut self, peer: DeviceId) {
        self.phones.retain(|phone| phone.device != peer);
    }

    fn check_enabled(&self, version: u16) -> Result<(), AccessError> {
        if version != PROTOCOL_VERSION {
            return Err(AccessError::UnsupportedVersion);
        }
        if self.generation.is_none() {
            return Err(AccessError::Disabled);
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), AccessError> {
    if name.trim().is_empty()
        || name.len() > MAX_PHONE_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        Err(AccessError::InvalidName)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
