use sha2::{Digest as _, Sha256};

use super::*;
use crate::Timestamp;

/// Deterministic stand-in for Ed25519: only the named device can produce it.
fn fake_signature(device: &DeviceId, message: &[u8]) -> Signature {
    let first: [u8; 32] =
        Sha256::digest([b"sig".as_slice(), device.as_bytes(), message].concat()).into();
    let second: [u8; 32] = Sha256::digest(first).into();
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(&first);
    bytes[32..].copy_from_slice(&second);
    Signature(bytes)
}

struct FakeVerifier;

impl SignatureVerifier for FakeVerifier {
    fn verify(&self, signer: &DeviceId, message: &[u8], signature: &Signature) -> bool {
        fake_signature(signer, message) == *signature
    }
}

fn device(n: u8) -> DeviceId {
    DeviceId([n; 32])
}

fn person(n: u8) -> PersonId {
    PersonId([100 + n; 32])
}

fn sign(op: UnsignedOp) -> MembershipOp {
    let signature = fake_signature(&op.author, &op.signing_bytes());
    op.sign(signature)
}

const NONCE: [u8; 16] = [7; 16];

/// A group history built one signed operation at a time.
struct Log {
    group: GroupId,
    ops: Vec<MembershipOp>,
}

impl Log {
    /// Mum (person 1) founds "Family" on her laptop (device 1).
    fn family() -> Self {
        let group = GroupId::derive(&device(1), &NONCE);
        let genesis = sign(UnsignedOp {
            group,
            author: device(1),
            seq: 1,
            prev: None,
            deps: Vec::new(),
            at: Timestamp(1_000),
            body: MembershipBody::Genesis {
                name: "Family".into(),
                nonce: NONCE,
                founder: person(1),
                founder_name: "Mum".into(),
                device_name: "Mum's laptop".into(),
            },
        });
        Self {
            group,
            ops: vec![genesis],
        }
    }

    fn state(&self) -> GroupState {
        fold(self.group, &self.ops, &FakeVerifier)
    }

    /// Signs `body` from `author` on top of everything applied so far.
    fn push(&mut self, author: DeviceId, at: i64, body: MembershipBody) -> MembershipOp {
        let op = sign(self.state().draft(author, Timestamp(at), body));
        self.ops.push(op.clone());
        op
    }

    /// Adds person `n` with one device `n` and the given role, signed by device 1.
    fn add(&mut self, n: u8, role: Role, at: i64) {
        self.push(
            device(1),
            at,
            MembershipBody::AddPerson {
                person: person(n),
                name: format!("Person {n}"),
                role,
            },
        );
        self.push(
            device(1),
            at + 1,
            MembershipBody::AddDevice {
                person: person(n),
                device: device(n),
                name: format!("Computer {n}"),
            },
        );
    }
}

/// Small LCG so shuffles are reproducible without a dependency.
fn shuffled(ops: &[MembershipOp], seed: u64) -> Vec<MembershipOp> {
    let mut out = ops.to_vec();
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    for i in (1..out.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let j = usize::try_from(state >> 33).expect("fits") % (i + 1);
        out.swap(i, j);
    }
    out
}

fn roster(state: &GroupState) -> Vec<(PersonId, Role, Vec<DeviceId>)> {
    state
        .people
        .values()
        .map(|member| {
            (
                member.person,
                member.role,
                member.devices.keys().copied().collect(),
            )
        })
        .collect()
}

#[test]
fn founding_a_group_makes_the_founder_its_only_admin() {
    let state = Log::family().state();
    assert_eq!(state.name, "Family");
    assert_eq!(state.founder, Some(person(1)));
    assert_eq!(state.role_of(&device(1)), Some(Role::Admin));
    assert!(state.admits(&device(1)));
    assert!(!state.admits(&device(2)));
    assert!(state.needs_second_admin());
    assert_eq!(state.applied.len(), 1);
    assert!(state.rejected.is_empty() && state.pending.is_empty());
}

#[test]
fn only_the_founding_device_can_write_genesis() {
    let log = Log::family();
    let impostor = sign(UnsignedOp {
        author: device(9),
        ..log.ops[0].unsigned.clone()
    });
    let state = fold(log.group, [&impostor], &FakeVerifier);
    assert_eq!(
        state.rejected.get(&impostor.hash()),
        Some(&Rejection::InvalidGenesis)
    );
    assert!(state.people.is_empty());
}

