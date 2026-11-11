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

/// The most entries one audit sync reply carries; the rest come on the next sync.
pub const MAX_AUDIT_REPLY_ENTRIES: usize = 500;

/// Verified copies of the other computers' audit histories, so the group shares one
/// append-only record and no single computer can quietly rewrite its past.
///
/// - Each computer's history may only be written by that computer: an entry's
///   origin must be [`origin_for`] its author.
/// - Every entry is signature- and chain-checked before it is kept.
/// - If a computer shows two different entries at the same place in its history,
///   it is marked as **forked**: someone rewrote it, and the screen says so.
/// - Entries are relayed: a computer passes on the copies it holds, so histories
///   reach computers that never met their author directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FleetAudit {
    ledgers: std::collections::BTreeMap<DeviceId, AuditLedger>,
    forked: std::collections::BTreeSet<DeviceId>,
}

impl FleetAudit {
    /// Rebuilds the copies from saved entries, re-checking each one.
    pub fn restore(
        me: &DeviceId,
        entries: Vec<super::ledger::LedgerEntry>,
        verifier: &dyn crate::group::SignatureVerifier,
    ) -> Self {
        let mut fleet = Self::default();
        fleet.merge(me, entries, verifier);
        fleet
    }

    /// How far this computer's copy of each history goes.
    pub fn heads(&self) -> std::collections::BTreeMap<DeviceId, u64> {
        self.ledgers
            .iter()
            .map(|(device, ledger)| (*device, ledger.len()))
            .collect()
    }

    /// Entries the asker lacks, from `own` (this computer's history) and every copy,
    /// at most [`MAX_AUDIT_REPLY_ENTRIES`].
    pub fn entries_after(
        &self,
        own: &AuditLedger,
        own_device: &DeviceId,
        have: &std::collections::BTreeMap<DeviceId, u64>,
    ) -> Vec<super::ledger::LedgerEntry> {
        let skip = |device: &DeviceId| {
            have.get(device)
                .and_then(|n| usize::try_from(*n).ok())
                .unwrap_or(0)
        };
        std::iter::once((own_device, own))
            .chain(self.ledgers.iter())
            .flat_map(|(device, ledger)| ledger.entries().iter().skip(skip(device)))
            .take(MAX_AUDIT_REPLY_ENTRIES)
            .cloned()
            .collect()
    }

    /// Keeps every entry that verifies and extends its author's history. Returns how
    /// many were new. Entries claiming to be from `me` are ignored: only this
    /// computer writes its own history.
    pub fn merge(
        &mut self,
        me: &DeviceId,
        entries: Vec<super::ledger::LedgerEntry>,
        verifier: &dyn crate::group::SignatureVerifier,
    ) -> usize {
        let mut added = 0;
        for entry in entries {
            let author = entry.body.author;
            if author == *me || entry.body.origin != origin_for(&author) {
                continue;
            }
            let ledger = self
                .ledgers
                .entry(author)
                .or_insert_with(|| AuditLedger::new(origin_for(&author)));
            let position = entry
                .body
                .seq
                .checked_sub(1)
                .and_then(|n| usize::try_from(n).ok());
            if let Some(existing) = position.and_then(|p| ledger.entries().get(p)) {
                if existing.hash() != entry.hash()
                    && verifier.verify(&author, &entry.body.signing_bytes(), &entry.signature)
                {
                    // Two signed entries for the same place: a rewritten history.
                    self.forked.insert(author);
                }
                continue;
            }
            if matches!(
                ledger.append(entry, verifier),
                Ok(super::ledger::Appended::New(_))
            ) {
                added += 1;
            }
        }
        self.ledgers.retain(|_, ledger| !ledger.is_empty());
        added
    }

    /// Every kept entry, for saving.
    pub fn all_entries(&self) -> Vec<super::ledger::LedgerEntry> {
        self.ledgers
            .values()
            .flat_map(|ledger| ledger.entries().iter().cloned())
            .collect()
    }

    pub fn computers(&self) -> u64 {
        self.ledgers.len() as u64
    }

    pub fn records(&self) -> u64 {
        self.ledgers.values().map(AuditLedger::len).sum()
    }

