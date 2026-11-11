//! The audit ledger: a shared, append-only history of who stored, copied, read,
//! purged or sent what, and of every model update. A ledger without a coin.
//!
//! - Each **entry** is signed by the computer that saw the event and chained to that
//!   computer's previous entry (`seq`, `prev`), so a computer cannot quietly drop or
//!   reorder its own history.
//! - The ledger keeps the entries in one order and a Merkle tree over them
//!   ([`super::merkle`], RFC 9162 with BLAKE3). Its **sequencer** signs
//!   **checkpoints** (size + root).
//! - **Witnesses** (security, an auditor) co-sign a checkpoint only after checking it
//!   extends the last one they saw, so nobody can be shown a different history.
//! - Replicas take new entries only together with a checkpoint that matches them.
//!
//! Payloads are never in the ledger: only BLAKE3 hashes and department ids. A purge
//! removes the payload where it lives and adds a `Purged` entry; the hash stays, so
//! erasure does not break the history (roadmap stage 3 §2).
//!
//! Core never signs. Callers sign [`UnsignedEntry::signing_bytes`] with the device
//! key in the mesh adapter; core only asks a [`SignatureVerifier`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::department::DepartmentId;
use super::merkle;
use crate::Timestamp;
use crate::group::{ContentHash, DeviceId, PersonId, Signature, SignatureVerifier};

const ENTRY_TAG: &[u8] = b"sovra-ledger-v1\n";
const CHECKPOINT_TAG: &str = "sovra-checkpoint-v1";
const COSIGN_TAG: &[u8] = b"sovra-cosignature-v1\n";

/// What kind of thing an item is, for the residency report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Document,
    Memory,
    VectorIndex,
    Model,
}

/// One thing that happened to data or to a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEvent {
    /// An item was saved on `device`.
    Stored {
        item: ContentHash,
        kind: ItemKind,
        department: DepartmentId,
        device: DeviceId,
    },
    /// An item was copied onto another computer.
    Replicated {
        item: ContentHash,
        department: DepartmentId,
        to: DeviceId,
    },
    /// An item's payload was erased from `device`. Its hash stays in history.
    Purged {
        item: ContentHash,
        department: DepartmentId,
        device: DeviceId,
    },
    /// A person, or a helper acting for them, read an item.
    Read {
        item: ContentHash,
        department: DepartmentId,
        person: PersonId,
        helper: Option<String>,
    },
    /// A person signed off on something leaving its computer.
    EgressApproved {
        content: ContentHash,
        department: DepartmentId,
        approver: PersonId,
        destination: String,
    },
    /// A training round produced a candidate model. `rho_micro` is the zCDP privacy
    /// cost of the round in millionths.
    ModelRound {
        model: ContentHash,
        parent: Option<ContentHash>,
        round: u64,
        departments: BTreeSet<DepartmentId>,
        contributors: u32,
        rho_micro: u64,
    },
    /// People promoted a candidate model for use: an approver from every
    /// department whose knowledge trained it.
    ModelPromoted {
        model: ContentHash,
        approvers: BTreeSet<PersonId>,
    },
    /// A person switched back to an earlier model.
    ModelRolledBack {
        from: ContentHash,
        to: ContentHash,
        approver: PersonId,
    },
}

/// An entry before it is signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedEntry {
    /// Which ledger, e.g. `acme/audit`. Binds the signature to one ledger.
    pub origin: String,
    pub author: DeviceId,
    /// The author's own counter, starting at 1.
    pub seq: u64,
    /// The hash of the author's previous entry (`None` only at `seq` 1).
    pub prev: Option<ContentHash>,
    pub at: Timestamp,
    pub event: AuditEvent,
}

impl UnsignedEntry {
    /// The bytes the author signs: a domain tag and the entry as JSON.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = ENTRY_TAG.to_vec();
        bytes.extend(serde_json::to_vec(self).expect("ledger entries are plain data"));
        bytes
    }

    pub fn sign_with(self, signature: Signature) -> LedgerEntry {
        LedgerEntry {
            body: self,
            signature,
        }
    }
}

/// A signed entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub body: UnsignedEntry,
    pub signature: Signature,
}

impl LedgerEntry {
    /// The entry's id: BLAKE3 of the signed bytes. Authors chain on this.
    pub fn hash(&self) -> ContentHash {
        ContentHash::of(&self.body.signing_bytes())
    }