#[test]
fn forged_signatures_and_other_groups_are_rejected() {
    let mut log = Log::family();
    let forged = log.state().draft(
        device(1),
        Timestamp(2_000),
        MembershipBody::Rename {
            name: "Hacked".into(),
        },
    );
    let wrong = fake_signature(&device(9), &forged.signing_bytes());
    let forged = forged.sign(wrong);
    log.ops.push(forged.clone());

    let mut elsewhere = log.state().draft(
        device(1),
        Timestamp(2_001),
        MembershipBody::Rename {
            name: "Other".into(),
        },
    );
    elsewhere.group = GroupId([0; 32]);
    let elsewhere = sign(elsewhere);
    log.ops.push(elsewhere.clone());

    let state = log.state();
    assert_eq!(state.name, "Family");
    assert_eq!(
        state.rejected.get(&forged.hash()),
        Some(&Rejection::BadSignature)
    );
    assert_eq!(
        state.rejected.get(&elsewhere.hash()),
        Some(&Rejection::WrongGroup)
    );
}

#[test]
fn admins_manage_people_and_members_only_their_own_computers() {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    // Dad (member) adds his own second computer: allowed.
    log.push(
        device(2),
        3_000,
        MembershipBody::AddDevice {
            person: person(2),
            device: device(22),
            name: "Dad's desktop".into(),
        },
    );
    // Dad tries to invite someone: only admins may.
    let refused = log.push(
        device(2),
        3_100,
        MembershipBody::AddPerson {
            person: person(3),
            name: "Stranger".into(),
            role: Role::Admin,
        },
    );
    let state = log.state();
    assert_eq!(state.role_of(&device(22)), Some(Role::Member));
    assert_eq!(
        state.rejected.get(&refused.hash()),
        Some(&Rejection::NotAllowed)
    );
    assert!(!state.people.contains_key(&person(3)));
}

#[test]
fn the_fold_is_the_same_in_any_order_with_duplicates() {
    let mut log = Log::family();
    log.add(2, Role::Admin, 2_000);
    log.add(3, Role::Member, 3_000);
    // Two admins act at once without seeing each other.
    let base = log.state();
    let rename = sign(base.draft(
        device(1),
        Timestamp(4_000),
        MembershipBody::Rename {
            name: "The Smiths".into(),
        },
    ));
    let promote = sign(base.draft(
        device(2),
        Timestamp(4_000),
        MembershipBody::ChangeRole {
            person: person(3),
            role: Role::Admin,
        },
    ));
    log.ops.extend([rename, promote]);
    log.add(4, Role::Viewer, 5_000);

    let expected = log.state();
    assert_eq!(expected.name, "The Smiths");
    assert_eq!(expected.admin_count(), 3);
    for seed in 0..300 {
        let mut ops = shuffled(&log.ops, seed);
        ops.extend(shuffled(&log.ops, seed + 1_000).into_iter().take(3));
        let state = fold(log.group, &ops, &FakeVerifier);
        assert_eq!(roster(&state), roster(&expected), "seed {seed}");
        assert_eq!(state.name, expected.name);
        assert_eq!(state.applied, expected.applied, "seed {seed}");
        assert_eq!(state.heads, expected.heads);
    }
}

#[test]
fn a_removed_admins_concurrent_actions_are_void() {
    let mut log = Log::family();
    log.add(2, Role::Admin, 2_000);
    let base = log.state();
    // Mum removes Dad while Dad, unaware, invites a stranger.
    let removal = sign(base.draft(
        device(1),
        Timestamp(3_000),
        MembershipBody::RemovePerson { person: person(2) },
    ));
    let invite = sign(base.draft(
        device(2),
        Timestamp(3_000),
        MembershipBody::AddPerson {
            person: person(9),
            name: "Stranger".into(),
            role: Role::Admin,
        },
    ));
    log.ops.extend([removal, invite.clone()]);

    for seed in 0..50 {
        let state = fold(log.group, &shuffled(&log.ops, seed), &FakeVerifier);
        assert!(!state.people.contains_key(&person(2)));
        assert!(!state.people.contains_key(&person(9)), "seed {seed}");
        assert_eq!(
            state.rejected.get(&invite.hash()),
            Some(&Rejection::AuthorRemoved)
        );
        assert!(!state.admits(&device(2)));
    }
}

