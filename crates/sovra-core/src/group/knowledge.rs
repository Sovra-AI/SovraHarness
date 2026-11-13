//! Shared knowledge: details people choose to share with their group.
//!
//! Each detail is a signed operation in the group's shared lane. Nothing is edited
//! or deleted in place: a correction is a new detail that *replaces* older ones, and
//! "erase the words" is a redaction the fold applies everywhere. Any set of these
//! operations folds to the same [`KnowledgeView`] on every computer:
//!
//! - Signature, group, lane and own-chain shape are checked; forks quarantine a device.
//! - A replacement retires every detail it names that its author had seen.
//! - Two corrections of the same detail made without seeing each other are a
//!   **dispute**: both stay visible, one is the display choice (later `valid_at`, then
//!   later share time, then hash), and a later correction naming both settles it.
//! - Details from a computer that is no longer in the group are kept but hidden from
//!   answers. That is conservative and needs no clock: someone removed cannot keep
//!   adding "old-looking" details to what others' helpers read.
//! - Only the author's person, or an admin, may withdraw or redact a detail.
//!
//! [`KnowledgeView::search`] is the keyword leg helpers use; it ranks with a small
//! BM25 and weights agreement and disputes so a lone contested claim never outranks a
//! corroborated one.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::dag::{self, DagNode};
use super::fold::GroupState;
use super::keys::{DeviceId, GroupId, OpHash, PersonId, Signature, SignatureVerifier};
use super::lane_keys::{Bytes, LaneKey};
use super::shared_artifact::{MAX_ORIGIN_CHARS, SharedArtifact, check_artifact};
use super::space::Space;
use crate::Timestamp;
use crate::artifacts::ArtifactKind;

/// Longest shared detail, in characters (a paragraph, not a document).
pub const MAX_SHARED_DETAIL_CHARS: usize = 2_000;
/// At most this many topic names per detail.
pub const MAX_SHARED_TOPICS: usize = 8;

/// Which shared space a detail lives in. Today every group has one shared lane;
/// person-to-person lanes come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LaneId(pub OpHash);

impl LaneId {
    /// The whole-group lane.
    pub fn whole_group(group: &GroupId) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"sovra-lane-v1/group");
        hasher.update(group.as_bytes());
        Self(OpHash(hasher.finalize().into()))
    }
}

/// The change a knowledge operation makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum KnowledgeBody {
    /// A detail, optionally correcting earlier ones.
    Share {
        text: String,
        about: Vec<String>,
        valid_at: Option<Timestamp>,
        replaces: Vec<OpHash>,
    },
    /// "I agree" from another person.
    Corroborate { detail: OpHash },
    /// Stop using a detail in answers (it stays in history).
    Withdraw { detail: OpHash },
    /// Erase a detail's words on every computer; only the record that it existed stays.
    Redact { detail: OpHash },
    /// An artifact (document, checklist, picture…), optionally a newer version of
    /// earlier shared ones. Withdraw and Redact name artifacts the same way as details.
    ShareArtifact {
        title: String,
        kind: ArtifactKind,
        language: String,
        content: String,
        replaces: Vec<OpHash>,
        /// The sharer's own library id for it, so their later edits of that artifact
        /// are shared as its new versions. Not meaningful on other computers.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        origin: String,
    },
    /// A helper or schedule, optionally an update of earlier shared ones.
    ShareTemplate {
        template: super::template::Template,
        replaces: Vec<OpHash>,
    },
    /// Hands the lane key `key` to member computers: one sealed copy per computer.
    KeyGrant {
        key: OpHash,
        grants: Vec<(DeviceId, Bytes)>,
    },
    /// Any other operation, encrypted with lane key `key` (see `group::lane_keys`).
    Sealed { key: OpHash, ciphertext: Bytes },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedKnowledge {
    pub group: GroupId,
    pub lane: LaneId,
    pub author: DeviceId,
    pub seq: u64,
    pub prev: Option<OpHash>,
    pub deps: Vec<OpHash>,
    pub at: Timestamp,
    pub body: KnowledgeBody,
}

/// A signed knowledge operation. Read-only once made: its hash is computed once and
/// kept, because a shared artifact makes hashing the whole operation costly and every
/// merge, fold and sync asks for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeOp {
    #[serde(flatten)]
    pub(super) unsigned: UnsignedKnowledge,
    pub(super) signature: Signature,
    #[serde(skip)]
    hash: std::sync::OnceLock<OpHash>,
    /// Whether the signature checked out, remembered after the first fold. Each
    /// computer checks with one verifier (its own transport's), so the answer holds.
    #[serde(skip)]
    signed: std::sync::OnceLock<bool>,
}