    /// The Merkle leaf: the signed bytes and the signature, so the tree commits to
    /// who signed as well as what.
    pub fn leaf(&self) -> ContentHash {
        let mut bytes = self.body.signing_bytes();
        bytes.extend_from_slice(&self.signature.0);
        merkle::leaf_hash(&bytes)
    }
}

/// Why the ledger refused an entry or a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    #[error("the entry is for another ledger")]
    WrongOrigin,
    #[error("the signature doesn't match the computer that claims to have written it")]
    BadSignature,
    #[error("the entry skips or repeats in its computer's history (expected {expected})")]
    OutOfOrder { expected: u64 },
    #[error("the entry doesn't follow its computer's previous entry")]
    BrokenChain,
    #[error("the checkpoint doesn't match these entries")]
    CheckpointMismatch,
    #[error("the checkpoint wasn't signed by this ledger's sequencer")]
    UntrustedCheckpoint,
}

/// What appending did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    /// The entry is new and now sits at this index.
    New(u64),
    /// The ledger already had it at this index; nothing changed.
    Known(u64),
}

/// A tree size and root, as the sequencer signs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub origin: String,
    pub size: u64,
    pub root: ContentHash,
}

impl Checkpoint {
    /// The bytes a sequencer signs (a C2SP signed-note style body).
    pub fn signing_bytes(&self) -> Vec<u8> {
        format!(
            "{CHECKPOINT_TAG}\n{}\n{}\n{}\n",
            self.origin,
            self.size,
            self.root.to_hex()
        )
        .into_bytes()
    }

    /// The bytes a witness signs to co-sign it.
    pub fn cosigning_bytes(&self) -> Vec<u8> {
        let mut bytes = COSIGN_TAG.to_vec();
        bytes.extend(self.signing_bytes());
        bytes
    }
}

/// A witness's co-signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cosignature {
    pub witness: DeviceId,
    pub signature: Signature,
}

/// A checkpoint with its sequencer's signature and any witness co-signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCheckpoint {
    pub checkpoint: Checkpoint,
    pub signer: DeviceId,
    pub signature: Signature,
    #[serde(default)]
    pub cosignatures: Vec<Cosignature>,
}

impl SignedCheckpoint {
    /// Whether the sequencer signed it and at least `quorum` of `witnesses` co-signed.
    pub fn verify(
        &self,
        sequencer: &DeviceId,
        witnesses: &BTreeSet<DeviceId>,
        quorum: usize,
        verifier: &dyn SignatureVerifier,
    ) -> bool {
        if self.signer != *sequencer
            || !verifier.verify(sequencer, &self.checkpoint.signing_bytes(), &self.signature)
        {
            return false;
        }
        let bytes = self.checkpoint.cosigning_bytes();
        let cosigned: BTreeSet<DeviceId> = self
            .cosignatures
            .iter()
            .filter(|c| witnesses.contains(&c.witness))
            .filter(|c| verifier.verify(&c.witness, &bytes, &c.signature))
            .map(|c| c.witness)
            .collect();
        cosigned.len() >= quorum
    }
}

/// The ordered history and its tree. Kept by the sequencer and by every replica.
///
/// Not serialisable on purpose: save [`Self::entries`] and load with
/// [`Self::restore`], which re-checks every signature and chain instead of trusting
/// whatever was on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditLedger {
    pub origin: String,
    entries: Vec<LedgerEntry>,
    leaves: Vec<ContentHash>,
    index: BTreeMap<ContentHash, u64>,
    heads: BTreeMap<DeviceId, (u64, ContentHash)>,
}

