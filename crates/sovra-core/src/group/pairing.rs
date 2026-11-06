//! Pairing a computer into a group: invites, the short-code window, and the steps a
//! person sees. Pure bookkeeping — the mesh adapter does the networking and the PAKE,
//! then reports what happened here so every rule stays testable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::keys::GroupId;
use super::membership::Role;
use super::sas::ShortAuthString;
use crate::Timestamp;

/// A QR code or link stays usable for ten minutes, once.
pub const INVITE_LIFETIME_MS: i64 = 10 * 60 * 1000;
/// A same-room six-digit code stays open for two minutes and allows one guess.
pub const SHORT_CODE_LIFETIME_MS: i64 = 2 * 60 * 1000;

/// Who the invite is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InviteKind {
    /// Another computer of the person who made the invite.
    OwnDevice,
    /// A new person, joining with the role the inviter chose.
    Person { name: String, role: Role },
}

/// A one-time invitation, shown as a QR code or a `sovra://join/…` link. The secret
/// is 128 random bits from the adapter, so it cannot be guessed and needs no PAKE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub group: GroupId,
    pub kind: InviteKind,
    pub secret: [u8; 16],
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
}

impl Invite {
    pub fn new(group: GroupId, kind: InviteKind, secret: [u8; 16], now: Timestamp) -> Self {
        Self {
            group,
            kind,
            secret,
            issued_at: now,
            expires_at: Timestamp(now.millis() + INVITE_LIFETIME_MS),
        }
    }

    /// What a joiner sends to prove it holds this invite, bound to the connection's
    /// transcript so a captured proof is useless on any other connection.
    pub fn proof(&self, transcript: &[u8]) -> [u8; 32] {
        invite_proof(&self.secret, transcript)
    }
}

/// `SHA-256(tag ‖ secret ‖ SHA-256(transcript))`: the secret is fixed-size and the
/// transcript is pre-hashed, so length extension gains nothing.
pub fn invite_proof(secret: &[u8; 16], transcript: &[u8]) -> [u8; 32] {
    let inner: [u8; 32] = Sha256::digest(transcript).into();
    let mut hasher = Sha256::new();
    hasher.update(b"sovra-invite-proof-v1");
    hasher.update(secret);
    hasher.update(inner);
    hasher.finalize().into()
}

/// Why an invite could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RedeemError {
    #[error("this invite isn't from this computer")]
    Unknown,
    #[error("this invite has expired — make a new one")]
    Expired,
    #[error("this invite was already used — make a new one")]
    AlreadyUsed,
}

/// Invites this computer has issued, each usable once before it expires.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteBook {
    invites: BTreeMap<[u8; 32], (Invite, bool)>,
}

impl InviteBook {
    pub fn issue(&mut self, invite: Invite) {
        let key: [u8; 32] = Sha256::digest(invite.secret).into();
        self.invites.insert(key, (invite, false));
    }

    /// Finds the invite a joiner proved it holds and burns it. The proof is compared
    /// against every live invite in constant time per invite.
    pub fn redeem(
        &mut self,
        proof: &[u8; 32],
        transcript: &[u8],
        now: Timestamp,
    ) -> Result<Invite, RedeemError> {
        let (invite, used) = self
            .invites
            .values_mut()
            .find(|(invite, _)| constant_time_eq(&invite.proof(transcript), proof))
            .ok_or(RedeemError::Unknown)?;
        if *used {
            return Err(RedeemError::AlreadyUsed);
        }
        if now >= invite.expires_at {
            return Err(RedeemError::Expired);
        }
        *used = true;
        Ok(invite.clone())
    }

    /// Drops invites that can no longer be used.
    pub fn prune(&mut self, now: Timestamp) {
        self.invites
            .retain(|_, (invite, used)| !*used && now < invite.expires_at);
    }

    pub fn live(&self, now: Timestamp) -> usize {
        self.invites
            .values()
            .filter(|(invite, used)| !*used && now < invite.expires_at)
            .count()
    }
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The same-room code: open for two minutes, one guess, then burnt (the Magic
/// Wormhole rule that makes a six-digit code safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortCodeWindow {
    pub expires_at: Timestamp,
    used: bool,
}

impl ShortCodeWindow {
    pub const fn open(now: Timestamp) -> Self {
        Self {
            expires_at: Timestamp(now.millis() + SHORT_CODE_LIFETIME_MS),
            used: false,
        }
    }

    /// Spends the single guess. Returns false when the window is closed or spent.
    pub fn take_guess(&mut self, now: Timestamp) -> bool {
        if self.used || now >= self.expires_at {
            return false;
        }
        self.used = true;
        true
    }
}

/// Why pairing stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingFailure {
    Expired,
    CodeMismatch,
    Unreachable,
    Refused,
    Cancelled,
}

/// What the pairing screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingStage {
    Idle,
    ShowingInvite {
        expires_at: Timestamp,
    },
    Connecting,
    /// Both people check the code. `None` means that side has not answered yet.
    ConfirmCode {
        code: ShortAuthString,
        here: Option<bool>,
        there: Option<bool>,
    },
    Admitting,
    Joined {
        group: GroupId,
    },
    Failed {
        reason: PairingFailure,
    },
}

/// What the adapter or the person reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingStep {
    InviteShown {
        expires_at: Timestamp,
    },
    PeerConnected,
    CodeReady(ShortAuthString),
    /// The person on this computer tapped "They match" (`true`) or "They don't".
    ConfirmedHere(bool),
    ConfirmedThere(bool),
    /// A code check was not needed (QR scanned in the same room).
    SkipCode,
    Admitted {
        group: GroupId,
    },
    Failed(PairingFailure),
    Cancel,
    Tick(Timestamp),
}

impl PairingStage {
    /// Pure transition. Steps that make no sense in the current stage are ignored.
    #[must_use]
    pub fn advance(self, step: PairingStep) -> Self {
        match (self, step) {
            (_, PairingStep::Cancel) => Self::Failed {
                reason: PairingFailure::Cancelled,
            },
            (_, PairingStep::Failed(reason)) => Self::Failed { reason },
            (Self::Idle | Self::Failed { .. }, PairingStep::InviteShown { expires_at }) => {
                Self::ShowingInvite { expires_at }
            }
            (Self::ShowingInvite { expires_at }, PairingStep::Tick(now)) if now >= expires_at => {
                Self::Failed {
                    reason: PairingFailure::Expired,
                }
            }
            (Self::Idle | Self::ShowingInvite { .. }, PairingStep::PeerConnected) => {
                Self::Connecting
            }
            (Self::Connecting, PairingStep::CodeReady(code)) => Self::ConfirmCode {
                code,
                here: None,
                there: None,
            },
            (Self::Connecting, PairingStep::SkipCode) => Self::Admitting,
            (Self::ConfirmCode { code, there, .. }, PairingStep::ConfirmedHere(answer)) => {
                Self::after_answers(code, Some(answer), there)
            }
            (Self::ConfirmCode { code, here, .. }, PairingStep::ConfirmedThere(answer)) => {
                Self::after_answers(code, here, Some(answer))
            }
            (Self::Admitting, PairingStep::Admitted { group }) => Self::Joined { group },
            (stage, _) => stage,
        }
    }

    fn after_answers(code: ShortAuthString, here: Option<bool>, there: Option<bool>) -> Self {
        match (here, there) {
            (Some(false), _) | (_, Some(false)) => Self::Failed {
                reason: PairingFailure::CodeMismatch,
            },
            (Some(true), Some(true)) => Self::Admitting,
            _ => Self::ConfirmCode { code, here, there },
        }
    }
}
