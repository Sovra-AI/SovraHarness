//! Groups: people and their computers sharing helpers, memory and work.
//!
//! This module is the pure half of the team stage (`docs/roadmap/02-a-team.md`):
//! identities, the signed membership log and the fold that turns it into a roster,
//! the pairing steps, and the code both screens show. It never touches the network,
//! a clock or a crypto library — the `sovra-mesh` adapter does, and reports here.
//!
//! "Group" is the user-facing word (roadmap decision D1); "team" stays the AI helpers.

mod dag;
mod federated;
mod fold;
mod keys;
mod knowledge;
mod link;
mod membership;
mod pairing;
mod sas;
mod session;
mod shared_artifact;
mod wire;

pub use federated::{
    ASKS_PER_MINUTE, AskLimiter, AskRecord, FederatedItem, FederatedSettings, GroupAnswer,
    MAX_ASK_CHARS, MAX_ASK_LOG, MAX_FEDERATED_ITEMS, MAX_FEDERATED_STATEMENT_CHARS, check_ask,
    federated_items, log_ask,
};
pub use fold::{DeviceEntry, GroupState, MAX_NAME_CHARS, Member, Rejection, fold};
pub use keys::{DeviceId, GroupId, KeyParseError, OpHash, PersonId, Signature, SignatureVerifier};
pub use knowledge::{
    DetailStatus, KnowledgeBody, KnowledgeOp, KnowledgeRejection, KnowledgeView, LaneId,
    MAX_SHARED_DETAIL_CHARS, MAX_SHARED_TOPICS, ShareError, SharedDetail, SharedHit,
    UnsignedKnowledge, check_share, fold_knowledge,
};
pub use link::{JOIN_LINK_PREFIX, JoinLink, LinkError, MAX_JOIN_LINK_CHARS};
pub use membership::{MembershipBody, MembershipOp, Role, UnsignedOp};
pub use pairing::{
    INVITE_LIFETIME_MS, Invite, InviteBook, InviteKind, PairingFailure, PairingStage, PairingStep,
    RedeemError, SHORT_CODE_LIFETIME_MS, ShortCodeWindow, invite_proof,
};
pub use sas::{SAS_EMOJI, ShortAuthString, short_auth_string};
pub use session::{
    ComputerRow, GroupCommand, GroupEvent, GroupProblem, GroupsView, PersonRow, removal_warning,
    role_label, shareable_text, shareable_topics,
};
pub use shared_artifact::{
    ArtifactShareError, MAX_ORIGIN_CHARS, MAX_SHARED_ARTIFACT_BYTES, MAX_SHARED_LANGUAGE_CHARS,
    MAX_SHARED_TITLE_CHARS, SharedArtifact, SharingState, check_artifact, lineage_ids,
    sharing_state,
};
pub use wire::{
    GroupRecord, GroupStore, GroupStoreError, GroupTransport, MAX_PEER_MESSAGE_BYTES,
    MAX_REPLY_KNOWLEDGE_BYTES, PEER_ALPN, PeerAddress, PeerHandler, PeerRequest, PeerResponse,
    Refusal, Spawn, TransportError, admits_request, join_transcript, within_budget,
};

#[cfg(test)]
mod tests;
