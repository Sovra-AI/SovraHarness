//! Ledger, witness and residency behaviour across computers.

use std::collections::BTreeSet;

use super::department::{Department, DepartmentId, Directory};
use super::ledger::{
    Appended, AuditEvent, AuditLedger, Cosignature, ItemKind, LedgerEntry, LedgerError,
    SignedCheckpoint, UnsignedEntry, Witness, WitnessError, active_model, custody_violations,
    verify_entry,
};
use super::testing::{FakeVerifier, fake_sign};
use crate::Timestamp;
use crate::group::{ContentHash, DeviceId, PersonId};

const ORIGIN: &str = "acme/audit";

fn device(n: u8) -> DeviceId {
    DeviceId([n; 32])
}

fn finance() -> DepartmentId {
    DepartmentId::new("finance").expect("valid slug")
}

fn entry(ledger: &AuditLedger, author: DeviceId, event: AuditEvent) -> LedgerEntry {
    let (seq, prev) = ledger.next_for(&author);
    let body = UnsignedEntry {
        origin: ORIGIN.into(),
        author,
        seq,
        prev,
        at: Timestamp(i64::try_from(seq).expect("small")),
        event,
    };
    let signature = fake_sign(&author, &body.signing_bytes());
    body.sign_with(signature)
}

fn stored(item: &[u8], on: DeviceId) -> AuditEvent {
    AuditEvent::Stored {
        item: ContentHash::of(item),
        kind: ItemKind::Document,
        department: finance(),
        device: on,
    }
}

fn sign_checkpoint(ledger: &AuditLedger, sequencer: DeviceId) -> SignedCheckpoint {
    let checkpoint = ledger.checkpoint();
    SignedCheckpoint {
        signature: fake_sign(&sequencer, &checkpoint.signing_bytes()),
        signer: sequencer,
        checkpoint,
        cosignatures: Vec::new(),
    }
}

#[test]
fn entries_chain_per_computer_and_reject_tampering() {
    let mut ledger = AuditLedger::new(ORIGIN);
    let first = entry(&ledger, device(1), stored(b"q3 budget", device(1)));
    assert_eq!(
        ledger.append(first.clone(), &FakeVerifier),
        Ok(Appended::New(0))
    );
    assert_eq!(
        ledger.append(first.clone(), &FakeVerifier),
        Ok(Appended::Known(0))
    );

    let mut forged = entry(&ledger, device(1), stored(b"other", device(1)));
    forged.body.event = stored(b"swapped after signing", device(1));
    assert_eq!(
        ledger.append(forged, &FakeVerifier),
        Err(LedgerError::BadSignature)
    );

    // Skipping ahead in a computer's own history is refused.
    let mut skipped = UnsignedEntry {
        seq: 3,
        ..first.body.clone()
    };
    skipped.prev = Some(first.hash());
    let skipped = skipped
        .clone()
        .sign_with(fake_sign(&device(1), &skipped.signing_bytes()));
    assert_eq!(
        ledger.append(skipped, &FakeVerifier),
        Err(LedgerError::OutOfOrder { expected: 2 })
    );

    let mut other_ledger = first.body;
    other_ledger.origin = "evil/audit".into();
    let other_ledger = other_ledger
        .clone()
        .sign_with(fake_sign(&device(1), &other_ledger.signing_bytes()));
    assert_eq!(
        ledger.append(other_ledger, &FakeVerifier),
        Err(LedgerError::WrongOrigin)
    );
}

#[test]
fn an_auditor_checks_one_entry_against_a_checkpoint() {
    let mut ledger = AuditLedger::new(ORIGIN);
    for n in 0..7u8 {
        let e = entry(&ledger, device(n % 3), stored(&[n], device(n % 3)));
        ledger.append(e, &FakeVerifier).expect("valid");
    }
    let checkpoint = ledger.checkpoint();
    for (index, e) in ledger.entries().iter().enumerate() {
        let index = index as u64;
        let proof = ledger.inclusion_proof(index).expect("inside");
        assert!(verify_entry(e, index, &proof, &checkpoint));
    }
    let restored =
        AuditLedger::restore(ORIGIN, ledger.entries().to_vec(), &FakeVerifier).expect("valid");
    assert_eq!(restored.checkpoint(), checkpoint);
}

