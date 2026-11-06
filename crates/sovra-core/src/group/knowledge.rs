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
use crate::Timestamp;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeOp {
    #[serde(flatten)]
    pub unsigned: UnsignedKnowledge,
    pub signature: Signature,
}

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
        }
    }
}

impl KnowledgeOp {
    pub fn hash(&self) -> OpHash {
        self.unsigned.hash()
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
    pub rejected: BTreeMap<OpHash, KnowledgeRejection>,
    pub pending: BTreeSet<OpHash>,
    pub forked: BTreeSet<DeviceId>,
    /// Groups of details in dispute with each other, each sorted.
    pub disputes: Vec<Vec<OpHash>>,
    /// The `deps` for this computer's next operation.
    pub heads: Vec<OpHash>,
    tips: BTreeMap<DeviceId, (u64, OpHash)>,
    lane: Option<(GroupId, LaneId)>,
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
    let group = roster.group;
    let mut view = KnowledgeView {
        lane: Some((group, lane)),
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
        } else if !verifier.verify(&unsigned.author, &unsigned.signing_bytes(), &op.signature) {
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
        let result = match &op.unsigned.body {
            KnowledgeBody::Share {
                text,
                about,
                valid_at,
                replaces,
            } => {
                if check_share(text, about).is_err() {
                    Err(KnowledgeRejection::InvalidDetail)
                } else if replaces.iter().any(|old| {
                    !view.details.contains_key(old) || !order.ancestors[hash].contains(old)
                }) {
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
                match view.details.get(detail) {
                    None => Err(KnowledgeRejection::UnknownDetail),
                    Some(target) if target.person != Some(person) && !admin => {
                        Err(KnowledgeRejection::NotAllowed)
                    }
                    Some(_) => {
                        if matches!(op.unsigned.body, KnowledgeBody::Redact { .. }) {
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

    settle_replacements(&mut view, &order);
    for (detail, person) in agreements {
        if let Some(target) = view.details.get_mut(&detail) {
            target.agreed_by.insert(person);
        }
    }
    for detail in withdrawals {
        if let Some(target) = view.details.get_mut(&detail)
            && !matches!(target.status, DetailStatus::Redacted)
        {
            target.status = DetailStatus::Withdrawn;
        }
    }
    for detail in redactions {
        if let Some(target) = view.details.get_mut(&detail) {
            target.text.clear();
            target.about.clear();
            target.status = DetailStatus::Redacted;
        }
    }
    view.heads = order.heads(&nodes, &applied);
    view
}

/// Marks replaced details, and disputes where corrections did not see each other.
fn settle_replacements(view: &mut KnowledgeView, order: &dag::DagOrder) {
    // Which shares replace each old detail.
    let mut replaced_by: BTreeMap<OpHash, Vec<OpHash>> = BTreeMap::new();
    for detail in view.details.values() {
        if !detail.from_former_member {
            for old in &detail.replaces {
                replaced_by.entry(*old).or_default().push(detail.id);
            }
        }
    }
    let mut disputed: BTreeMap<OpHash, BTreeSet<OpHash>> = BTreeMap::new();
    for (old, replacements) in &replaced_by {
        if let Some(target) = view.details.get_mut(old) {
            target.status = DetailStatus::Replaced {
                by: replacements.clone(),
            };
        }
        // Siblings that differ and did not see each other are in dispute, unless a
        // later correction already retired them.
        for a in replacements {
            for b in replacements {
                if a < b
                    && order.concurrent(a, b)
                    && !same_claim(&view.details[a], &view.details[b])
                {
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
            .max_by_key(|id| {
                let detail = &view.details[id];
                (detail.valid_at, detail.shared_at, *id)
            })
            .expect("a dispute has at least two details");
        for id in members {
            let with = members
                .iter()
                .copied()
                .filter(|other| other != id)
                .collect();
            if let Some(detail) = view.details.get_mut(id) {
                detail.status = DetailStatus::Disputed {
                    with,
                    chosen: *id == chosen,
                };
            }
        }
    }
    view.disputes = groups.into_iter().collect();
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
        const K1: f32 = 1.2;
        const B: f32 = 0.75;
        let query: BTreeSet<String> = keys(question).collect();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let docs: Vec<(&SharedDetail, Vec<String>)> = self
            .usable()
            .map(|detail| {
                let mut words: Vec<String> = keys(&detail.text).collect();
                for topic in &detail.about {
                    words.extend(keys(topic));
                }
                (detail, words)
            })
            .collect();
        if docs.is_empty() {
            return Vec::new();
        }
        #[expect(clippy::cast_precision_loss, reason = "detail counts are small")]
        let total = docs.len() as f32;
        #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
        let average = docs.iter().map(|(_, words)| words.len()).sum::<usize>() as f32 / total;
        let mut hits: Vec<SharedHit> = docs
            .iter()
            .filter_map(|(detail, words)| {
                #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
                let length = words.len() as f32;
                let score: f32 = query
                    .iter()
                    .map(|term| {
                        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
                        let containing =
                            docs.iter().filter(|(_, w)| w.contains(term)).count() as f32;
                        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
                        let frequency = words.iter().filter(|w| *w == term).count() as f32;
                        if frequency == 0.0 {
                            return 0.0;
                        }
                        let idf = ((total - containing + 0.5) / (containing + 0.5) + 1.0).ln();
                        idf * frequency * (K1 + 1.0)
                            / (frequency + K1 * (1.0 - B + B * length / average.max(1.0)))
                    })
                    .sum();
                (score > 0.0).then(|| {
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