impl PartialEq for KnowledgeOp {
    fn eq(&self, other: &Self) -> bool {
        self.unsigned == other.unsigned && self.signature == other.signature
    }
}

impl Eq for KnowledgeOp {}

impl UnsignedKnowledge {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = b"sovra-knowledge-v1\n".to_vec();
        bytes.extend(serde_json::to_vec(self).expect("knowledge operations are plain data"));
        bytes
    }

    pub fn hash(&self) -> OpHash {
        OpHash(Sha256::digest(self.signing_bytes()).into())
    }

    pub fn sign(self, signature: Signature) -> KnowledgeOp {
        KnowledgeOp {
            unsigned: self,
            signature,
            hash: std::sync::OnceLock::new(),
            signed: std::sync::OnceLock::new(),
        }
    }
}

impl KnowledgeOp {
    pub fn hash(&self) -> OpHash {
        *self.hash.get_or_init(|| self.unsigned.hash())
    }

    /// Whether its author really signed it; checked once, then remembered.
    pub fn signed_by_author(&self, verifier: &dyn SignatureVerifier) -> bool {
        *self.signed.get_or_init(|| {
            verifier.verify(
                &self.unsigned.author,
                &self.unsigned.signing_bytes(),
                &self.signature,
            )
        })
    }

    pub fn unsigned(&self) -> &UnsignedKnowledge {
        &self.unsigned
    }

    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    fn node(&self) -> DagNode {
        DagNode {
            author: self.unsigned.author,
            seq: self.unsigned.seq,
            prev: self.unsigned.prev,
            deps: self.unsigned.deps.clone(),
            at: self.unsigned.at,
        }
    }
}

/// Why a shared detail could not be shared (checked before signing, and again on fold).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    #[error("there's nothing to share")]
    Empty,
    #[error("that's too long to share as one detail")]
    TooLong,
    #[error("too many topics")]
    TooManyTopics,
}

/// The rules a detail must meet, used by the sharer and by every receiver.
pub fn check_share(text: &str, about: &[String]) -> Result<(), ShareError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ShareError::Empty);
    }
    if trimmed.chars().count() > MAX_SHARED_DETAIL_CHARS {
        return Err(ShareError::TooLong);
    }
    if about.len() > MAX_SHARED_TOPICS {
        return Err(ShareError::TooManyTopics);
    }
    Ok(())
}

/// Where a detail stands now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DetailStatus {
    Current,
    /// Corrected by these later details.
    Replaced {
        by: Vec<OpHash>,
    },
    /// Corrected differently at the same time as these; `chosen` is the display pick.
    Disputed {
        with: Vec<OpHash>,
        chosen: bool,
    },
    Withdrawn,
    Redacted,
}

/// One shared detail as people and helpers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDetail {
    pub id: OpHash,
    pub text: String,
    pub about: Vec<String>,
    pub author: DeviceId,
    /// The sharer's person and display name, from the roster.
    pub person: Option<PersonId>,
    pub shared_by: String,
    pub computer: String,
    pub shared_at: Timestamp,
    pub valid_at: Option<Timestamp>,
    pub replaces: Vec<OpHash>,
    pub status: DetailStatus,
    /// Other people who agreed.
    pub agreed_by: BTreeSet<PersonId>,
    /// The sharer's computer is no longer in the group.
    pub from_former_member: bool,
    /// Who it is shared with: the whole group, or just two people.
    #[serde(default)]
    pub space: Space,
}

impl SharedDetail {
    /// Whether helpers may use it in answers.
    pub fn usable(&self) -> bool {
        !self.from_former_member
            && matches!(
                self.status,
                DetailStatus::Current | DetailStatus::Disputed { .. }
            )
    }
}

/// Why an operation is not part of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRejection {
    WrongGroup,
    WrongLane,
    BadSignature,
    BrokenChain,
    Forked,
    ParentRejected,
    UnknownAuthor,
    NotAllowed,
    InvalidDetail,
    UnknownDetail,
}