#[test]
fn witnesses_cosign_extensions_and_catch_forks_and_rollbacks() {
    let sequencer = device(10);
    let auditor = device(20);
    let mut ledger = AuditLedger::new(ORIGIN);
    let mut witness = Witness::new(ORIGIN, sequencer);
    for n in 0..3u8 {
        let e = entry(&ledger, device(1), stored(&[n], device(1)));
        ledger.append(e, &FakeVerifier).expect("valid");
    }
    let first = sign_checkpoint(&ledger, sequencer);
    witness
        .observe(&first, &[], &FakeVerifier)
        .expect("first checkpoint");

    // An honest extension is co-signed, and the co-signature verifies with quorum 1.
    let fork_base = ledger.clone();
    for n in 3..8u8 {
        let e = entry(&ledger, device(2), stored(&[n], device(2)));
        ledger.append(e, &FakeVerifier).expect("valid");
    }
    let mut second = sign_checkpoint(&ledger, sequencer);
    let proof = ledger.consistency_proof(3).expect("prefix");
    let bytes = witness
        .observe(&second, &proof, &FakeVerifier)
        .expect("extends");
    second.cosignatures.push(Cosignature {
        witness: auditor,
        signature: fake_sign(&auditor, &bytes),
    });
    assert!(second.verify(&sequencer, &BTreeSet::from([auditor]), 1, &FakeVerifier));
    assert!(!second.verify(&sequencer, &BTreeSet::from([device(21)]), 1, &FakeVerifier));

    // A sequencer that shows another history of the same length is caught.
    let mut forked = fork_base;
    for n in 30..35u8 {
        let e = entry(&forked, device(3), stored(&[n], device(3)));
        forked.append(e, &FakeVerifier).expect("valid");
    }
    let forked_checkpoint = sign_checkpoint(&forked, sequencer);
    let mut late_witness = witness.clone();
    assert_eq!(
        late_witness.observe(
            &forked_checkpoint,
            &forked.consistency_proof(8).unwrap_or_default(),
            &FakeVerifier
        ),
        Err(WitnessError::Fork)
    );
    // A shorter history is a rollback.
    assert_eq!(
        witness.observe(&first, &[], &FakeVerifier),
        Err(WitnessError::Rollback)
    );
    // A checkpoint someone else signed is not trusted.
    let impostor = sign_checkpoint(&ledger, device(66));
    assert_eq!(
        witness.observe(&impostor, &[], &FakeVerifier),
        Err(WitnessError::Untrusted)
    );
}

#[test]
fn replicas_take_entries_only_with_a_matching_checkpoint() {
    let sequencer = device(10);
    let mut primary = AuditLedger::new(ORIGIN);
    let mut replica = AuditLedger::new(ORIGIN);
    for n in 0..4u8 {
        let e = entry(&primary, device(1), stored(&[n], device(1)));
        primary.append(e, &FakeVerifier).expect("valid");
    }
    let checkpoint = sign_checkpoint(&primary, sequencer);
    // Withholding an entry does not match the checkpoint, and nothing is kept.
    let partial = primary.entries()[..3].to_vec();
    assert_eq!(
        replica.extend_verified(partial, &checkpoint, &sequencer, &FakeVerifier),
        Err(LedgerError::CheckpointMismatch)
    );
    assert!(replica.is_empty());
    replica
        .extend_verified(
            primary.entries().to_vec(),
            &checkpoint,
            &sequencer,
            &FakeVerifier,
        )
        .expect("matches");
    assert_eq!(replica.checkpoint(), primary.checkpoint());
}

#[test]
fn the_ledger_proves_where_every_item_resides() {
    let mut directory = Directory::default();
    let mut finance_dept = Department::new(finance(), "Finance");
    finance_dept
        .members
        .extend([PersonId([1; 32]), PersonId([2; 32])]);
    directory.add(finance_dept);
    directory.devices.extend([
        (device(1), PersonId([1; 32])),
        (device(2), PersonId([2; 32])),
        (device(3), PersonId([3; 32])),
    ]);

    let item = ContentHash::of(b"payroll.xlsx");
    let mut ledger = AuditLedger::new(ORIGIN);
    let events = [
        stored(b"payroll.xlsx", device(1)),
        AuditEvent::Replicated {
            item,
            department: finance(),
            to: device(2),
        },
        AuditEvent::Read {
            item,
            department: finance(),
            person: PersonId([2; 32]),
            helper: Some("Analyst".into()),
        },
        AuditEvent::Purged {
            item,
            department: finance(),
            device: device(1),
        },
    ];
    for event in events {
        let e = entry(&ledger, device(1), event);
        ledger.append(e, &FakeVerifier).expect("valid");
    }
    let report = ledger.residency();
    let where_ = &report[&item];
    assert_eq!(where_.holders, BTreeSet::from([device(2)]));
    assert_eq!(where_.purged_from, BTreeSet::from([device(1)]));
    assert_eq!(where_.reads, 1);
    assert_eq!(custody_violations(&report, &directory), []);

    // A copy onto a computer outside finance shows up as a violation.
    let leak = entry(
        &ledger,
        device(2),
        AuditEvent::Replicated {
            item,
            department: finance(),
            to: device(3),
        },
    );
    ledger.append(leak, &FakeVerifier).expect("valid");
    assert_eq!(
        custody_violations(&ledger.residency(), &directory),
        [(item, device(3))]
    );
}

#[test]
fn model_lineage_follows_promotions_and_rollbacks() {
    let (a, b) = (ContentHash::of(b"model-a"), ContentHash::of(b"model-b"));
    let approver = PersonId([1; 32]);
    let approvers = BTreeSet::from([approver]);
    let events = [
        AuditEvent::ModelPromoted {
            model: a,
            approvers: approvers.clone(),
        },
        AuditEvent::ModelPromoted {
            model: b,
            approvers,
        },
        AuditEvent::ModelRolledBack {
            from: b,
            to: a,
            approver,
        },
    ];
    assert_eq!(active_model(&events[..0]), None);
    assert_eq!(active_model(&events[..2]), Some(b));
    assert_eq!(active_model(&events), Some(a));
}
