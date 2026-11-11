//! The group's audit history on this computer: which group events become ledger
//! entries, and the plain-words summary the Groups screen shows.
//!
//! Every computer keeps its own signed ledger (origin `sovra-audit/<device>`). Today
//! it records what this computer shared and which memories it gave out in answer
//! to someone else's question; the payloads are never in it, only BLAKE3 hashes.
//! Syncing ledgers and witness co-signing between computers come next (see
//! `docs/swarm.md`).

use serde::{Deserialize, Serialize};

use super::department::DepartmentId;
use super::ledger::{AuditEvent, AuditLedger, ItemKind};
use crate::Timestamp;
use crate::group::{ContentHash, DeviceId, KnowledgeBody, LaneId};

/// How many recent lines the summary carries.
pub const RECENT_AUDIT_LINES: usize = 8;

/// The ledger name for one computer's own history.
pub fn origin_for(device: &DeviceId) -> String {
    format!("sovra-audit/{}", device.to_hex())
}

/// A lane as a department id ("lane-3f2a9c01e4b7"), so group spaces and company
/// departments share one custody vocabulary.
pub fn lane_department(lane: &LaneId) -> DepartmentId {
    let hex = lane.0.to_hex();
    DepartmentId::new(format!("lane-{}", &hex[..12])).expect("lane slugs are lowercase hex")
}

/// What a share stores, for the ledger: the content's hash and kind. `None` for
/// changes that store nothing new (corrections, withdrawals, keys).
pub fn shared_item(body: &KnowledgeBody) -> Option<(ContentHash, ItemKind)> {
    match body {
        KnowledgeBody::Share { text, .. } => {
            Some((ContentHash::of(text.as_bytes()), ItemKind::Memory))
        }
        KnowledgeBody::ShareArtifact { content, .. } => {
            Some((ContentHash::of(content.as_bytes()), ItemKind::Document))
        }
        KnowledgeBody::ShareTemplate { template, .. } => Some((
            ContentHash::of(&serde_json::to_vec(template).unwrap_or_default()),
            ItemKind::Document,
        )),
        KnowledgeBody::Corroborate { .. }
        | KnowledgeBody::Withdraw { .. }
        | KnowledgeBody::Redact { .. }
        | KnowledgeBody::KeyGrant { .. }
        | KnowledgeBody::Sealed { .. } => None,
    }
}

/// One line of history in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLine {
    pub at: Timestamp,
    pub words: String,
}

/// What the Groups screen shows about this computer's history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    pub records: u64,
    /// Every signature and link was re-checked when the app started.
    pub checked: bool,
    /// The first eight hex characters of the history's Merkle root, for comparing
    /// with an auditor's copy.
    pub fingerprint: String,
    pub shared: u64,
    /// Memories this computer gave out in answer to others' questions.
    pub given_out: u64,
    /// Newest first.
    pub recent: Vec<AuditLine>,
}

impl AuditSummary {
    /// The one line the Groups screen shows above the history.
    pub fn headline(&self) -> String {
        if !self.checked {
            return "This computer's saved history didn't check out: something changed it after it was written. Nothing new is added to it.".into();
        }
        match self.records {
            0 => "Nothing recorded yet. Everything you share, and every memory you give to someone's helpers, will be listed here.".into(),
            1 => format!("1 record, checked · fingerprint {}", self.fingerprint),
            n => format!("{n} records, all checked · fingerprint {}", self.fingerprint),
        }
    }
}

fn words(event: &AuditEvent) -> String {
    match event {
        AuditEvent::Stored { kind, .. } => match kind {
            ItemKind::Memory => "Shared a detail with the group".into(),
            ItemKind::Document => "Shared a document, helper or schedule".into(),
            ItemKind::VectorIndex => "Saved a search index".into(),
            ItemKind::Model => "Saved a model".into(),
        },
        AuditEvent::Replicated { to, .. } => format!("Copied to computer {}", to.fingerprint()),
        AuditEvent::Purged { .. } => "Erased the words (the record stays)".into(),
        AuditEvent::Read { helper, .. } => match helper {
            Some(by) => format!("Gave a memory to {by}"),
            None => "A memory was read".into(),
        },
        AuditEvent::EgressApproved { destination, .. } => {
            format!("Approved sending to {destination}")
        }
        AuditEvent::ModelRound {
            round,
            contributors,
            ..
        } => {
            format!("Training round {round} with {contributors} computers")
        }
        AuditEvent::ModelPromoted { .. } => "A new model was put into use".into(),
        AuditEvent::ModelRolledBack { .. } => "Switched back to an earlier model".into(),
    }
}

/// The summary for `ledger`. `checked` says whether its saved entries all verified.
pub fn summarize(ledger: &AuditLedger, checked: bool) -> AuditSummary {
    let entries = ledger.entries();
    let count =
        |f: fn(&AuditEvent) -> bool| entries.iter().filter(|e| f(&e.body.event)).count() as u64;
    AuditSummary {
        records: ledger.len(),
        checked,
        fingerprint: ledger.checkpoint().root.to_hex()[..8].to_owned(),
        shared: count(|e| matches!(e, AuditEvent::Stored { .. })),
        given_out: count(|e| matches!(e, AuditEvent::Read { .. })),
        recent: entries
            .iter()
            .rev()
            .take(RECENT_AUDIT_LINES)
            .map(|e| AuditLine {
                at: e.body.at,
                words: words(&e.body.event),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{OpHash, PersonId};
    use crate::swarm::ledger::UnsignedEntry;
    use crate::swarm::testing::{FakeVerifier, fake_sign};

    #[test]
    fn a_summary_counts_shares_and_answers_newest_first() {
        let me = DeviceId([1; 32]);
        let lane = LaneId(OpHash([7; 32]));
        let department = lane_department(&lane);
        assert!(department.as_str().starts_with("lane-0707"));
        let mut ledger = AuditLedger::new(origin_for(&me));
        let events = [
            AuditEvent::Stored {
                item: ContentHash::of(b"boiler"),
                kind: ItemKind::Memory,
                department: department.clone(),
                device: me,
            },
            AuditEvent::Read {
                item: ContentHash::of(b"dentist"),
                department,
                person: PersonId([2; 32]),
                helper: Some("Sam's helpers".into()),
            },
        ];
        for (n, event) in events.into_iter().enumerate() {
            let (seq, prev) = ledger.next_for(&me);
            let body = UnsignedEntry {
                origin: origin_for(&me),
                author: me,
                seq,
                prev,
                at: Timestamp(i64::try_from(n).expect("small")),
                event,
            };
            let signature = fake_sign(&me, &body.signing_bytes());
            ledger
                .append(body.sign_with(signature), &FakeVerifier)
                .expect("valid");
        }
        let summary = summarize(&ledger, true);
        assert_eq!(
            (summary.records, summary.shared, summary.given_out),
            (2, 1, 1)
        );
        assert_eq!(summary.recent[0].words, "Gave a memory to Sam's helpers");
        assert_eq!(summary.fingerprint.len(), 8);
        assert!(summary.headline().starts_with("2 records, all checked"));
        let empty = summarize(&AuditLedger::new("x"), true);
        assert_eq!(empty.records, 0);
        assert!(empty.headline().starts_with("Nothing recorded yet"));
        assert!(
            summarize(&ledger, false)
                .headline()
                .contains("didn't check out")
        );
    }
}