/// Everything shared in a lane, folded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgeView {
    pub details: BTreeMap<OpHash, SharedDetail>,
    /// Every shared artifact version, by id.
    pub artifacts: BTreeMap<OpHash, SharedArtifact>,
    /// Shared helpers and schedules, by id.
    pub templates: BTreeMap<OpHash, super::template::SharedTemplate>,
    /// Lane keys and which computers hold a copy.
    pub keys: BTreeMap<OpHash, LaneKey>,
    /// Sealed operations this computer cannot open (yet): no key for them here.
    pub unreadable: BTreeSet<OpHash>,
    pub rejected: BTreeMap<OpHash, KnowledgeRejection>,
    pub pending: BTreeSet<OpHash>,
    pub forked: BTreeSet<DeviceId>,
    /// Groups of details in dispute with each other, each sorted.
    pub disputes: Vec<Vec<OpHash>>,
    /// The `deps` for this computer's next operation.
    pub heads: Vec<OpHash>,
    tips: BTreeMap<DeviceId, (u64, OpHash)>,
    lane: Option<(GroupId, LaneId)>,
    /// Which space this lane is (the whole group unless folded with `fold_space`).
    pub space: Space,
}

/// Every device that has ever been in the roster, with its person and names.
fn device_directory(roster: &GroupState) -> BTreeMap<DeviceId, (PersonId, String, String)> {
    roster
        .people
        .values()
        .flat_map(|member| {
            member.devices.iter().map(|(device, entry)| {
                (
                    *device,
                    (member.person, member.name.clone(), entry.name.clone()),
                )
            })
        })
        .collect()
}

/// Folds shared-knowledge operations for one lane against the current roster.
///
/// Details from computers that have left (`roster.former_devices`) are kept but
/// hidden from answers. Computers that were never in the group are rejected.
pub fn fold_knowledge<'a>(
    roster: &GroupState,
    lane: LaneId,
    ops: impl IntoIterator<Item = &'a KnowledgeOp>,
    verifier: &dyn SignatureVerifier,
) -> KnowledgeView {
    fold_knowledge_opened(roster, lane, ops, verifier, &BTreeMap::new())
}

/// [`fold_knowledge`] with the plaintext of sealed operations this computer could open
/// (`opened`, by operation hash). Sealed operations missing from it are kept, counted
/// as applied for ordering, and listed in [`KnowledgeView::unreadable`].
pub fn fold_knowledge_opened<'a>(
    roster: &GroupState,
    lane: LaneId,
    ops: impl IntoIterator<Item = &'a KnowledgeOp>,
    verifier: &dyn SignatureVerifier,
    opened: &BTreeMap<OpHash, KnowledgeBody>,
) -> KnowledgeView {
    fold_lane(roster, lane, Space::Group, ops, verifier, opened)
}

/// Folds one space's lane: only operations in that lane are read (others are not
/// this view's business), and in a two-person space only those two may write.
pub fn fold_space<'a>(
    roster: &GroupState,
    space: Space,
    ops: impl IntoIterator<Item = &'a KnowledgeOp>,
    verifier: &dyn SignatureVerifier,
    opened: &BTreeMap<OpHash, KnowledgeBody>,
) -> KnowledgeView {
    let lane = space.lane(&roster.group);
    fold_lane(
        roster,
        lane,
        space,
        ops.into_iter().filter(|op| op.unsigned.lane == lane),
        verifier,
        opened,
    )
}