#[test]
fn a_removal_wins_over_a_concurrent_add_but_a_later_re_add_works() {
    let mut log = Log::family();
    log.add(2, Role::Admin, 2_000);
    log.add(3, Role::Member, 2_100);
    let base = log.state();
    // Mum removes Sam; Dad, unaware, adds a computer for Sam.
    let removal = sign(base.draft(
        device(1),
        Timestamp(3_000),
        MembershipBody::RemovePerson { person: person(3) },
    ));
    let add = sign(base.draft(
        device(2),
        Timestamp(3_000),
        MembershipBody::AddDevice {
            person: person(3),
            device: device(33),
            name: "Sam's phone".into(),
        },
    ));
    log.ops.extend([removal, add.clone()]);
    let state = log.state();
    assert!(!state.people.contains_key(&person(3)));
    assert!(matches!(
        state.rejected.get(&add.hash()),
        Some(Rejection::RemovedConcurrently | Rejection::UnknownTarget)
    ));

    // Later, having seen the removal, Mum adds Sam back and Sam's computer works again.
    log.add(3, Role::Member, 4_000);
    let renamed = log.push(
        device(3),
        5_000,
        MembershipBody::RemoveDevice { device: device(3) },
    );
    let state = log.state();
    assert!(state.people.contains_key(&person(3)));
    assert!(state.applied.contains(&renamed.hash()));
}

#[test]
fn admins_who_remove_each_other_at_once_leave_exactly_one_admin() {
    let mut log = Log::family();
    log.add(2, Role::Admin, 2_000);
    let base = log.state();
    let mum_removes_dad = sign(base.draft(
        device(1),
        Timestamp(3_000),
        MembershipBody::RemovePerson { person: person(2) },
    ));
    let dad_removes_mum = sign(base.draft(
        device(2),
        Timestamp(3_000),
        MembershipBody::RemovePerson { person: person(1) },
    ));
    log.ops.extend([mum_removes_dad, dad_removes_mum]);

    let expected = log.state();
    assert_eq!(expected.admin_count(), 1);
    assert_eq!(expected.people.len(), 1);
    for seed in 0..50 {
        let state = fold(log.group, &shuffled(&log.ops, seed), &FakeVerifier);
        assert_eq!(roster(&state), roster(&expected), "seed {seed}");
    }
}

#[test]
fn the_last_admin_cannot_leave_or_step_down() {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    let leave = log.push(
        device(1),
        3_000,
        MembershipBody::RemovePerson { person: person(1) },
    );
    let demote = log.push(
        device(1),
        3_100,
        MembershipBody::ChangeRole {
            person: person(1),
            role: Role::Member,
        },
    );
    let state = log.state();
    assert_eq!(
        state.rejected.get(&leave.hash()),
        Some(&Rejection::WouldLeaveNoAdmin)
    );
    assert_eq!(
        state.rejected.get(&demote.hash()),
        Some(&Rejection::WouldLeaveNoAdmin)
    );
    assert_eq!(state.admin_count(), 1);

    // A member may leave on their own.
    let left = log.push(
        device(2),
        4_000,
        MembershipBody::RemovePerson { person: person(2) },
    );
    assert!(log.state().applied.contains(&left.hash()));
    assert!(log.state().removed_people.contains(&person(2)));
}

#[test]
fn a_device_that_signs_two_histories_is_quarantined() {
    let mut log = Log::family();
    let base = log.state();
    let one = sign(base.draft(
        device(1),
        Timestamp(2_000),
        MembershipBody::Rename { name: "One".into() },
    ));
    let two = sign(base.draft(
        device(1),
        Timestamp(2_000),
        MembershipBody::Rename { name: "Two".into() },
    ));
    log.ops.extend([one.clone(), two.clone()]);
    let state = log.state();
    assert!(state.forked.contains(&device(1)));
    assert_eq!(state.rejected.get(&one.hash()), Some(&Rejection::Forked));
    assert_eq!(state.rejected.get(&two.hash()), Some(&Rejection::Forked));
    assert_eq!(state.name, "Family");
}

#[test]
fn an_operation_waits_until_its_parent_arrives() {
    let mut log = Log::family();
    let parent = log.push(
        device(1),
        2_000,
        MembershipBody::Rename {
            name: "Smiths".into(),
        },
    );
    let child = log.push(
        device(1),
        3_000,
        MembershipBody::Rename {
            name: "The Smiths".into(),
        },
    );

    let without_parent: Vec<&MembershipOp> = log
        .ops
        .iter()
        .filter(|op| op.hash() != parent.hash())
        .collect();
    let state = fold(log.group, without_parent, &FakeVerifier);
    assert!(state.pending.contains(&child.hash()));
    assert_eq!(state.name, "Family");

    let state = log.state();
    assert!(state.pending.is_empty());
    assert_eq!(state.name, "The Smiths");
}

