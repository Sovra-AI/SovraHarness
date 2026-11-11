//! A whole fleet trains together: three departments with conflicting data, every
//! upload clipped, noised, masked and signed off by a person, the sum unmasked
//! within a privacy budget, and every round on the audit ledger.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::swarm::department::{Department, TrainingConsent};
use crate::swarm::egress::EgressGate;
use crate::swarm::learn::privacy::epsilon_for;
use crate::swarm::learn::scaffold::tests::{
    WORK, departments as objectives, distance, global_optimum,
};
use crate::swarm::learn::scaffold::{ScaffoldClient, ScaffoldServer};
use crate::swarm::learn::secagg::MaskingClient;
use crate::swarm::ledger::{AuditLedger, UnsignedEntry, active_model};
use crate::swarm::testing::{FakeVerifier, SeededRandom, fake_sign};

const PER_DEPARTMENT: u8 = 10;
const DELTA: f64 = 1e-5;

fn device(n: u8) -> DeviceId {
    DeviceId([n; 32])
}

fn approver_of(department: usize) -> PersonId {
    PersonId([200 + u8::try_from(department).expect("small"); 32])
}

fn slug(i: usize) -> DepartmentId {
    DepartmentId::new(["legal", "finance", "engineering"][i]).expect("valid slug")
}

fn company() -> Directory {
    let mut directory = Directory::default();
    for d in 0..3 {
        let mut department = Department::new(slug(d), slug(d).as_str());
        department.training = TrainingConsent::ApprovedExamples;
        department.approvers.insert(approver_of(d));
        department.members.insert(approver_of(d));
        for k in 0..PER_DEPARTMENT {
            let n = u8::try_from(d).expect("small") * PER_DEPARTMENT + k + 1;
            let person = PersonId([n; 32]);
            department.members.insert(person);
            directory.devices.insert(device(n), person);
        }
        directory.add(department);
    }
    directory
}

fn department_of(n: u8) -> usize {
    usize::from((n - 1) / PER_DEPARTMENT)
}

fn pair_seed(a: u8, b: u8, round: u64) -> [u8; 32] {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[lo, hi]);
    hasher.update(&round.to_le_bytes());
    *hasher.finalize().as_bytes()
}

fn append(ledger: &mut AuditLedger, author: DeviceId, event: AuditEvent) {
    let (seq, prev) = ledger.next_for(&author);
    let body = UnsignedEntry {
        origin: ledger.origin.clone(),
        author,
        seq,
        prev,
        at: Timestamp(i64::try_from(seq).expect("small")),
        event,
    };
    let signature = fake_sign(&author, &body.signing_bytes());
    ledger
        .append(body.sign_with(signature), &FakeVerifier)
        .expect("valid entry");
}