fn fold_lane<'a>(
    roster: &GroupState,
    lane: LaneId,
    space: Space,
    ops: impl IntoIterator<Item = &'a KnowledgeOp>,
    verifier: &dyn SignatureVerifier,
    opened: &BTreeMap<OpHash, KnowledgeBody>,
) -> KnowledgeView {
    let group = roster.group;
    let mut view = KnowledgeView {
        lane: Some((group, lane)),
        space,
        ..KnowledgeView::default()
    };
    let mut candidates: BTreeMap<OpHash, &KnowledgeOp> = BTreeMap::new();
    for op in ops {
        candidates.entry(op.hash()).or_insert(op);
    }
    for (hash, op) in &candidates {
        let unsigned = &op.unsigned;
        let rejection = if unsigned.group != group {
            Some(KnowledgeRejection::WrongGroup)
        } else if unsigned.lane != lane {
            Some(KnowledgeRejection::WrongLane)
        } else if !op.signed_by_author(verifier) {
            Some(KnowledgeRejection::BadSignature)
        } else {
            None
        };
        if let Some(rejection) = rejection {
            view.rejected.insert(*hash, rejection);
        }
    }
    let nodes: BTreeMap<OpHash, DagNode> = candidates
        .iter()
        .filter(|(hash, _)| !view.rejected.contains_key(*hash))
        .map(|(hash, op)| (*hash, op.node()))
        .collect();
    let order = dag::order(&nodes);
    for (hash, reason) in &order.rejected {
        view.rejected.insert(
            *hash,
            match reason {
                dag::DagReject::BrokenChain => KnowledgeRejection::BrokenChain,
                dag::DagReject::Forked => KnowledgeRejection::Forked,
                dag::DagReject::ParentRejected => KnowledgeRejection::ParentRejected,
            },
        );
    }
    view.pending.clone_from(&order.pending);
    view.forked.clone_from(&order.forked);
    view.tips.clone_from(&order.tips);

    let current = device_directory(roster);
    let mut applied: BTreeSet<OpHash> = BTreeSet::new();
    let mut agreements: Vec<(OpHash, PersonId)> = Vec::new();
    let mut withdrawals: Vec<OpHash> = Vec::new();
    let mut redactions: Vec<OpHash> = Vec::new();

    for hash in &order.order {
        let op = candidates[hash];
        let author = op.unsigned.author;
        let (person, shared_by, computer, former) = match current.get(&author) {
            Some((person, name, computer)) => (*person, name.clone(), computer.clone(), false),
            None => {
                if let Some((person, name)) = roster.former_devices.get(&author) {
                    (*person, name.clone(), String::new(), true)
                } else {
                    view.rejected
                        .insert(*hash, KnowledgeRejection::UnknownAuthor);
                    continue;
                }
            }
        };
        if !space.includes(roster, &person) {
            view.rejected.insert(*hash, KnowledgeRejection::NotAllowed);
            continue;
        }
        let body = match &op.unsigned.body {
            KnowledgeBody::Sealed { .. } => match opened.get(hash) {
                Some(KnowledgeBody::Sealed { .. } | KnowledgeBody::KeyGrant { .. }) => {
                    view.rejected
                        .insert(*hash, KnowledgeRejection::InvalidDetail);
                    continue;
                }
                Some(inner) => inner,
                None => {
                    view.unreadable.insert(*hash);
                    applied.insert(*hash);
                    continue;
                }
            },
            plain => plain,
        };
        // Viewers read but never add. Former members are judged by what they could do
        // then, which nobody recorded, so their additions are kept (hidden) instead.
        let adds = matches!(
            body,
            KnowledgeBody::Share { .. }
                | KnowledgeBody::ShareArtifact { .. }
                | KnowledgeBody::ShareTemplate { .. }
                | KnowledgeBody::Corroborate { .. }
        );
        if adds && !former && !roster.role_of(&author).is_some_and(super::Role::writes) {
            view.rejected.insert(*hash, KnowledgeRejection::NotAllowed);
            continue;
        }
        let result = match body {
            // Kept even from someone who has since left: removing a person must not
            // make what the group already shared under their key unreadable. Their key
            // is retired for new content by `LaneKey::usable` instead.
            KnowledgeBody::KeyGrant { key, grants } => {
                let entry = view.keys.entry(*key).or_default();
                entry.created_at = Some(
                    entry
                        .created_at
                        .map_or(op.unsigned.at, |at| at.min(op.unsigned.at)),
                );
                for (device, sealed) in grants {
                    entry
                        .grants
                        .entry(*device)
                        .or_insert_with(|| sealed.clone());
                }
                Ok(())
            }
            KnowledgeBody::Sealed { .. } => Err(KnowledgeRejection::InvalidDetail),
            KnowledgeBody::ShareTemplate { template, replaces } => {
                if super::template::check_template(template).is_err() {
                    Err(KnowledgeRejection::InvalidDetail)
                } else if replaces
                    .iter()
                    .any(|old| !view.templates.contains_key(old) || !order.saw(hash, old))
                {
                    Err(KnowledgeRejection::UnknownDetail)
                } else {
                    view.templates.insert(
                        *hash,
                        super::template::SharedTemplate {
                            id: *hash,
                            template: template.clone(),
                            author,
                            person: Some(person),
                            shared_by: shared_by.clone(),
                            computer: computer.clone(),
                            shared_at: op.unsigned.at,
                            replaces: replaces.clone(),
                            status: DetailStatus::Current,
                            from_former_member: former,
                            space,
                        },
                    );
                    Ok(())
                }
            }
            KnowledgeBody::ShareArtifact {
                title,
                kind,
                language,
                content,
                replaces,
                origin,
            } => {
                if check_artifact(title, language, content).is_err()
                    || origin.chars().count() > MAX_ORIGIN_CHARS
                {
                    Err(KnowledgeRejection::InvalidDetail)
                } else if replaces
                    .iter()
                    .any(|old| !view.artifacts.contains_key(old) || !order.saw(hash, old))
                {
                    Err(KnowledgeRejection::UnknownDetail)
                } else {
                    view.artifacts.insert(
                        *hash,
                        SharedArtifact {
                            id: *hash,
                            title: title.trim().to_owned(),
                            kind: *kind,
                            language: language.trim().to_owned(),
                            content: content.clone(),
                            author,
                            person: Some(person),
                            shared_by: shared_by.clone(),
                            computer: computer.clone(),
                            shared_at: op.unsigned.at,
                            replaces: replaces.clone(),
                            status: DetailStatus::Current,
                            from_former_member: former,
                            origin: origin.clone(),
                            space,
                        },
                    );
                    Ok(())
                }
            }
            KnowledgeBody::Share {
                text,
                about,
                valid_at,
                replaces,
            } => {
                if check_share(text, about).is_err() {
                    Err(KnowledgeRejection::InvalidDetail)
                } else if replaces
                    .iter()
                    .any(|old| !view.details.contains_key(old) || !order.saw(hash, old))
                {
                    Err(KnowledgeRejection::UnknownDetail)
                } else {
                    view.details.insert(
                        *hash,
                        SharedDetail {
                            id: *hash,
                            text: text.trim().to_owned(),
                            about: about.clone(),
                            author,
                            person: Some(person),
                            shared_by,
                            computer,
                            shared_at: op.unsigned.at,
                            valid_at: *valid_at,
                            replaces: replaces.clone(),
                            status: DetailStatus::Current,
                            agreed_by: BTreeSet::new(),
                            from_former_member: former,
                            space,
                        },
                    );
                    Ok(())
                }
            }
            KnowledgeBody::Corroborate { detail } => match view.details.get(detail) {
                None => Err(KnowledgeRejection::UnknownDetail),
                Some(target) if target.person == Some(person) => {
                    Err(KnowledgeRejection::NotAllowed)
                }
                Some(_) => {
                    if !former {
                        agreements.push((*detail, person));
                    }
                    Ok(())
                }
            },
            KnowledgeBody::Withdraw { detail } | KnowledgeBody::Redact { detail } => {
                let admin = roster.role_of(&author).is_some_and(super::Role::manages);
                let owner = view
                    .details
                    .get(detail)
                    .map(|target| target.person)
                    .or_else(|| view.artifacts.get(detail).map(|target| target.person))
                    .or_else(|| view.templates.get(detail).map(|target| target.person));
                match owner {
                    None => Err(KnowledgeRejection::UnknownDetail),
                    Some(owner) if owner != Some(person) && !admin => {
                        Err(KnowledgeRejection::NotAllowed)
                    }
                    Some(_) => {
                        if matches!(body, KnowledgeBody::Redact { .. }) {
                            redactions.push(*detail);
                        } else {
                            withdrawals.push(*detail);
                        }
                        Ok(())
                    }
                }
            }
        };
        match result {
            Ok(()) => {
                applied.insert(*hash);
            }
            Err(rejection) => {
                view.rejected.insert(*hash, rejection);
            }
        }
    }

    view.disputes = settle_replacements(&mut view.details, &order);
    settle_replacements(&mut view.artifacts, &order);
    settle_replacements(&mut view.templates, &order);
    for (detail, person) in agreements {
        if let Some(target) = view.details.get_mut(&detail) {
            target.agreed_by.insert(person);
        }
    }
    for id in withdrawals {
        if let Some(target) = view.details.get_mut(&id)
            && !matches!(target.status, DetailStatus::Redacted)
        {
            target.status = DetailStatus::Withdrawn;
        }
        if let Some(target) = view.artifacts.get_mut(&id)
            && !matches!(target.status, DetailStatus::Redacted)
        {
            target.status = DetailStatus::Withdrawn;
        }
        if let Some(target) = view.templates.get_mut(&id)
            && !matches!(target.status, DetailStatus::Redacted)
        {
            target.status = DetailStatus::Withdrawn;
        }
    }
    for id in redactions {
        if let Some(target) = view.details.get_mut(&id) {
            target.text.clear();
            target.about.clear();
            target.status = DetailStatus::Redacted;
        }
        if let Some(target) = view.artifacts.get_mut(&id) {
            target.title.clear();
            target.content.clear();
            target.status = DetailStatus::Redacted;
        }
        if let Some(target) = view.templates.get_mut(&id) {
            target.erase();
            target.status = DetailStatus::Redacted;
        }
    }
    view.heads = order.heads(&nodes, &applied);
    view
}

