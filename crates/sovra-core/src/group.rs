//! Groups: people and their computers sharing helpers, memory and work.
//!
//! This module is the pure half of the team stage (`docs/roadmap/02-a-team.md`):
//! identities, the signed membership log and the fold that turns it into a roster,
//! the pairing steps, and the code both screens show. It never touches the network,
//! a clock or a crypto library — the `sovra-mesh` adapter does, and reports here.
//!
//! "Group" is the user-facing word (roadmap decision D1); "team" stays the AI helpers.

mod fold;
mod keys;
mod link;
mod membership;
mod pairing;
mod sas;
mod session;

pub use fold::{DeviceEntry, GroupState, MAX_NAME_CHARS, Member, Rejection, fold};
pub use keys::{DeviceId, GroupId, KeyParseError, OpHash, PersonId, Signature, SignatureVerifier};
pub use link::{JOIN_LINK_PREFIX, JoinLink, LinkError, MAX_JOIN_LINK_CHARS};
pub use membership::{MembershipBody, MembershipOp, Role, UnsignedOp};
pub use pairing::{
    INVITE_LIFETIME_MS, Invite, InviteBook, InviteKind, PairingFailure, PairingStage, PairingStep,
    RedeemError, SHORT_CODE_LIFETIME_MS, ShortCodeWindow, invite_proof,
};
pub use sas::{SAS_EMOJI, ShortAuthString, short_auth_string};
pub use session::{
    ComputerRow, GroupCommand, GroupEvent, GroupProblem, GroupsView, PersonRow, removal_warning,
    role_label,
};

#[cfg(test)]
mod tests;