#[test]
fn a_fleet_trains_a_shared_model_privately_with_sign_off_and_lineage() {
    let directory = company();
    let parts = objectives();
    let target = global_optimum(&parts);
    let all: Vec<u8> = (1..=3 * PER_DEPARTMENT).collect();
    let roster: BTreeSet<DeviceId> = all.iter().copied().map(device).collect();
    let departments: BTreeSet<DepartmentId> = (0..3).map(slug).collect();
    let rounds: u32 = 40;
    let mut budget = PrivacyBudget::for_epsilon(10.0, DELTA);
    let quantiser = Quantiser {
        clip: 2.0,
        scale: 4096.0,
    };
    let mut server = ScaffoldServer::new(vec![0.0; 3], all.len(), WORK);
    let mut clients: BTreeMap<u8, ScaffoldClient> =
        all.iter().map(|n| (*n, ScaffoldClient::new(3))).collect();
    let mut gates: BTreeMap<u8, EgressGate> =
        all.iter().map(|n| (*n, EgressGate::default())).collect();
    let mut ledger = AuditLedger::new("acme/audit");
    let coordinator = device(250);
    let mut rng = SeededRandom(2026);
    let start = distance(&server.params, &target);

    for round in 1..=u64::from(rounds) {
        let plan = RoundPlan::new(
            RoundSettings {
                round,
                base_model: model_hash(&server.params),
                departments: departments.clone(),
                roster: roster.clone(),
                dimension: 3,
                quantiser,
                assumed_colluders: 0,
                rounds_left: rounds - u32::try_from(round).expect("small") + 1,
            },
            &budget,
            &directory,
        )
        .expect("every department consented");

        // One computer in ten misses each round; the rest still finish it.
        let dropped = u8::try_from(round % 30).expect("small") + 1;
        let mut uploads = BTreeMap::new();
        let mut maskers = BTreeMap::new();
        for &n in &all {
            let seeds = all
                .iter()
                .filter(|m| **m != n)
                .map(|m| (device(*m), pair_seed(n, *m, round)))
                .collect();
            let masker = MaskingClient::new(
                device(n),
                round,
                roster.clone(),
                plan.threshold,
                [n.wrapping_mul(7) ^ u8::try_from(round % 251).expect("small"); 32],
                seeds,
            )
            .expect("seeds for everyone");
            let d = department_of(n);
            let update = clients.get_mut(&n).expect("client").train(
                &parts[d],
                &server.params,
                &server.control,
                WORK,
            );
            let contribution = contribute(
                &plan,
                &directory,
                &slug(d),
                &update,
                &masker,
                &mut rng,
                Timestamp(1),
            )
            .expect("consented department");

            // Nothing leaves until the department's approver signs off on these bytes.
            let gate = gates.get_mut(&n).expect("gate");
            let id = gate.hold(contribution.draft);
            let hash = ContentHash::of(&upload_bytes(&contribution.masked));
            assert!(gate.release(id, &hash).is_err(), "unapproved upload left");
            let approval = gate
                .approve(id, approver_of(d), Timestamp(2), &directory)
                .expect("approver may approve");
            gate.release(id, &hash).expect("approved bytes");
            if n == 1 {
                append(&mut ledger, device(n), approval);
            }
            if n != dropped {
                uploads.insert(device(n), contribution.masked);
            }
            maskers.insert(n, masker);
        }

        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<Reveal> = maskers
            .values_mut()
            .filter(|m| survivors.contains(&m.me))
            .map(|m| m.reveal(&survivors).expect("consistent list"))
            .collect();
        let outcome = aggregate(&plan, &uploads, &reveals, &mut budget).expect("within budget");
        assert_eq!(outcome.participants, all.len() - 1);
        server.apply_mean(&outcome.mean, outcome.participants);
        append(
            &mut ledger,
            coordinator,
            round_event(&plan, &outcome, model_hash(&server.params)),
        );
    }

    let end = distance(&server.params, &target);
    assert!(end < 1.0 && end < start * 0.25, "{start} -> {end}");
    // The whole run kept its promise.
    assert!(epsilon_for(budget.spent_rho, DELTA) <= 10.0 + 1e-6);
    assert!(budget.charge(budget.left() + 1.0).is_err());

    // Promotion needs an approver from every department that trained it.
    let candidate = model_hash(&server.params);
    let two: BTreeSet<PersonId> = [approver_of(0), approver_of(1)].into();
    assert_eq!(
        promote(candidate, &departments, &two, &directory),
        Err(RoundError::NotAnApprover(slug(2)))
    );
    let three: BTreeSet<PersonId> = (0..3).map(approver_of).collect();
    let promotion = promote(candidate, &departments, &three, &directory).expect("all signed");
    append(&mut ledger, coordinator, promotion);

    let events: Vec<&AuditEvent> = ledger.entries().iter().map(|e| &e.body.event).collect();
    assert_eq!(active_model(events.iter().copied()), Some(candidate));
    let lineage = events
        .iter()
        .filter(|e| matches!(e, AuditEvent::ModelRound { .. }))
        .count();
    assert_eq!(lineage, usize::try_from(rounds).expect("small"));
}

#[test]
fn a_department_that_did_not_consent_cannot_join_or_contribute() {
    let mut directory = company();
    if let Some(legal) = directory.departments.get_mut(&slug(0)) {
        legal.training = TrainingConsent::Off;
    }
    let settings = RoundSettings {
        round: 1,
        base_model: model_hash(&[0.0; 3]),
        departments: (0..3).map(slug).collect(),
        roster: (1..=30).map(device).collect(),
        dimension: 3,
        quantiser: Quantiser {
            clip: 1.0,
            scale: 100.0,
        },
        assumed_colluders: 0,
        rounds_left: 1,
    };
    let budget = PrivacyBudget::for_epsilon(4.0, DELTA);
    assert!(matches!(
        RoundPlan::new(settings.clone(), &budget, &directory),
        Err(RoundError::Custody(CustodyError::TrainingOff(_)))
    ));
    let mut without_legal = settings;
    without_legal.departments.remove(&slug(0));
    let plan = RoundPlan::new(without_legal, &budget, &directory).expect("two consented");
    let masker = MaskingClient::new(
        device(1),
        1,
        [device(1)].into(),
        2,
        [0; 32],
        BTreeMap::new(),
    )
    .expect("alone");
    let update = vec![0.0f32; 3];
    assert_eq!(
        contribute(
            &plan,
            &directory,
            &slug(0),
            &update,
            &masker,
            &mut SeededRandom(1),
            Timestamp(1)
        ),
        Err(RoundError::NotInRound)
    );
}

#[test]
fn a_round_with_too_few_uploads_is_not_unmasked_or_charged() {
    let directory = company();
    let plan = RoundPlan::new(
        RoundSettings {
            round: 1,
            base_model: model_hash(&[0.0]),
            departments: [slug(1)].into(),
            roster: (11..=13).map(device).collect(),
            dimension: 2,
            quantiser: Quantiser {
                clip: 1.0,
                scale: 100.0,
            },
            assumed_colluders: 0,
            rounds_left: 1,
        },
        &PrivacyBudget::for_epsilon(4.0, DELTA),
        &directory,
    )
    .expect("finance consented");
    let mut budget = PrivacyBudget::for_epsilon(4.0, DELTA);
    let one: BTreeMap<DeviceId, Vec<u64>> = [(device(11), vec![0, 0])].into();
    assert_eq!(
        aggregate(&plan, &one, &[], &mut budget),
        Err(RoundError::TooFewComputers(plan.threshold))
    );
    assert!(budget.spent_rho <= 0.0);
}
