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
mod lane_keys;
mod link;
mod membership;
mod pairing;
mod sas;
mod session;
mod shared_artifact;
mod space;
mod template;
mod wire;
mod work;

pub use federated::{
    ASKS_PER_MINUTE, AskLimiter, AskRecord, FederatedItem, FederatedSettings, GroupAnswer,
    MAX_ASK_CHARS, MAX_ASK_LOG, MAX_FEDERATED_ITEMS, MAX_FEDERATED_STATEMENT_CHARS, check_ask,
    federated_items, log_ask,
};
pub use fold::{DeviceEntry, GroupDepartment, GroupState, MAX_NAME_CHARS, Member, Rejection, fold};
pub use keys::{
    ContentHash, DeviceId, GroupId, KeyParseError, OpHash, PersonId, Signature, SignatureVerifier,
};
pub use knowledge::{
    DetailStatus, KnowledgeBody, KnowledgeOp, KnowledgeRejection, KnowledgeView, LaneId,
    MAX_SHARED_DETAIL_CHARS, MAX_SHARED_TOPICS, ShareError, SharedDetail, SharedHit,
    UnsignedKnowledge, check_share, fold_knowledge, fold_knowledge_opened, fold_space,
};
pub use lane_keys::{Bytes, GroupCipher, LaneKey, current_devices, sealed_aad, usable_key};
pub use link::{JOIN_LINK_PREFIX, JoinLink, LinkError, MAX_JOIN_LINK_CHARS};
pub use membership::{MembershipBody, MembershipOp, Role, UnsignedOp};
pub use pairing::{
    INVITE_LIFETIME_MS, Invite, InviteBook, InviteKind, PairingFailure, PairingStage, PairingStep,
    RedeemError, SHORT_CODE_LIFETIME_MS, ShortCodeWindow, code_hint, code_transcript, format_code,
    invite_proof, normalise_code, short_code_secret,
};
pub use sas::{SAS_EMOJI, ShortAuthString, short_auth_string};
pub use session::{
    ComputerRow, GroupCommand, GroupEvent, GroupProblem, GroupsView, ONLINE_WINDOW_MS, PersonRow,
    presence_label, removal_warning, role_label, shareable_text, shareable_topics,
};
pub use shared_artifact::{
    ArtifactShareError, MAX_ORIGIN_CHARS, MAX_SHARED_ARTIFACT_BYTES, MAX_SHARED_LANGUAGE_CHARS,
    MAX_SHARED_TITLE_CHARS, SharedArtifact, SharingState, check_artifact, lineage_ids,
    sharing_state,
};
pub use space::{Space, department_ref, lanes_of, space_of_lane, spaces_of};
pub use template::{
    MAX_TEMPLATE_INSTRUCTIONS_CHARS, MAX_TEMPLATE_LINE_CHARS, MAX_TEMPLATE_TOPICS, SharedTemplate,
    Template, TemplateError, check_template,
};
pub use wire::{
    GossipHandler, GroupRecord, GroupStore, GroupStoreError, GroupTransport,
    MAX_PEER_MESSAGE_BYTES, MAX_REPLY_KNOWLEDGE_BYTES, PEER_ALPN, PeerAddress, PeerHandler,
    PeerRequest, PeerResponse, Refusal, Spawn, TransportError, admits_request, join_transcript,
    within_budget,
};
pub use work::{
    ChatJob, JOBS_PER_MINUTE, JobAnswer, JobBuffer, JobId, JobTurn, MAX_JOB_BYTES, MAX_JOB_TURNS,
    MAX_WORK_LOG, WorkError, WorkModel, WorkPeer, WorkProgress, WorkRecord, WorkSettings,
    WorkTicket, check_job, helper_order, log_work,
};

#[cfg(test)]
mod tests;