    pub fn forked(&self) -> &std::collections::BTreeSet<DeviceId> {
        &self.forked
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
    /// Other computers whose histories this computer holds checked copies of.
    pub other_computers: u64,
    pub other_records: u64,
    /// Fingerprints of computers caught showing two different histories.
    pub forked: Vec<String>,
}

impl AuditSummary {
    /// The line about the rest of the group's histories, if there is anything to say.
    pub fn fleet_line(&self) -> Option<String> {
        if !self.forked.is_empty() {
            return Some(format!(
                "Warning: {} showed two different histories. Someone changed what already happened there.",
                self.forked
                    .iter()
                    .map(|f| format!("computer {f}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        match self.other_computers {
            0 => None,
            1 => Some(format!(
                "Also holds a checked copy of 1 other computer's history ({} records).",
                self.other_records
            )),
            n => Some(format!(
                "Also holds checked copies of {n} other computers' histories ({} records).",
                self.other_records
            )),
        }
    }

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

/// The summary for this computer's `ledger` and its copies of the group's others.
/// `checked` says whether its own saved entries all verified.
pub fn summarize(ledger: &AuditLedger, checked: bool, fleet: &FleetAudit) -> AuditSummary {
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
        other_computers: fleet.computers(),
        other_records: fleet.records(),
        forked: fleet.forked().iter().map(DeviceId::fingerprint).collect(),
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
        let summary = summarize(&ledger, true, &FleetAudit::default());
        assert_eq!(
            (summary.records, summary.shared, summary.given_out),
            (2, 1, 1)
        );
        assert_eq!(summary.recent[0].words, "Gave a memory to Sam's helpers");
        assert_eq!(summary.fingerprint.len(), 8);
        assert!(summary.headline().starts_with("2 records, all checked"));
        let empty = summarize(&AuditLedger::new("x"), true, &FleetAudit::default());
        assert_eq!(empty.records, 0);
        assert!(empty.headline().starts_with("Nothing recorded yet"));
        assert!(
            summarize(&ledger, false, &FleetAudit::default())
                .headline()
                .contains("didn't check out")
        );
    }

    fn history(author: DeviceId, events: usize, salt: u8) -> AuditLedger {
        let mut ledger = AuditLedger::new(origin_for(&author));
        for n in 0..events {
            let (seq, prev) = ledger.next_for(&author);
            let body = UnsignedEntry {
                origin: origin_for(&author),
                author,
                seq,
                prev,
                at: Timestamp(1),
                event: AuditEvent::Stored {
                    item: ContentHash::of(&[salt, u8::try_from(n).expect("small")]),
                    kind: ItemKind::Memory,
                    department: DepartmentId::new("legal").expect("valid"),
                    device: author,
                },
            };
            let signature = fake_sign(&author, &body.signing_bytes());
            ledger
                .append(body.sign_with(signature), &FakeVerifier)
                .expect("valid");
        }
        ledger
    }

    #[test]
    fn computers_relay_each_others_histories_and_cannot_write_mine() {
        let (me, sam, ana) = (DeviceId([1; 32]), DeviceId([2; 32]), DeviceId([3; 32]));
        let mine = history(me, 2, 0);
        let sams = history(sam, 3, 0);
        // Sam's computer holds a copy of Ana's history and relays it.
        let anas = history(ana, 2, 0);
        let mut sams_copies = FleetAudit::default();
        sams_copies.merge(&sam, anas.entries().to_vec(), &FakeVerifier);

        let mut fleet = FleetAudit::default();
        let have = fleet.heads();
        let offered = sams_copies.entries_after(&sams, &sam, &have);
        assert_eq!(offered.len(), 5);
        assert_eq!(fleet.merge(&me, offered.clone(), &FakeVerifier), 5);
        assert_eq!((fleet.computers(), fleet.records()), (2, 5));
        // Offering again adds nothing, and asks only for what is new.
        assert_eq!(fleet.merge(&me, offered, &FakeVerifier), 0);
        assert!(sams_copies.entries_after(&sams, &sam, &fleet.heads()).len() <= 2);
        // Nobody else can put entries into my own history.
        assert_eq!(fleet.merge(&me, mine.entries().to_vec(), &FakeVerifier), 0);
        // An entry under someone else's ledger name is refused.
        let mut misplaced = sams.entries()[0].body.clone();
        misplaced.origin = origin_for(&ana);
        let misplaced = misplaced
            .clone()
            .sign_with(fake_sign(&sam, &misplaced.signing_bytes()));
        assert_eq!(fleet.merge(&me, vec![misplaced], &FakeVerifier), 0);
        // Restored from saved entries, every copy is re-checked and kept.
        let restored = FleetAudit::restore(&me, fleet.all_entries(), &FakeVerifier);
        assert_eq!(restored.records(), 5);
        assert!(restored.forked().is_empty());
    }

    #[test]
    fn a_computer_that_rewrites_its_history_is_caught() {
        let (me, sam) = (DeviceId([1; 32]), DeviceId([2; 32]));
        let mut fleet = FleetAudit::default();
        fleet.merge(&me, history(sam, 2, 0).entries().to_vec(), &FakeVerifier);
        // Sam's computer later shows a different, validly signed first entry.
        fleet.merge(&me, history(sam, 2, 9).entries().to_vec(), &FakeVerifier);
        assert!(fleet.forked().contains(&sam));
        let summary = summarize(&history(me, 0, 0), true, &fleet);
        assert!(
            summary
                .fleet_line()
                .expect("warning")
                .starts_with("Warning")
        );
        // A forged entry nobody signed is just ignored, not a fork.
        let mut other = FleetAudit::default();
        other.merge(&me, history(sam, 1, 0).entries().to_vec(), &FakeVerifier);
        let mut forged = history(sam, 1, 5).entries()[0].clone();
        forged.signature = crate::group::Signature([0; 64]);
        other.merge(&me, vec![forged], &FakeVerifier);
        assert!(other.forked().is_empty());
        assert!(
            summarize(&history(me, 0, 0), true, &other)
                .fleet_line()
                .expect("one copy")
                .contains("1 other computer")
        );
    }
}