#[test]
fn drafts_chain_to_the_authors_last_operation_and_the_current_heads() {
    let mut log = Log::family();
    let first = log.push(
        device(1),
        2_000,
        MembershipBody::Rename { name: "A".into() },
    );
    let draft = log.state().draft(
        device(1),
        Timestamp(3_000),
        MembershipBody::Rename { name: "B".into() },
    );
    assert_eq!(draft.seq, 3);
    assert_eq!(draft.prev, Some(first.hash()));
    assert_eq!(draft.deps, Vec::<OpHash>::new());
    let fresh = log.state().draft(
        device(2),
        Timestamp(3_000),
        MembershipBody::Rename { name: "C".into() },
    );
    assert_eq!((fresh.seq, fresh.prev), (1, None));
    assert_eq!(fresh.deps, vec![first.hash()]);
}

#[test]
fn empty_or_overlong_names_are_rejected() {
    let mut log = Log::family();
    let blank = log.push(
        device(1),
        2_000,
        MembershipBody::Rename { name: "   ".into() },
    );
    let long = log.push(
        device(1),
        2_100,
        MembershipBody::Rename {
            name: "x".repeat(MAX_NAME_CHARS + 1),
        },
    );
    let state = log.state();
    assert_eq!(
        state.rejected.get(&blank.hash()),
        Some(&Rejection::InvalidName)
    );
    assert_eq!(
        state.rejected.get(&long.hash()),
        Some(&Rejection::InvalidName)
    );
}

#[test]
fn operations_survive_a_json_round_trip_with_the_same_hash() {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    for op in &log.ops {
        let text = serde_json::to_string(op).expect("serialises");
        let back: MembershipOp = serde_json::from_str(&text).expect("parses");
        assert_eq!(back.hash(), op.hash());
        assert_eq!(&back, op);
    }
    assert!(
        serde_json::to_string(&log.ops[0])
            .expect("serialises")
            .contains(&device(1).to_hex())
    );
}

#[test]
fn ids_parse_back_from_hex_and_reject_junk() {
    let id = device(0xab);
    assert_eq!(DeviceId::from_hex(&id.to_hex()), Ok(id));
    assert_eq!(id.fingerprint(), "abab · abab");
    assert!(matches!(
        DeviceId::from_hex("abc"),
        Err(KeyParseError::Length { .. })
    ));
    assert_eq!(
        DeviceId::from_hex(&"zz".repeat(32)),
        Err(KeyParseError::NotHex)
    );
}

#[test]
fn both_screens_derive_the_same_code_whatever_the_order() {
    let a = device(1);
    let b = device(2);
    let here = short_auth_string(b"transcript", &a, &b);
    let there = short_auth_string(b"transcript", &b, &a);
    assert_eq!(here, there);
    assert_ne!(here, short_auth_string(b"other transcript", &a, &b));
    assert_ne!(here, short_auth_string(b"transcript", &a, &device(3)));
    assert_eq!(here.digits.len(), 7);
    assert_eq!(here.spoken().matches(" · ").count(), 3);
    let mut words: Vec<&str> = SAS_EMOJI.iter().map(|(_, word)| *word).collect();
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), SAS_EMOJI.len(), "spoken words must be unique");
}

#[test]
fn an_invite_works_once_and_only_before_it_expires() {
    let group = Log::family().group;
    let now = Timestamp(10_000);
    let invite = Invite::new(group, InviteKind::OwnDevice, [5; 16], now);
    let mut book = InviteBook::default();
    book.issue(invite.clone());
    assert_eq!(book.live(now), 1);

    let proof = invite.proof(b"connection");
    assert_eq!(
        book.redeem(&invite.proof(b"other connection"), b"connection", now),
        Err(RedeemError::Unknown)
    );
    assert_eq!(book.redeem(&proof, b"connection", now), Ok(invite));
    assert_eq!(
        book.redeem(&proof, b"connection", now),
        Err(RedeemError::AlreadyUsed)
    );

    let late = Invite::new(group, InviteKind::OwnDevice, [6; 16], now);
    book.issue(late.clone());
    let after = Timestamp(now.millis() + INVITE_LIFETIME_MS);
    assert_eq!(
        book.redeem(&late.proof(b"c"), b"c", after),
        Err(RedeemError::Expired)
    );
    book.prune(after);
    assert_eq!(book.live(after), 0);
}

