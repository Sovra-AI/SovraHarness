//! What computers in a group say to each other, and the ports that carry it.
//!
//! Three requests are enough for a working group:
//!
//! - **Join**: a computer holding an invite proves it (bound to both computers' keys,
//!   so a captured proof is useless elsewhere) and receives the group's history.
//! - **Sync**: "here is what I have"; the answer carries what the asker lacks and lists
//!   what the answerer lacks, which the asker then **Push**es. Sets of immutable,
//!   hash-addressed operations make this order-free and idempotent.
//! - **Push**: new operations, sent as soon as they are made.
//!
//! Only computers in the roster may sync or push; joining needs a live invite. The
//! transport authenticates who is speaking (the QUIC handshake proves the key), so
//! handlers receive a trusted [`DeviceId`] and decide everything else here, purely.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Work the transport needs run in the background (one task per connection). The
/// runtime provides this so every task has an owner and is cancelled on shutdown.
pub type Spawn = Arc<dyn Fn(Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync>;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::fold::GroupState;
use super::keys::{DeviceId, GroupId, OpHash, PersonId, Signature, SignatureVerifier};
use super::knowledge::KnowledgeOp;
use super::membership::MembershipOp;

/// The application protocol name negotiated in the QUIC handshake. Versioned so
/// computers on different releases can be told apart.
pub const PEER_ALPN: &[u8] = b"sovra/group/1";
/// Largest single message either way; a group's whole history fits comfortably.
pub const MAX_PEER_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Most shared-knowledge bytes one message carries. Shared artifacts make a history
/// larger than one message; whatever does not fit follows on the next sync round.
pub const MAX_REPLY_KNOWLEDGE_BYTES: usize = 6 * 1024 * 1024;

/// Keeps operations, in order, until `budget` bytes of JSON are used. The first one
/// is always kept so every round makes progress. Returns whether any were left out.
pub fn within_budget(ops: Vec<KnowledgeOp>, budget: usize) -> (Vec<KnowledgeOp>, bool) {
    let mut used = 0usize;
    let mut kept = Vec::with_capacity(ops.len());
    let total = ops.len();
    for op in ops {
        let size = serde_json::to_vec(&op).map_or(usize::MAX, |bytes| bytes.len());
        if !kept.is_empty() && used.saturating_add(size) > budget {
            break;
        }
        used = used.saturating_add(size);
        kept.push(op);
    }
    let trimmed = kept.len() < total;
    (kept, trimmed)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum PeerRequest {
    Join {
        group: GroupId,
        proof: [u8; 32],
        person: PersonId,
        person_name: String,
        computer_name: String,
        /// How to reach the joiner, so the group can sync with it.
        from: PeerAddress,
    },
    Sync {
        group: GroupId,
        membership: BTreeSet<OpHash>,
        knowledge: BTreeSet<OpHash>,
        from: PeerAddress,
    },
    Push {
        group: GroupId,
        membership: Vec<MembershipOp>,
        knowledge: Vec<KnowledgeOp>,
    },
    /// Federated recall: "search your memory for this", answered only by computers
    /// whose owner turned answering on (see `group::federated`).
    Ask { group: GroupId, question: String },
    /// Same room: "I typed your code". The proof binds the code to both computers;
    /// the inviting computer allows one guess, then both people compare pictures.
    JoinByCode {
        proof: [u8; 32],
        person: PersonId,
        computer_name: String,
        from: PeerAddress,
    },
    /// Same room: the joining person's answer to "do the pictures match?". The
    /// inviting computer answers only once its own person has answered too.
    ConfirmJoin { matches: bool },
    /// Work handoff: "start answering this chat turn with your AI" (no tools travel).
    Work {
        group: GroupId,
        id: super::work::JobId,
        job: super::work::ChatJob,
    },
    /// "The answer's words after byte `from`" (answered within about a second).
    WorkNext {
        group: GroupId,
        id: super::work::JobId,
        from: usize,
    },
    /// "Stop that job now."
    WorkCancel {
        group: GroupId,
        id: super::work::JobId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum PeerResponse {
    /// The joiner's history: everything it needs to fold the group itself.
    Joined {
        membership: Vec<MembershipOp>,
        knowledge: Vec<KnowledgeOp>,
    },
    /// Operations the asker lacked, and hashes the answerer lacks.
    Synced {
        membership: Vec<MembershipOp>,
        knowledge: Vec<KnowledgeOp>,
        want_membership: BTreeSet<OpHash>,
        want_knowledge: BTreeSet<OpHash>,
    },
    Accepted,
    /// A few short answers from the asked computer's own memory.
    Answered {
        items: Vec<super::federated::FederatedItem>,
    },
    /// A helping computer accepted a chat job.
    WorkStarted,
    /// New words of a chat job's answer.
    WorkProgress {
        progress: super::work::WorkProgress,
    },
    /// The code was right: compare pictures now. Carries what the joiner needs to
    /// derive them and to know what it is joining.
    CodeCheck {
        group: GroupId,
        group_name: String,
        inviter_name: String,
        /// The name the joining computer's person will have in the group.
        person_name: String,
    },
    Refused {
        reason: Refusal,
    },
}

/// Why a request was turned down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    NotAMember,
    WrongGroup,
    InviteUnknown,
    InviteExpired,
    InviteUsed,
    Malformed,
    /// This computer's owner has not turned on answering, or it is asked too often.
    NotAnswering,
    /// This computer is not helping with work now (off, no AI, or busy).
    NotHelping,
}

/// The bytes a join proof is bound to: both computers and the group.
pub fn join_transcript(inviter: &DeviceId, joiner: &DeviceId, group: &GroupId) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(b"sovra-join-v1");
    hasher.update(inviter.as_bytes());
    hasher.update(joiner.as_bytes());
    hasher.update(group.as_bytes());
    hasher.finalize().to_vec()
}

/// Whether a request from `from` may be served at all. Joining is the only request a
/// computer outside the roster may make.
pub fn admits_request(
    roster: Option<&GroupState>,
    from: &DeviceId,
    request: &PeerRequest,
) -> Result<(), Refusal> {
    let group = match request {
        // Anyone nearby may try a same-room code; the code and pictures decide.
        PeerRequest::JoinByCode { .. } | PeerRequest::ConfirmJoin { .. } => {
            return roster.map(|_| ()).ok_or(Refusal::WrongGroup);
        }
        PeerRequest::Join { group, .. } => {
            return match roster {
                Some(state) if state.group == *group => Ok(()),
                _ => Err(Refusal::WrongGroup),
            };
        }
        PeerRequest::Sync { group, .. }
        | PeerRequest::Push { group, .. }
        | PeerRequest::Ask { group, .. }
        | PeerRequest::Work { group, .. }
        | PeerRequest::WorkNext { group, .. }
        | PeerRequest::WorkCancel { group, .. } => group,
    };
    match roster {
        Some(state) if state.group != *group => Err(Refusal::WrongGroup),
        Some(state) if state.admits(from) => Ok(()),
        Some(_) => Err(Refusal::NotAMember),
        None => Err(Refusal::WrongGroup),
    }
}

/// Where to reach a computer: its key plus hints. Hints are optional; with relays and
/// address lookup the key alone is enough once a computer is online.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAddress {
    pub device: DeviceId,
    pub relay: Option<String>,
    pub addrs: Vec<String>,
}

/// Why a request could not be delivered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("couldn't reach that computer: {0}")]
    Unreachable(String),
    #[error("the other computer answered something unexpected: {0}")]
    Protocol(String),
    #[error("the transport has shut down")]
    Closed,
}