impl AuditLedger {
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            ..Self::default()
        }
    }

    /// Rebuilds a saved ledger, re-checking every entry. Stops at the first bad one.
    pub fn restore(
        origin: impl Into<String>,
        entries: Vec<LedgerEntry>,
        verifier: &dyn SignatureVerifier,
    ) -> Result<Self, LedgerError> {
        let mut ledger = Self::new(origin);
        for entry in entries {
            ledger.append(entry, verifier)?;
        }
        Ok(ledger)
    }

    pub fn len(&self) -> u64 {
        self.entries.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// The next `(seq, prev)` for an entry by `author`.
    pub fn next_for(&self, author: &DeviceId) -> (u64, Option<ContentHash>) {
        self.heads
            .get(author)
            .map_or((1, None), |(seq, hash)| (seq + 1, Some(*hash)))
    }

    /// Adds a signed entry at the end, after checking its signature and its
    /// author's chain. Re-adding a known entry changes nothing.
    pub fn append(
        &mut self,
        entry: LedgerEntry,
        verifier: &dyn SignatureVerifier,
    ) -> Result<Appended, LedgerError> {
        let hash = entry.hash();
        if let Some(index) = self.index.get(&hash) {
            return Ok(Appended::Known(*index));
        }
        if entry.body.origin != self.origin {
            return Err(LedgerError::WrongOrigin);
        }
        if !verifier.verify(
            &entry.body.author,
            &entry.body.signing_bytes(),
            &entry.signature,
        ) {
            return Err(LedgerError::BadSignature);
        }
        let (expected, prev) = self.next_for(&entry.body.author);
        if entry.body.seq != expected {
            return Err(LedgerError::OutOfOrder { expected });
        }
        if entry.body.prev != prev {
            return Err(LedgerError::BrokenChain);
        }
        let at = self.len();
        self.heads.insert(entry.body.author, (entry.body.seq, hash));
        self.index.insert(hash, at);
        self.leaves.push(entry.leaf());
        self.entries.push(entry);
        Ok(Appended::New(at))
    }

    /// The checkpoint for the ledger as it is now, for the sequencer to sign.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            origin: self.origin.clone(),
            size: self.len(),
            root: merkle::root(&self.leaves),
        }
    }

    /// The checkpoint for the first `size` entries, if there are that many.
    pub fn checkpoint_at(&self, size: u64) -> Option<Checkpoint> {
        let size_usize = usize::try_from(size).ok()?;
        (size_usize <= self.leaves.len()).then(|| Checkpoint {
            origin: self.origin.clone(),
            size,
            root: merkle::root(&self.leaves[..size_usize]),
        })
    }

    /// Proof that entry `index` is in the current tree.
    pub fn inclusion_proof(&self, index: u64) -> Option<Vec<ContentHash>> {
        merkle::inclusion_proof(usize::try_from(index).ok()?, &self.leaves)
    }

    /// Proof that the current tree extends the tree of its first `old_size` entries.
    pub fn consistency_proof(&self, old_size: u64) -> Option<Vec<ContentHash>> {
        merkle::consistency_proof(usize::try_from(old_size).ok()?, &self.leaves)
    }

    /// A replica takes `entries` from the sequencer only if, appended, they produce
    /// exactly the signed `checkpoint`. On any mismatch nothing is kept.
    pub fn extend_verified(
        &mut self,
        entries: Vec<LedgerEntry>,
        checkpoint: &SignedCheckpoint,
        sequencer: &DeviceId,
        verifier: &dyn SignatureVerifier,
    ) -> Result<(), LedgerError> {
        if !checkpoint.verify(sequencer, &BTreeSet::new(), 0, verifier) {
            return Err(LedgerError::UntrustedCheckpoint);
        }
        let mut next = self.clone();
        for entry in entries {
            next.append(entry, verifier)?;
        }
        if next.checkpoint() != checkpoint.checkpoint {
            return Err(LedgerError::CheckpointMismatch);
        }
        *self = next;
        Ok(())
    }

    /// Where every item is now, folded from the whole history.
    pub fn residency(&self) -> BTreeMap<ContentHash, Residency> {
        residency(self.entries.iter().map(|entry| &entry.body.event))
    }
}

/// Where one item lives and what has happened to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residency {
    pub kind: Option<ItemKind>,
    pub department: Option<DepartmentId>,
    /// Computers holding the payload now.
    pub holders: BTreeSet<DeviceId>,
    /// Computers that held it and erased it.
    pub purged_from: BTreeSet<DeviceId>,
    pub reads: u64,
    /// Times a person signed off on it leaving its computer.
    pub releases: u64,
}