/// Something shared that newer shares can replace: a detail or an artifact version.
trait Versioned {
    fn replaces(&self) -> &[OpHash];
    fn is_from_former_member(&self) -> bool;
    fn set_status(&mut self, status: DetailStatus);
    /// Two replacements that say the same thing agree rather than conflict.
    fn same_as(&self, other: &Self) -> bool;
    /// Which side of a dispute is shown first: later `valid_at`, then later share.
    fn preference(&self) -> (Option<Timestamp>, Timestamp);
}

impl Versioned for SharedDetail {
    fn replaces(&self) -> &[OpHash] {
        &self.replaces
    }
    fn is_from_former_member(&self) -> bool {
        self.from_former_member
    }
    fn set_status(&mut self, status: DetailStatus) {
        self.status = status;
    }
    fn same_as(&self, other: &Self) -> bool {
        same_claim(self, other)
    }
    fn preference(&self) -> (Option<Timestamp>, Timestamp) {
        (self.valid_at, self.shared_at)
    }
}

impl Versioned for super::template::SharedTemplate {
    fn replaces(&self) -> &[OpHash] {
        &self.replaces
    }
    fn is_from_former_member(&self) -> bool {
        self.from_former_member
    }
    fn set_status(&mut self, status: DetailStatus) {
        self.status = status;
    }
    fn same_as(&self, other: &Self) -> bool {
        self.template == other.template
    }
    fn preference(&self) -> (Option<Timestamp>, Timestamp) {
        (None, self.shared_at)
    }
}

