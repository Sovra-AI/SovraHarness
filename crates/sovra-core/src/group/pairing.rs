//! Pairing a computer into a group: invites, the short-code window, and the steps a
//! person sees. Pure bookkeeping — the mesh adapter finds nearby computers and carries
//! the messages, and reports what happened here so every rule stays testable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::keys::{DeviceId, GroupId};
use super::membership::Role;
use super::sas::ShortAuthString;
use crate::Timestamp;

/// A QR code or link stays usable for seven days, once. Long enough to send to
/// someone far away who still has to install Sovra and find a quiet moment; the
/// person who made it can cancel it sooner (see [`InviteBook::cancel`]).
pub const INVITE_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;
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

/// Names one invite without giving away its secret (the secret's hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InviteId(pub [u8; 32]);

impl InviteId {
    pub fn of(invite: &Invite) -> Self {
        Self(Sha256::digest(invite.secret).into())
    }
}

/// An invite as it is kept between restarts, so a link sent yesterday still works
/// after the inviting computer was switched off overnight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedInvite {
    pub invite: Invite,
    pub used: bool,
}

/// An invite nobody has used yet, as the Invite page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInvite {
    pub id: InviteId,
    pub kind: InviteKind,
    pub expires_at: Timestamp,
}

impl PendingInvite {
    /// Who the invite is waiting for, in plain words.
    pub fn who(&self) -> String {
        match &self.kind {
            InviteKind::OwnDevice => "Another of your computers".to_owned(),
            InviteKind::Person { name, role } => {
                let name = if name.trim().is_empty() {
                    "Someone new"
                } else {
                    name.trim()
                };
                if role.manages() {
                    format!("{name} (as an admin)")
                } else {
                    name.to_owned()
                }
            }
        }
    }
}

/// "6 days", "about 3 hours", "about 5 minutes": how long something still works.
pub fn time_left_label(expires_at: Timestamp, now: Timestamp) -> String {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let left = expires_at.millis().saturating_sub(now.millis()).max(0);
    let plural = |count: i64, unit: &str| {
        if count == 1 {
            format!("{count} {unit}")
        } else {
            format!("{count} {unit}s")
        }
    };
    if left >= 2 * DAY {
        plural((left + DAY / 2) / DAY, "day")
    } else if left >= 2 * HOUR {
        format!("about {}", plural((left + HOUR / 2) / HOUR, "hour"))
    } else {
        format!(
            "about {}",
            plural(((left + MINUTE - 1) / MINUTE).max(1), "minute")
        )
    }
}

/// Invites this computer has issued, each usable once before it expires.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteBook {
    invites: BTreeMap<[u8; 32], (Invite, bool)>,
}

impl InviteBook {
    pub fn issue(&mut self, invite: Invite) {
        self.invites
            .insert(InviteId::of(&invite).0, (invite, false));
    }

    /// The book as it was saved, without anything that has expired since.
    pub fn restore(saved: Vec<SavedInvite>, now: Timestamp) -> Self {
        let mut book = Self::default();
        for SavedInvite { invite, used } in saved {
            if now < invite.expires_at {
                book.invites.insert(InviteId::of(&invite).0, (invite, used));
            }
        }
        book
    }

    /// What to keep between restarts: every invite that has not expired. Used ones
    /// stay until then so a second try is told "already used", not "unknown".
    pub fn saved(&self, now: Timestamp) -> Vec<SavedInvite> {
        self.invites
            .values()
            .filter(|(invite, _)| now < invite.expires_at)
            .map(|(invite, used)| SavedInvite {
                invite: invite.clone(),
                used: *used,
            })
            .collect()
    }

    /// Invites still waiting for someone, the one that runs out first at the top.
    pub fn pending(&self, now: Timestamp) -> Vec<PendingInvite> {
        let mut pending: Vec<PendingInvite> = self
            .invites
            .iter()
            .filter(|(_, (invite, used))| !*used && now < invite.expires_at)
            .map(|(id, (invite, _))| PendingInvite {
                id: InviteId(*id),
                kind: invite.kind.clone(),
                expires_at: invite.expires_at,
            })
            .collect();
        pending.sort_by_key(|invite| (invite.expires_at, invite.id));
        pending
    }

    /// A waiting invite, to show its link again.
    pub fn waiting(&self, id: &InviteId, now: Timestamp) -> Option<&Invite> {
        self.invites
            .get(&id.0)
            .filter(|(invite, used)| !*used && now < invite.expires_at)
            .map(|(invite, _)| invite)
    }

    /// Cancels an invite so its link stops working at once. Returns whether there
    /// was one to cancel.
    pub fn cancel(&mut self, id: &InviteId) -> bool {
        self.invites.remove(&id.0).is_some()
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
            // A new attempt starts from wherever the last one ended.
            (
                Self::Idle | Self::ShowingInvite { .. } | Self::Failed { .. } | Self::Joined { .. },
                PairingStep::PeerConnected,
            ) => Self::Connecting,
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

/// A typed same-room code, as six digits (spaces and dashes ignored).
pub fn normalise_code(input: &str) -> Option<String> {
    let digits: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

/// Six digits from 32 random bits, shown as "123 456".
pub fn format_code(random: u32) -> (String, String) {
    let number = random % 1_000_000;
    let digits = format!("{number:06}");
    let shown = format!("{} {}", &digits[..3], &digits[3..]);
    (digits, shown)
}

/// The invite secret both computers derive from the same-room code.
///
/// Six digits are guessable offline by whoever receives a proof, so the code alone
/// never admits anyone: the inviting computer allows one guess in two minutes, and
/// both people must then see the same four pictures (`short_auth_string`), which a
/// computer in the middle cannot arrange.
pub fn short_code_secret(digits: &str) -> [u8; 16] {
    let digest = Sha256::digest([b"sovra-short-code-v1".as_slice(), digits.as_bytes()].concat());
    let mut secret = [0u8; 16];
    secret.copy_from_slice(&digest[..16]);
    secret
}

/// What a same-room proof is bound to: both computers (the joiner does not know the
/// group yet).
pub fn code_transcript(inviter: &DeviceId, joiner: &DeviceId) -> Vec<u8> {
    let mut bytes = b"sovra-code-join-v1".to_vec();
    bytes.extend_from_slice(inviter.as_bytes());
    bytes.extend_from_slice(joiner.as_bytes());
    bytes
}

/// A 4-bit hint a computer showing a code announces, so a joiner only contacts the
/// computers whose hint matches its code. Two people showing codes in one office
/// then rarely spend each other's single guess. It gives away 4 of the code's ~20
/// bits, leaving a nearby attacker one guess in about 62,500 before the picture check.
pub fn code_hint(digits: &str) -> u8 {
    Sha256::digest([b"sovra-code-hint-v1".as_slice(), digits.as_bytes()].concat())[0] & 0x0f
}