/// Folds events into a residency report: the ledger's proof of where everything is.
pub fn residency<'a>(
    events: impl IntoIterator<Item = &'a AuditEvent>,
) -> BTreeMap<ContentHash, Residency> {
    let mut report: BTreeMap<ContentHash, Residency> = BTreeMap::new();
    for event in events {
        match event {
            AuditEvent::Stored {
                item,
                kind,
                department,
                device,
            } => {
                let entry = report.entry(*item).or_default();
                entry.kind = Some(*kind);
                entry.department = Some(department.clone());
                entry.holders.insert(*device);
                entry.purged_from.remove(device);
            }
            AuditEvent::Replicated {
                item,
                department,
                to,
            } => {
                let entry = report.entry(*item).or_default();
                entry.department.get_or_insert_with(|| department.clone());
                entry.holders.insert(*to);
                entry.purged_from.remove(to);
            }
            AuditEvent::Purged { item, device, .. } => {
                let entry = report.entry(*item).or_default();
                if entry.holders.remove(device) {
                    entry.purged_from.insert(*device);
                }
            }
            AuditEvent::Read { item, .. } => report.entry(*item).or_default().reads += 1,
            AuditEvent::EgressApproved { content, .. } => {
                report.entry(*content).or_default().releases += 1;
            }
            AuditEvent::ModelRound { model, .. } => {
                report.entry(*model).or_default().kind = Some(ItemKind::Model);
            }
            AuditEvent::ModelPromoted { .. } | AuditEvent::ModelRolledBack { .. } => {}
        }
    }
    report
}

/// Copies the ledger shows that the department rules forbid: an item on a computer
/// whose owner is not in the item's department.
pub fn custody_violations(
    report: &BTreeMap<ContentHash, Residency>,
    directory: &super::department::Directory,
) -> Vec<(ContentHash, DeviceId)> {
    report
        .iter()
        .filter_map(|(item, residency)| residency.department.as_ref().map(|d| (item, d, residency)))
        .flat_map(|(item, department, residency)| {
            residency
                .holders
                .iter()
                .filter(|device| directory.may_replicate(department, device).is_err())
                .map(|device| (*item, *device))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Which model is in use, folded from the history: the latest promotion or
/// rollback wins. `None` until a person promotes one.
pub fn active_model<'a>(events: impl IntoIterator<Item = &'a AuditEvent>) -> Option<ContentHash> {
    let mut active = None;
    for event in events {
        match event {
            AuditEvent::ModelPromoted { model, .. } => active = Some(*model),
            AuditEvent::ModelRolledBack { to, .. } => active = Some(*to),
            AuditEvent::Stored { .. }
            | AuditEvent::Replicated { .. }
            | AuditEvent::Purged { .. }
            | AuditEvent::Read { .. }
            | AuditEvent::EgressApproved { .. }
            | AuditEvent::ModelRound { .. } => {}
        }
    }
    active
}

/// A witness: remembers the last checkpoint it co-signed and refuses any later one
/// that does not extend it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    pub origin: String,
    pub sequencer: DeviceId,
    pub latest: Option<Checkpoint>,
}

/// Why a witness refused to co-sign.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessError {
    #[error("the checkpoint wasn't signed by this ledger's sequencer")]
    Untrusted,
    #[error("the checkpoint is for another ledger")]
    WrongOrigin,
    #[error("the ledger got shorter: history was removed")]
    Rollback,
    #[error("this history doesn't extend the one already signed: someone rewrote it")]
    Fork,
}

impl Witness {
    pub fn new(origin: impl Into<String>, sequencer: DeviceId) -> Self {
        Self {
            origin: origin.into(),
            sequencer,
            latest: None,
        }
    }

    /// Checks `signed` against the last co-signed checkpoint with `proof` (from
    /// [`AuditLedger::consistency_proof`] at the old size). On success remembers it
    /// and returns the bytes to co-sign.
    pub fn observe(
        &mut self,
        signed: &SignedCheckpoint,
        proof: &[ContentHash],
        verifier: &dyn SignatureVerifier,
    ) -> Result<Vec<u8>, WitnessError> {
        if signed.checkpoint.origin != self.origin {
            return Err(WitnessError::WrongOrigin);
        }
        if !signed.verify(&self.sequencer, &BTreeSet::new(), 0, verifier) {
            return Err(WitnessError::Untrusted);
        }
        let new = &signed.checkpoint;
        if let Some(old) = &self.latest {
            if new.size < old.size {
                return Err(WitnessError::Rollback);
            }
            if !merkle::verify_consistency(old.size, new.size, &old.root, &new.root, proof) {
                return Err(WitnessError::Fork);
            }
        }
        self.latest = Some(new.clone());
        Ok(new.cosigning_bytes())
    }
}

/// Checks that `entry` is entry `index` of the history `checkpoint` describes, so an
/// auditor can confirm one record without downloading the ledger.
pub fn verify_entry(
    entry: &LedgerEntry,
    index: u64,
    proof: &[ContentHash],
    checkpoint: &Checkpoint,
) -> bool {
    merkle::verify_inclusion(
        &entry.leaf(),
        index,
        checkpoint.size,
        proof,
        &checkpoint.root,
    )
}