impl Versioned for SharedArtifact {
    fn replaces(&self) -> &[OpHash] {
        &self.replaces
    }
    fn is_from_former_member(&self) -> bool {
        self.from_former_member
    }
    fn set_status(&mut self, status: DetailStatus) {
        self.status = status;
    }
    fn same_as(&self, other: &Self) -> bool {
        self.title == other.title && self.content == other.content
    }
    fn preference(&self) -> (Option<Timestamp>, Timestamp) {
        (None, self.shared_at)
    }
}

/// Marks replaced items, and disputes where replacements did not see each other.
/// Returns the groups of items in dispute, each sorted.
fn settle_replacements<T: Versioned>(
    items: &mut BTreeMap<OpHash, T>,
    order: &dag::DagOrder,
) -> Vec<Vec<OpHash>> {
    // Which shares replace each old item.
    let mut replaced_by: BTreeMap<OpHash, Vec<OpHash>> = BTreeMap::new();
    for (id, item) in items.iter() {
        if !item.is_from_former_member() {
            for old in item.replaces() {
                replaced_by.entry(*old).or_default().push(*id);
            }
        }
    }
    let mut disputed: BTreeMap<OpHash, BTreeSet<OpHash>> = BTreeMap::new();
    for (old, replacements) in &replaced_by {
        if let Some(target) = items.get_mut(old) {
            target.set_status(DetailStatus::Replaced {
                by: replacements.clone(),
            });
        }
        // Siblings that differ and did not see each other are in dispute, unless a
        // later replacement already retired them.
        for a in replacements {
            for b in replacements {
                if a < b && order.concurrent(a, b) && !items[a].same_as(&items[b]) {
                    disputed.entry(*a).or_default().insert(*b);
                    disputed.entry(*b).or_default().insert(*a);
                }
            }
        }
    }
    let mut groups: BTreeSet<Vec<OpHash>> = BTreeSet::new();
    for (id, others) in &disputed {
        if replaced_by.contains_key(id) {
            continue;
        }
        let mut members: Vec<OpHash> = others
            .iter()
            .copied()
            .filter(|other| !replaced_by.contains_key(other))
            .collect();
        if members.is_empty() {
            continue;
        }
        members.push(*id);
        members.sort();
        groups.insert(members);
    }
    for members in &groups {
        let chosen = members
            .iter()
            .copied()
            .max_by_key(|id| (items[id].preference(), *id))
            .expect("a dispute has at least two items");
        for id in members {
            let with = members
                .iter()
                .copied()
                .filter(|other| other != id)
                .collect();
            if let Some(item) = items.get_mut(id) {
                item.set_status(DetailStatus::Disputed {
                    with,
                    chosen: *id == chosen,
                });
            }
        }
    }
    groups.into_iter().collect()
}

/// Corrections that say the same thing agree rather than conflict.
fn same_claim(a: &SharedDetail, b: &SharedDetail) -> bool {
    normalise(&a.text) == normalise(&b.text) && a.valid_at == b.valid_at
}

fn normalise(text: &str) -> String {
    terms(text).collect::<Vec<_>>().join(" ")
}

