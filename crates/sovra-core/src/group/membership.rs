//! Membership operations: the signed, hash-linked history of who is in a group.
//!
//! Each device keeps its own chain (`seq` + `prev`) and names the operations it had
//! already seen (`deps`), so the history is a causal DAG with no central order. The
//! fold in [`super::fold`] turns any set of these operations into one roster.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::keys::{DeviceId, GroupId, OpHash, PersonId, Signature};
use crate::Timestamp;

/// What a person may do in a group. Plain-word labels live in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Reads shared lanes; changes nothing.
    Viewer,
    /// Adds to shared lanes but cannot read them (for example a shopping list).
    Contributor,
    /// Reads and writes shared lanes.
    Member,
    /// Manages people, devices, roles and the group's name.
    Admin,
}

impl Role {
    pub const fn manages(self) -> bool {
        matches!(self, Self::Admin)
    }

    pub const fn writes(self) -> bool {
        matches!(self, Self::Admin | Self::Member | Self::Contributor)
    }

    pub const fn reads(self) -> bool {
        matches!(self, Self::Admin | Self::Member | Self::Viewer)
    }
}

/// The change an operation makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MembershipBody {
    /// Starts the group. Only valid from the founding device, as the first operation.
    Genesis {
        name: String,
        nonce: [u8; 16],
        founder: PersonId,
        founder_name: String,
        device_name: String,
    },
    AddPerson {
        person: PersonId,
        name: String,
        role: Role,
    },
    AddDevice {
        person: PersonId,
        device: DeviceId,
        name: String,
    },
    RemoveDevice {
        device: DeviceId,
    },
    RemovePerson {
        person: PersonId,
    },
    ChangeRole {
        person: PersonId,
        role: Role,
    },
    Rename {
        name: String,
    },
    /// Admins only: creates department `slug`, or replaces its name and members.
    /// An empty member list closes it to new content; its history stays.
    SetDepartment {
        slug: crate::swarm::department::DepartmentId,
        name: String,
        members: std::collections::BTreeSet<PersonId>,
    },
}

/// Everything an operation commits to, in the order it is signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedOp {
    pub group: GroupId,
    pub author: DeviceId,
    /// Position in the author device's own chain, starting at 1.
    pub seq: u64,
    /// The author's previous operation; `None` only when `seq == 1`.
    pub prev: Option<OpHash>,
    /// Other operations the author had already applied (causal dependencies).
    pub deps: Vec<OpHash>,
    /// The author's clock. A display hint and tie-breaker, never an authority.
    pub at: Timestamp,
    pub body: MembershipBody,
}

/// A signed membership operation as stored and sent between computers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipOp {
    #[serde(flatten)]
    pub unsigned: UnsignedOp,
    pub signature: Signature,
}

impl UnsignedOp {
    /// The exact bytes a device signs: a domain tag followed by the canonical JSON of
    /// this struct. Field order is fixed by the struct, so every computer agrees.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = b"sovra-membership-v1\n".to_vec();
        bytes.extend(
            serde_json::to_vec(self).expect("membership operations are plain serialisable data"),
        );
        bytes
    }

    pub fn hash(&self) -> OpHash {
        OpHash(Sha256::digest(self.signing_bytes()).into())
    }

    pub fn sign(self, signature: Signature) -> MembershipOp {
        MembershipOp {
            unsigned: self,
            signature,
        }
    }
}

impl MembershipOp {
    pub fn hash(&self) -> OpHash {
        self.unsigned.hash()
    }

    pub fn signing_bytes(&self) -> Vec<u8> {
        self.unsigned.signing_bytes()
    }

    pub const fn author(&self) -> DeviceId {
        self.unsigned.author
    }

    pub const fn body(&self) -> &MembershipBody {
        &self.unsigned.body
    }

    /// Every operation this one causally follows (its own chain plus `deps`).
    pub fn parents(&self) -> impl Iterator<Item = OpHash> + '_ {
        self.unsigned
            .prev
            .iter()
            .copied()
            .chain(self.unsigned.deps.iter().copied())
    }
}