/// Serves requests from other computers. Implemented by the runtime.
#[async_trait::async_trait]
pub trait PeerHandler: Send + Sync {
    async fn handle(&self, from: DeviceId, request: PeerRequest) -> PeerResponse;
}

/// The network as the runtime sees it. Implemented by the mesh adapter, which owns
/// this computer's device key and never lets it out.
#[async_trait::async_trait]
pub trait GroupTransport: Send + Sync {
    fn this_device(&self) -> DeviceId;
    fn sign(&self, message: &[u8]) -> Signature;
    fn verifier(&self) -> Arc<dyn SignatureVerifier + Send + Sync>;
    /// Lane encryption with this computer's device key (see `group::lane_keys`).
    fn cipher(&self) -> Arc<dyn super::lane_keys::GroupCipher>;
    /// Same room: tell computers on this network a code with this `hint` is showing
    /// here (`None` stops). See `group::code_hint`.
    fn set_pairing_open(&self, hint: Option<u8>);
    /// Same room: computers on this network showing a code with `hint`, heard
    /// within `listen_ms`.
    async fn nearby_pairing(&self, hint: u8, listen_ms: u64) -> Vec<PeerAddress>;
    /// How others can reach this computer right now (for join links).
    fn address(&self) -> PeerAddress;
    /// Accepts connections and hands each request to `handler`, until [`Self::close`].
    /// Each connection runs on `spawn`.
    async fn serve(&self, handler: Arc<dyn PeerHandler>, spawn: Spawn);
    async fn request(
        &self,
        to: &PeerAddress,
        request: PeerRequest,
    ) -> Result<PeerResponse, TransportError>;
    async fn close(&self);
}

/// Everything this computer keeps about its group.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GroupRecord {
    /// This computer's person, once it has founded or joined a group.
    pub person: Option<PersonId>,
    pub person_name: String,
    pub computer_name: String,
    pub group: Option<GroupId>,
    pub membership: Vec<MembershipOp>,
    pub knowledge: Vec<KnowledgeOp>,
    /// Last known way to reach each computer, learned from links and sync.
    pub addresses: Vec<PeerAddress>,
    /// Whether this computer answers and asks the group's memories (both off at first).
    #[serde(default)]
    pub federated: super::federated::FederatedSettings,
    /// Whether this computer helps with, and hands off, chat work (both off at first).
    #[serde(default)]
    pub work: super::work::WorkSettings,
}

/// Why the group record could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupStoreError {
    #[error("couldn't read saved group details: {0}")]
    Read(String),
    #[error("couldn't save group details: {0}")]
    Write(String),
}

/// Durable storage for [`GroupRecord`]. Implemented by an adapter.
#[async_trait::async_trait]
pub trait GroupStore: Send + Sync {
    async fn load(&self) -> Result<GroupRecord, GroupStoreError>;
    async fn save(&self, record: &GroupRecord) -> Result<(), GroupStoreError>;
}