/// Lowercase alphanumeric words; the shared tokenizer for matching and ranking.
fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Words that carry no meaning on their own, so a question's "is" or "the" never
/// matches every detail.
const STOP_WORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did",
    "do", "does", "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it",
    "its", "me", "my", "of", "on", "or", "our", "s", "so", "that", "the", "their", "them", "there",
    "they", "this", "to", "us", "was", "we", "were", "what", "when", "where", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

/// A light English stemmer so "serviced", "service" and "servicing" meet. It only
/// trims common endings from longer words; it never needs to be linguistically exact,
/// just the same on both sides of the match.
fn stem(word: &str) -> String {
    let mut stem = word;
    if stem.chars().count() > 4 {
        for (suffix, keep) in [("ies", "y"), ("ing", ""), ("ed", ""), ("es", ""), ("s", "")] {
            if let Some(base) = stem.strip_suffix(suffix)
                && (suffix != "s" || !base.ends_with('s'))
            {
                let mut out = base.to_owned();
                out.push_str(keep);
                return trim_e(&out);
            }
        }
        stem = stem.strip_suffix('e').unwrap_or(stem);
    }
    stem.to_owned()
}

fn trim_e(word: &str) -> String {
    if word.chars().count() > 4 {
        word.strip_suffix('e').unwrap_or(word).to_owned()
    } else {
        word.to_owned()
    }
}

/// Searchable keys for text: meaningful words, stemmed.
fn keys(text: &str) -> impl Iterator<Item = String> + '_ {
    terms(text)
        .filter(|word| word.chars().count() > 1 && !STOP_WORDS.contains(&word.as_str()))
        .map(|word| stem(&word))
}

/// BM25 (k1 1.2, b 0.75) of each document's words against the query terms.
fn bm25(query: &BTreeSet<String>, docs: &[Vec<String>]) -> Vec<f32> {
    const K1: f32 = 1.2;
    const B: f32 = 0.75;
    if docs.is_empty() {
        return Vec::new();
    }
    #[expect(clippy::cast_precision_loss, reason = "document counts are small")]
    let total = docs.len() as f32;
    #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
    let average = docs.iter().map(Vec::len).sum::<usize>() as f32 / total;
    docs.iter()
        .map(|words| {
            #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
            let length = words.len() as f32;
            query
                .iter()
                .map(|term| {
                    #[expect(clippy::cast_precision_loss, reason = "counts are small")]
                    let frequency = words.iter().filter(|w| *w == term).count() as f32;
                    if frequency == 0.0 {
                        return 0.0;
                    }
                    #[expect(clippy::cast_precision_loss, reason = "counts are small")]
                    let containing = docs.iter().filter(|w| w.contains(term)).count() as f32;
                    let idf = ((total - containing + 0.5) / (containing + 0.5) + 1.0).ln();
                    idf * frequency * (K1 + 1.0)
                        / (frequency + K1 * (1.0 - B + B * length / average.max(1.0)))
                })
                .sum()
        })
        .collect()
}

/// One shared detail ranked for a question.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedHit {
    pub detail: SharedDetail,
    pub score: f32,
}

impl KnowledgeView {
    /// Details helpers may use, newest first.
    pub fn usable(&self) -> impl Iterator<Item = &SharedDetail> {
        let mut details: Vec<&SharedDetail> = self
            .details
            .values()
            .filter(|detail| detail.usable())
            .collect();
        details.sort_by(|a, b| b.shared_at.cmp(&a.shared_at).then(b.id.cmp(&a.id)));
        details.into_iter()
    }

    /// Ranks usable details for `question` with BM25 (k1 1.2, b 0.75) over text and
    /// topics, then weights agreement up and unresolved disputes down.
    pub fn search(&self, question: &str, limit: usize) -> Vec<SharedHit> {
        let query: BTreeSet<String> = keys(question).collect();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let details: Vec<&SharedDetail> = self.usable().collect();
        let docs: Vec<Vec<String>> = details
            .iter()
            .map(|detail| {
                let mut words: Vec<String> = keys(&detail.text).collect();
                for topic in &detail.about {
                    words.extend(keys(topic));
                }
                words
            })
            .collect();
        let mut hits: Vec<SharedHit> = details
            .iter()
            .zip(bm25(&query, &docs))
            .filter(|(_, score)| *score > 0.0)
            .map(|(detail, score)| {
                #[expect(clippy::cast_precision_loss, reason = "agreement counts are small")]
                let agreement = 1.0 + 0.25 * detail.agreed_by.len().min(4) as f32;
                let dispute = match detail.status {
                    DetailStatus::Disputed { chosen: true, .. } => 0.8,
                    DetailStatus::Disputed { chosen: false, .. } => 0.6,
                    _ => 1.0,
                };
                SharedHit {
                    detail: (*detail).clone(),
                    score: score * agreement * dispute,
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| b.detail.shared_at.cmp(&a.detail.shared_at))
                .then_with(|| a.detail.id.cmp(&b.detail.id))
        });
        hits.truncate(limit);
        hits
    }