#[test]
fn a_short_code_allows_one_guess_within_two_minutes() {
    let now = Timestamp(0);
    let mut window = ShortCodeWindow::open(now);
    assert!(window.take_guess(Timestamp(1_000)));
    assert!(!window.take_guess(Timestamp(1_001)));
    let mut late = ShortCodeWindow::open(now);
    assert!(!late.take_guess(Timestamp(SHORT_CODE_LIFETIME_MS)));
}

#[test]
fn pairing_needs_both_people_to_confirm_the_code() {
    let group = Log::family().group;
    let code = short_auth_string(b"t", &device(1), &device(2));
    let stage = PairingStage::Idle
        .advance(PairingStep::InviteShown {
            expires_at: Timestamp(600_000),
        })
        .advance(PairingStep::PeerConnected)
        .advance(PairingStep::CodeReady(code))
        .advance(PairingStep::ConfirmedHere(true));
    assert!(matches!(
        stage,
        PairingStage::ConfirmCode {
            here: Some(true),
            there: None,
            ..
        }
    ));
    let joined = stage
        .clone()
        .advance(PairingStep::ConfirmedThere(true))
        .advance(PairingStep::Admitted { group });
    assert_eq!(joined, PairingStage::Joined { group });

    let mismatch = stage.advance(PairingStep::ConfirmedThere(false));
    assert_eq!(
        mismatch,
        PairingStage::Failed {
            reason: PairingFailure::CodeMismatch
        }
    );

    let expired = PairingStage::Idle
        .advance(PairingStep::InviteShown {
            expires_at: Timestamp(10),
        })
        .advance(PairingStep::Tick(Timestamp(10)));
    assert_eq!(
        expired,
        PairingStage::Failed {
            reason: PairingFailure::Expired
        }
    );

    // A scanned QR in the same room skips the code; stray steps change nothing.
    let quick = PairingStage::Idle
        .advance(PairingStep::PeerConnected)
        .advance(PairingStep::ConfirmedHere(true))
        .advance(PairingStep::SkipCode)
        .advance(PairingStep::Admitted { group });
    assert_eq!(quick, PairingStage::Joined { group });
}

#[test]
fn the_groups_view_shows_the_roster_admins_first_with_this_computer_marked() {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    log.add(3, Role::Admin, 2_100);
    let mut view = GroupsView::default();
    assert!(!view.loaded);
    view.apply(GroupEvent::Loaded {
        this_computer: device(2),
        group: Some(log.state()),
    });
    assert!(view.loaded);
    assert_eq!(view.my_role(), Some(Role::Member));
    assert!(!view.can_manage());
    assert!(!view.show_second_admin_banner());

    let rows = view.rows();
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["Mum", "Person 3", "Person 2"]);
    assert!(rows[2].computers[0].is_this_computer);
    assert_eq!(rows[0].role_label, role_label(Role::Admin));
}

#[test]
fn the_view_leaves_the_group_when_this_computer_is_removed() {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    let mut view = GroupsView::default();
    view.apply(GroupEvent::Loaded {
        this_computer: device(2),
        group: Some(log.state()),
    });
    view.apply(GroupEvent::InviteReady {
        link: "sovra://join/x".into(),
        expires_at: Timestamp(9_000),
    });
    assert!(matches!(view.pairing, PairingStage::ShowingInvite { .. }));

    log.push(
        device(1),
        3_000,
        MembershipBody::RemoveDevice { device: device(2) },
    );
    view.apply(GroupEvent::Changed(log.state()));
    assert_eq!(view.group, None);
    assert_eq!(view.invite, None);
    assert_eq!(view.pairing, PairingStage::Idle);
}

#[test]
fn the_founder_alone_sees_the_second_admin_banner_and_problems_are_plain() {
    let mut view = GroupsView::default();
    view.apply(GroupEvent::Loaded {
        this_computer: device(1),
        group: Some(Log::family().state()),
    });
    assert!(view.can_manage());
    assert!(view.show_second_admin_banner());
    view.apply(GroupEvent::Problem(GroupProblem::LastAdmin));
    let message = view.problem.as_ref().map(GroupProblem::message);
    assert!(message.is_some_and(|text| text.contains("at least one admin")));
    assert!(removal_warning("Sam's phone").starts_with("Sam's phone will stop"));
}