    /// Ranks the latest shared artifacts for `question` with the same BM25 as details,
    /// over the title (counted twice) and the start of the content. Best first.
    pub fn search_artifacts(&self, question: &str, limit: usize) -> Vec<&SharedArtifact> {
        /// Long artifacts are ranked by their opening, which is what they are about.
        const RANKED_CHARS: usize = 20_000;
        let query: BTreeSet<String> = keys(question).collect();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let artifacts: Vec<&SharedArtifact> = self
            .artifacts
            .values()
            .filter(|artifact| artifact.is_latest())
            .collect();
        let docs: Vec<Vec<String>> = artifacts
            .iter()
            .map(|artifact| {
                let opening: String = artifact.content.chars().take(RANKED_CHARS).collect();
                let mut words: Vec<String> = keys(&artifact.title).collect();
                words.extend(keys(&artifact.title));
                words.extend(keys(&opening));
                words
            })
            .collect();
        let mut hits: Vec<(&SharedArtifact, f32)> = artifacts
            .into_iter()
            .zip(bm25(&query, &docs))
            .filter(|(_, score)| *score > 0.0)
            .collect();
        hits.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| b.0.shared_at.cmp(&a.0.shared_at))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        hits.into_iter()
            .take(limit)
            .map(|(artifact, _)| artifact)
            .collect()
    }

    /// Shared helpers and schedules, latest versions first, newest first.
    pub fn shared_templates(&self) -> Vec<super::template::SharedTemplate> {
        let mut templates: Vec<_> = self.templates.values().cloned().collect();
        templates.sort_by(|a, b| {
            b.is_latest()
                .cmp(&a.is_latest())
                .then(b.shared_at.cmp(&a.shared_at))
                .then(b.id.cmp(&a.id))
        });
        templates
    }

    /// Shared artifacts, newest first: the latest version of each (both, when two
    /// were made at once), then older and withdrawn versions.
    pub fn shared_artifacts(&self) -> Vec<SharedArtifact> {
        let mut artifacts: Vec<SharedArtifact> = self.artifacts.values().cloned().collect();
        artifacts.sort_by(|a, b| {
            b.is_latest()
                .cmp(&a.is_latest())
                .then(b.shared_at.cmp(&a.shared_at))
                .then(b.id.cmp(&a.id))
        });
        artifacts
    }

    /// What a new version of `artifact` should replace: the latest versions that grew
    /// from it, so sharing an edit of an old copy never forks the history. Empty when
    /// the artifact is unknown here (then the edit is shared as a new artifact).
    pub fn latest_versions_of(&self, artifact: &OpHash) -> Vec<OpHash> {
        if !self.artifacts.contains_key(artifact) {
            return Vec::new();
        }
        let mut latest: Vec<OpHash> = self
            .artifacts
            .values()
            .filter(|candidate| {
                !matches!(
                    candidate.status,
                    DetailStatus::Replaced { .. } | DetailStatus::Redacted
                ) && candidate.lineage(&self.artifacts).contains(artifact)
            })
            .map(|candidate| candidate.id)
            .collect();
        if latest.is_empty() {
            latest.push(*artifact);
        }
        latest.sort();
        latest
    }

    /// The newest lane key that may encrypt new content in this view's space.
    pub fn usable_key(&self, roster: &GroupState) -> Option<OpHash> {
        super::lane_keys::usable_key(&self.keys, &self.space.audience(roster))
    }

    /// The next operation this computer should sign in this lane.
    pub fn draft(
        &self,
        author: DeviceId,
        at: Timestamp,
        body: KnowledgeBody,
    ) -> Option<UnsignedKnowledge> {
        let (group, lane) = self.lane?;
        let (seq, prev) = match self.tips.get(&author) {
            Some(&(seq, hash)) => (seq + 1, Some(hash)),
            None => (1, None),
        };
        let deps = self
            .heads
            .iter()
            .copied()
            .filter(|head| Some(*head) != prev)
            .collect();
        Some(UnsignedKnowledge {
            group,
            lane,
            author,
            seq,
            prev,
            deps,
            at,
            body,
        })
    }
}
