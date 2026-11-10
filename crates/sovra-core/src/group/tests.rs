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

// ---- shared knowledge ----------------------------------------------------

fn sign_knowledge(op: UnsignedKnowledge) -> KnowledgeOp {
    let signature = fake_signature(&op.author, &op.signing_bytes());
    op.sign(signature)
}

/// Shared details on top of a roster.
struct Shared {
    roster: GroupState,
    lane: LaneId,
    ops: Vec<KnowledgeOp>,
}

impl Shared {
    fn new(log: &Log) -> Self {
        Self {
            roster: log.state(),
            lane: LaneId::whole_group(&log.group),
            ops: Vec::new(),
        }
    }

    fn view(&self) -> KnowledgeView {
        fold_knowledge(&self.roster, self.lane, &self.ops, &FakeVerifier)
    }

    fn push(&mut self, author: DeviceId, at: i64, body: KnowledgeBody) -> KnowledgeOp {
        let draft = self
            .view()
            .draft(author, Timestamp(at), body)
            .expect("a folded view knows its lane");
        let op = sign_knowledge(draft);
        self.ops.push(op.clone());
        op
    }

    fn share(&mut self, author: DeviceId, at: i64, text: &str, replaces: Vec<OpHash>) -> OpHash {
        self.push(author, at, share_body(text, replaces)).hash()
    }
}

fn share_body(text: &str, replaces: Vec<OpHash>) -> KnowledgeBody {
    KnowledgeBody::Share {
        text: text.into(),
        about: Vec::new(),
        valid_at: None,
        replaces,
    }
}

fn family_of_three() -> Log {
    let mut log = Log::family();
    log.add(2, Role::Member, 2_000);
    log.add(3, Role::Member, 2_100);
    log
}

#[test]
fn a_shared_detail_is_found_by_others_with_who_shared_it() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    shared.share(
        device(2),
        3_000,
        "The boiler service is booked for 12 March",
        Vec::new(),
    );
    shared.share(
        device(3),
        3_100,
        "Sam's football is on Saturdays",
        Vec::new(),
    );

    let hits = shared.view().search("when is the boiler serviced?", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].detail.shared_by, "Person 2");
    assert_eq!(hits[0].detail.computer, "Computer 2");
    assert!(hits[0].detail.text.contains("12 March"));
    assert_eq!(
        shared.view().search("a", 5),
        Vec::new(),
        "one-letter words are ignored"
    );
}

#[test]
fn a_correction_replaces_the_old_detail_in_answers() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let old = shared.share(
        device(2),
        3_000,
        "The boiler service is on 12 March",
        Vec::new(),
    );
    let new = shared.share(
        device(1),
        4_000,
        "The boiler service moved to 19 March",
        vec![old],
    );
    let view = shared.view();
    assert_eq!(
        view.details[&old].status,
        DetailStatus::Replaced { by: vec![new] }
    );
    let hits = view.search("boiler service", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].detail.id, new);
}

#[test]
fn different_corrections_made_at_once_are_a_dispute_until_settled() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let price = shared.share(device(1), 3_000, "Call-out fee is 60 pounds", Vec::new());
    let base = shared.view();
    let dad = sign_knowledge(
        base.draft(
            device(2),
            Timestamp(4_000),
            share_body("Call-out fee is 65 pounds", vec![price]),
        )
        .expect("lane"),
    );
    let sam = sign_knowledge(
        base.draft(
            device(3),
            Timestamp(4_001),
            share_body("Call-out fee is 70 pounds", vec![price]),
        )
        .expect("lane"),
    );
    shared.ops.extend([dad.clone(), sam.clone()]);

    let view = shared.view();
    assert_eq!(view.disputes.len(), 1);
    assert!(matches!(
        view.details[&sam.hash()].status,
        DetailStatus::Disputed { chosen: true, .. }
    ));
    assert!(matches!(
        view.details[&dad.hash()].status,
        DetailStatus::Disputed { chosen: false, .. }
    ));
    assert_eq!(
        view.search("call-out fee", 5).len(),
        2,
        "both sides stay visible"
    );

    let settled = shared.share(
        device(1),
        5_000,
        "Call-out fee is 65 pounds",
        vec![dad.hash(), sam.hash()],
    );
    let view = shared.view();
    assert_eq!(view.disputes, Vec::<Vec<OpHash>>::new());
    let hits = view.search("call-out fee", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].detail.id, settled);
}

#[test]
fn identical_corrections_made_at_once_agree_instead_of_disputing() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let old = shared.share(device(1), 3_000, "Bins go out on Tuesday", Vec::new());
    let base = shared.view();
    let body = share_body("Bins go out on Wednesday", vec![old]);
    let a = sign_knowledge(
        base.draft(device(2), Timestamp(4_000), body.clone())
            .expect("lane"),
    );
    let b = sign_knowledge(base.draft(device(3), Timestamp(4_000), body).expect("lane"));
    shared.ops.extend([a, b]);
    assert_eq!(shared.view().disputes, Vec::<Vec<OpHash>>::new());
}

#[test]
fn agreement_ranks_a_detail_higher_and_nobody_agrees_with_themselves() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let lone = shared.share(
        device(2),
        3_000,
        "The spare key is under the blue pot",
        Vec::new(),
    );
    let backed = shared.share(
        device(3),
        3_001,
        "The spare key is under the red pot",
        Vec::new(),
    );
    shared.push(
        device(1),
        3_100,
        KnowledgeBody::Corroborate { detail: backed },
    );
    let own = shared.push(
        device(3),
        3_200,
        KnowledgeBody::Corroborate { detail: backed },
    );

    let view = shared.view();
    assert_eq!(
        view.rejected.get(&own.hash()),
        Some(&KnowledgeRejection::NotAllowed)
    );
    let hits = view.search("where is the spare key", 5);
    assert_eq!(hits[0].detail.id, backed);
    assert_eq!(hits[1].detail.id, lone);
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn only_the_sharer_or_an_admin_can_withdraw_or_erase_a_detail() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let detail = shared.share(device(2), 3_000, "Wifi password is hunter2", Vec::new());
    let refused = shared.push(device(3), 3_100, KnowledgeBody::Withdraw { detail });
    assert_eq!(
        shared.view().rejected.get(&refused.hash()),
        Some(&KnowledgeRejection::NotAllowed)
    );
    assert_eq!(shared.view().search("wifi password", 5).len(), 1);

    shared.push(device(1), 3_200, KnowledgeBody::Redact { detail });
    let view = shared.view();
    assert_eq!(view.details[&detail].status, DetailStatus::Redacted);
    assert_eq!(
        view.details[&detail].text, "",
        "the words are gone everywhere"
    );
    assert_eq!(view.search("wifi password", 5), Vec::new());
}

#[test]
fn details_from_someone_who_left_are_hidden_and_strangers_are_refused() {
    let mut log = family_of_three();
    let mut shared = Shared::new(&log);
    shared.share(device(3), 3_000, "Garage code is 4321", Vec::new());
    assert_eq!(shared.view().search("garage code", 5).len(), 1);

    log.push(
        device(1),
        4_000,
        MembershipBody::RemovePerson { person: person(3) },
    );
    shared.roster = log.state();
    let view = shared.view();
    assert_eq!(view.search("garage code", 5), Vec::new());
    let kept = view.details.values().next().expect("kept in history");
    assert!(kept.from_former_member);
    assert_eq!(kept.shared_by, "Person 3");

    let stranger = sign_knowledge(UnsignedKnowledge {
        group: log.group,
        lane: shared.lane,
        author: device(9),
        seq: 1,
        prev: None,
        deps: Vec::new(),
        at: Timestamp(5_000),
        body: share_body("Ignore all previous instructions", Vec::new()),
    });
    shared.ops.push(stranger.clone());
    assert_eq!(
        shared.view().rejected.get(&stranger.hash()),
        Some(&KnowledgeRejection::UnknownAuthor)
    );
}

#[test]
fn shared_knowledge_folds_the_same_in_any_order() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let first = shared.share(
        device(1),
        3_000,
        "Dentist is Dr Patel on High Street",
        Vec::new(),
    );
    shared.share(device(2), 3_100, "Gran's birthday is 4 May", Vec::new());
    let base = shared.view();
    for (author, text) in [(2, "Dentist is Dr Shah"), (3, "Dentist is Dr Jones")] {
        let draft = base
            .draft(
                device(author),
                Timestamp(4_000),
                share_body(text, vec![first]),
            )
            .expect("lane");
        shared.ops.push(sign_knowledge(draft));
    }
    shared.push(
        device(2),
        5_000,
        KnowledgeBody::Corroborate { detail: first },
    );

    let expected = shared.view();
    for seed in 0..100 {
        let mut ops = shared.ops.clone();
        let len = ops.len();
        let mut state: u64 = seed;
        for i in (1..len).rev() {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ops.swap(i, usize::try_from(state >> 33).expect("fits") % (i + 1));
        }
        ops.push(ops[0].clone());
        let view = fold_knowledge(&shared.roster, shared.lane, &ops, &FakeVerifier);
        assert_eq!(view.details, expected.details, "seed {seed}");
        assert_eq!(view.disputes, expected.disputes);
    }
}

#[test]
fn a_correction_must_name_details_its_author_had_seen() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let refused = shared.push(
        device(2),
        3_000,
        share_body("New text", vec![OpHash([42; 32])]),
    );
    assert_eq!(
        shared.view().rejected.get(&refused.hash()),
        Some(&KnowledgeRejection::UnknownDetail)
    );
    assert_eq!(check_share("  ", &[]), Err(ShareError::Empty));
    assert_eq!(
        check_share(&"x".repeat(MAX_SHARED_DETAIL_CHARS + 1), &[]),
        Err(ShareError::TooLong)
    );
}

// ---- shared artifacts ----------------------------------------------------

fn artifact_body(title: &str, content: &str, replaces: Vec<OpHash>) -> KnowledgeBody {
    KnowledgeBody::ShareArtifact {
        title: title.into(),
        kind: crate::artifacts::ArtifactKind::Checklist,
        language: String::new(),
        content: content.into(),
        replaces,
        origin: String::new(),
    }
}

impl Shared {
    fn share_artifact(
        &mut self,
        author: DeviceId,
        at: i64,
        content: &str,
        replaces: Vec<OpHash>,
    ) -> OpHash {
        self.push(
            author,
            at,
            artifact_body("Shopping list", content, replaces),
        )
        .hash()
    }
}

#[test]
fn a_shared_artifact_reaches_everyone_and_a_new_version_replaces_the_old() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let first = shared.share_artifact(device(2), 3_000, "- milk\n- eggs", Vec::new());
    let view = shared.view();
    let artifact = &view.artifacts[&first];
    assert_eq!(artifact.shared_by, "Person 2");
    assert_eq!(artifact.title, "Shopping list");
    assert!(artifact.is_latest());

    // Sam edits a copy of the first version and shares it back.
    let replaces = shared.view().latest_versions_of(&first);
    assert_eq!(replaces, vec![first]);
    let second = shared.share_artifact(device(3), 4_000, "- milk\n- eggs\n- bread", replaces);
    let view = shared.view();
    assert_eq!(
        view.artifacts[&first].status,
        DetailStatus::Replaced { by: vec![second] }
    );
    assert_eq!(
        view.shared_artifacts()[0].id,
        second,
        "latest version first"
    );
    assert_eq!(
        view.latest_versions_of(&first),
        vec![second],
        "an edit of an old copy replaces the newest version, never forks"
    );
    assert_eq!(view.latest_versions_of(&OpHash([7; 32])), Vec::new());
}

#[test]
fn two_versions_made_at_once_are_both_kept_until_one_settles_them() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let first = shared.share_artifact(device(1), 3_000, "- milk", Vec::new());
    let base = shared.view();
    let dad = sign_knowledge(
        base.draft(
            device(2),
            Timestamp(4_000),
            artifact_body("Shopping list", "- milk\n- tea", vec![first]),
        )
        .expect("lane"),
    );
    let sam = sign_knowledge(
        base.draft(
            device(3),
            Timestamp(4_001),
            artifact_body("Shopping list", "- milk\n- crisps", vec![first]),
        )
        .expect("lane"),
    );
    shared.ops.extend([dad.clone(), sam.clone()]);
    let view = shared.view();
    assert!(matches!(
        view.artifacts[&sam.hash()].status,
        DetailStatus::Disputed { chosen: true, .. }
    ));
    assert!(matches!(
        view.artifacts[&dad.hash()].status,
        DetailStatus::Disputed { chosen: false, .. }
    ));
    assert_eq!(
        view.disputes,
        Vec::<Vec<OpHash>>::new(),
        "disputes list details"
    );
    let mut both = vec![dad.hash(), sam.hash()];
    both.sort();
    assert_eq!(view.latest_versions_of(&first), both);

    let merged = shared.share_artifact(
        device(1),
        5_000,
        "- milk\n- tea\n- crisps",
        view.latest_versions_of(&first),
    );
    let view = shared.view();
    assert_eq!(view.latest_versions_of(&first), vec![merged]);
    assert_ne!(view.artifacts[&dad.hash()].status, DetailStatus::Current);
    assert!(!view.artifacts[&sam.hash()].is_latest());
}

#[test]
fn viewers_cannot_add_and_only_the_sharer_or_an_admin_can_remove_an_artifact() {
    let mut log = family_of_three();
    log.add(4, Role::Viewer, 2_200);
    let mut shared = Shared::new(&log);
    let viewer_artifact = shared.push(
        device(4),
        3_000,
        artifact_body("Notes", "hello", Vec::new()),
    );
    let viewer_detail = shared.push(device(4), 3_001, share_body("Hello", Vec::new()));
    let view = shared.view();
    assert_eq!(
        view.rejected.get(&viewer_artifact.hash()),
        Some(&KnowledgeRejection::NotAllowed)
    );
    assert_eq!(
        view.rejected.get(&viewer_detail.hash()),
        Some(&KnowledgeRejection::NotAllowed)
    );

    let list = shared.share_artifact(device(2), 3_100, "- milk", Vec::new());
    let refused = shared.push(device(3), 3_200, KnowledgeBody::Withdraw { detail: list });
    assert_eq!(
        shared.view().rejected.get(&refused.hash()),
        Some(&KnowledgeRejection::NotAllowed)
    );
    shared.push(device(2), 3_300, KnowledgeBody::Withdraw { detail: list });
    assert_eq!(
        shared.view().artifacts[&list].status,
        DetailStatus::Withdrawn
    );
    shared.push(device(1), 3_400, KnowledgeBody::Redact { detail: list });
    let view = shared.view();
    assert_eq!(view.artifacts[&list].status, DetailStatus::Redacted);
    assert_eq!(
        view.artifacts[&list].content, "",
        "erased on every computer"
    );
    assert!(!view.artifacts[&list].is_latest());
}

#[test]
fn oversized_or_empty_artifacts_are_refused_by_sharer_and_receiver() {
    assert_eq!(
        check_artifact("List", "", &"x".repeat(MAX_SHARED_ARTIFACT_BYTES + 1)),
        Err(ArtifactShareError::TooBig)
    );
    assert_eq!(
        check_artifact(" ", "", "x"),
        Err(ArtifactShareError::NoTitle)
    );
    assert_eq!(
        check_artifact("List", "", "  "),
        Err(ArtifactShareError::Empty)
    );
    assert_eq!(check_artifact("List", "", "- milk"), Ok(()));

    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let huge = shared.push(
        device(2),
        3_000,
        artifact_body(
            "Big",
            &"x".repeat(MAX_SHARED_ARTIFACT_BYTES + 1),
            Vec::new(),
        ),
    );
    assert_eq!(
        shared.view().rejected.get(&huge.hash()),
        Some(&KnowledgeRejection::InvalidDetail)
    );
}

#[test]
fn shared_artifacts_fold_the_same_in_any_order() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let first = shared.share_artifact(device(1), 3_000, "- milk", Vec::new());
    let base = shared.view();
    for (author, content) in [(2, "- milk\n- tea"), (3, "- milk\n- jam")] {
        let draft = base
            .draft(
                device(author),
                Timestamp(4_000),
                artifact_body("Shopping list", content, vec![first]),
            )
            .expect("lane");
        shared.ops.push(sign_knowledge(draft));
    }
    shared.share(device(2), 5_000, "Bins on Tuesday", Vec::new());
    let expected = shared.view();
    for seed in 0..60 {
        let mut ops = shared.ops.clone();
        let mut state: u64 = seed;
        for i in (1..ops.len()).rev() {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ops.swap(i, usize::try_from(state >> 33).expect("fits") % (i + 1));
        }
        let view = fold_knowledge(&shared.roster, shared.lane, &ops, &FakeVerifier);
        assert_eq!(view.artifacts, expected.artifacts, "seed {seed}");
    }
}

#[test]
fn a_large_history_is_sent_in_rounds_that_always_make_progress() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    for n in 0..4_i64 {
        shared.share_artifact(device(2), 3_000 + n, &"x".repeat(1_000), Vec::new());
    }
    let ops = shared.ops.clone();
    let (all, trimmed) = within_budget(ops.clone(), usize::MAX);
    assert_eq!(all.len(), 4);
    assert!(!trimmed);
    let size = |op: &KnowledgeOp| serde_json::to_vec(op).expect("plain data").len();
    let (some, trimmed) = within_budget(ops.clone(), size(&ops[0]) + size(&ops[1]) + 10);
    assert_eq!(some.len(), 2);
    assert!(trimmed);
    assert_eq!(some, ops[..2].to_vec(), "oldest first, in order");
    let (one, trimmed) = within_budget(ops, 1);
    assert_eq!(one.len(), 1, "the first always fits so sync never stalls");
    assert!(trimmed);
}

#[test]
fn the_groups_view_keeps_shared_artifacts_until_this_computer_leaves() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    shared.share_artifact(device(2), 3_000, "- milk", Vec::new());
    let mut view = GroupsView::default();
    view.apply(GroupEvent::Loaded {
        this_computer: device(1),
        group: Some(log.state()),
    });
    view.apply(GroupEvent::Artifacts(shared.view().shared_artifacts()));
    assert_eq!(view.artifacts.len(), 1);
    view.apply(GroupEvent::LeftGroup);
    assert_eq!(view.artifacts, Vec::<SharedArtifact>::new());
}

#[test]
fn a_saved_artifact_knows_whether_the_group_has_its_latest_words() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let me = device(2);
    assert_eq!(
        sharing_state("art-1", "", "Shopping list", "- milk", &me, &[]),
        SharingState::NotShared
    );
    let original = shared
        .push(
            me,
            3_000,
            KnowledgeBody::ShareArtifact {
                title: "Shopping list".into(),
                kind: crate::artifacts::ArtifactKind::Checklist,
                language: String::new(),
                content: "- milk".into(),
                replaces: Vec::new(),
                origin: "art-1".into(),
            },
        )
        .hash();
    let all = shared.view().shared_artifacts();
    assert_eq!(
        sharing_state("art-1", "", "Shopping list", "- milk", &me, &all),
        SharingState::UpToDate
    );
    assert_eq!(
        sharing_state("art-1", "", "Shopping list", "- milk\n- eggs", &me, &all),
        SharingState::Changed {
            version_of: original
        }
    );
    assert_eq!(
        sharing_state("art-1", "", "Shopping list", "- milk", &device(3), &all),
        SharingState::NotShared,
        "another computer's library ids mean nothing here"
    );

    // Sam keeps a copy; the group moves on; Sam's copy is out of date.
    let sams_copy_from = original.to_hex();
    shared.share_artifact(device(1), 4_000, "- milk\n- jam", vec![original]);
    let all = shared.view().shared_artifacts();
    assert_eq!(
        sharing_state(
            "art-9",
            &sams_copy_from,
            "Shopping list",
            "- milk",
            &device(3),
            &all
        ),
        SharingState::Changed {
            version_of: original
        }
    );
    assert_eq!(
        sharing_state(
            "art-9",
            &sams_copy_from,
            "Shopping list",
            "- milk\n- jam",
            &device(3),
            &all
        ),
        SharingState::UpToDate
    );
}

#[test]
fn a_personal_memory_is_reworded_with_the_sharers_name() {
    assert_eq!(
        shareable_text("User's dentist is Dr Patel", "Mum"),
        "Mum's dentist is Dr Patel"
    );
    assert_eq!(
        shareable_text("The user prefers the 8am train", "Dad"),
        "Dad prefers the 8am train"
    );
    assert_eq!(
        shareable_text("the user's username is sam_88", "Sam"),
        "Sam's username is sam_88",
        "only the whole word changes"
    );
    assert_eq!(
        shareable_text("Bins go out on Tuesday", "Mum"),
        "Bins go out on Tuesday"
    );
    assert_eq!(
        shareable_text("Sam goes to the école près de la gare", "Mum"),
        "Sam goes to the école près de la gare"
    );
    assert_eq!(shareable_text("  User likes tea ", ""), "User likes tea");
    assert_eq!(
        shareable_topics(&["user".into(), "Dr Patel".into(), "User".into()], "Mum"),
        vec!["Mum".to_owned(), "Dr Patel".to_owned()]
    );
}

// ---- federated recall ----------------------------------------------------

#[test]
fn federated_answers_are_few_short_reworded_and_unique() {
    let found = vec![
        ("User's dentist is Dr Patel".to_owned(), Timestamp(1)),
        ("User's dentist is Dr Patel".to_owned(), Timestamp(2)),
        ("x".repeat(MAX_FEDERATED_STATEMENT_CHARS + 50), Timestamp(3)),
        ("  ".to_owned(), Timestamp(4)),
        ("A".into(), Timestamp(5)),
        ("B".into(), Timestamp(6)),
        ("C".into(), Timestamp(7)),
    ];
    let items = federated_items(found, "Mum");
    assert_eq!(items.len(), MAX_FEDERATED_ITEMS);
    assert_eq!(items[0].statement, "Mum's dentist is Dr Patel");
    assert_eq!(items[0].recorded_at, Timestamp(1), "duplicates collapse");
    assert_eq!(
        items[1].statement.chars().count(),
        MAX_FEDERATED_STATEMENT_CHARS + 1,
        "long answers are cut with an ellipsis"
    );
    assert!(items.iter().all(|item| !item.statement.trim().is_empty()));
    assert_eq!(
        check_ask("  who is the dentist? "),
        Some("who is the dentist?".into())
    );
    assert_eq!(check_ask("   "), None);
    assert_eq!(check_ask(&"q".repeat(MAX_ASK_CHARS + 1)), None);
}

#[test]
fn a_computer_may_ask_another_only_so_often() {
    let mut limiter = AskLimiter::default();
    for n in 0..ASKS_PER_MINUTE {
        assert!(limiter.allow(
            device(2),
            Timestamp(1_000 + i64::try_from(n).expect("small"))
        ));
    }
    assert!(
        !limiter.allow(device(2), Timestamp(2_000)),
        "too many in a minute"
    );
    assert!(
        limiter.allow(device(3), Timestamp(2_000)),
        "limits are per computer"
    );
    assert!(
        limiter.allow(device(2), Timestamp(61_000)),
        "a minute later it may ask again"
    );
}

#[test]
fn the_owner_sees_recent_questions_newest_first_and_answering_starts_off() {
    let mut log = Vec::new();
    for n in 0..(MAX_ASK_LOG + 5) {
        log_ask(
            &mut log,
            AskRecord {
                asked_by: "Dad".into(),
                computer: "Desktop".into(),
                question: format!("question {n}"),
                answered: 1,
                at: Timestamp(i64::try_from(n).expect("small")),
            },
        );
    }
    assert_eq!(log.len(), MAX_ASK_LOG);
    assert_eq!(log[0].question, format!("question {}", MAX_ASK_LOG + 4));
    assert_eq!(
        FederatedSettings::default(),
        FederatedSettings {
            answer: false,
            ask: false
        }
    );
    let old_record: GroupRecord = serde_json::from_str(
        r#"{"person":null,"person_name":"","computer_name":"","group":null,"membership":[],"knowledge":[],"addresses":[]}"#,
    )
    .expect("records saved before federated recall still load");
    assert_eq!(old_record.federated, FederatedSettings::default());

    let mut view = GroupsView::default();
    view.apply(GroupEvent::Federated {
        settings: FederatedSettings {
            answer: true,
            ask: false,
        },
        recent: log.clone(),
    });
    view.apply(GroupEvent::Answers {
        question: "who is the dentist".into(),
        answers: vec![GroupAnswer {
            statement: "Mum's dentist is Dr Patel".into(),
            from: "Mum".into(),
            computer: "Laptop".into(),
            recorded_at: Timestamp(1),
        }],
    });
    assert!(view.federated.answer);
    assert_eq!(view.recent_asks.len(), MAX_ASK_LOG);
    assert!(view.answers.is_some());
    view.apply(GroupEvent::LeftGroup);
    assert!(view.recent_asks.is_empty() && view.answers.is_none());
}

// ---- lane encryption -----------------------------------------------------

fn grant_body(key: u8, devices: &[u8]) -> KnowledgeBody {
    KnowledgeBody::KeyGrant {
        key: OpHash([key; 32]),
        grants: devices
            .iter()
            .map(|n| (device(*n), Bytes(vec![*n, key])))
            .collect(),
    }
}

#[test]
fn removing_someone_retires_every_key_they_held_and_newcomers_can_be_granted() {
    let mut log = family_of_three();
    let mut shared = Shared::new(&log);
    shared.push(device(1), 3_000, grant_body(1, &[1, 2, 3]));
    let view = shared.view();
    assert_eq!(view.usable_key(&shared.roster), Some(OpHash([1; 32])));
    assert_eq!(view.keys[&OpHash([1; 32])].grants.len(), 3);
    assert!(
        view.keys[&OpHash([1; 32])]
            .missing(&shared.roster)
            .is_empty()
    );

    log.push(
        device(1),
        4_000,
        MembershipBody::RemovePerson { person: person(3) },
    );
    shared.roster = log.state();
    let view = shared.view();
    assert_eq!(
        view.usable_key(&shared.roster),
        None,
        "Sam held it, so it is retired"
    );
    assert!(
        view.keys.contains_key(&OpHash([1; 32])),
        "history under the old key stays readable"
    );

    shared.push(device(1), 5_000, grant_body(2, &[1, 2]));
    let view = shared.view();
    assert_eq!(view.usable_key(&shared.roster), Some(OpHash([2; 32])));

    log.add(4, Role::Member, 6_000);
    shared.roster = log.state();
    let view = shared.view();
    assert_eq!(
        view.keys[&OpHash([2; 32])].missing(&shared.roster),
        std::collections::BTreeSet::from([device(4)]),
        "a newcomer still needs a copy"
    );
    assert_eq!(
        view.usable_key(&shared.roster),
        Some(OpHash([2; 32])),
        "adding someone does not retire a key"
    );
}

#[test]
fn sealed_shares_stay_unreadable_until_opened_then_fold_like_plain_ones() {
    let log = family_of_three();
    let mut shared = Shared::new(&log);
    let sealed = shared.push(
        device(2),
        3_000,
        KnowledgeBody::Sealed {
            key: OpHash([1; 32]),
            ciphertext: Bytes(vec![1, 2, 3]),
        },
    );
    let view = shared.view();
    assert!(view.unreadable.contains(&sealed.hash()));
    assert!(view.details.is_empty());
    assert!(view.rejected.is_empty());
    assert_eq!(
        view.heads,
        vec![sealed.hash()],
        "it still moves the chain on"
    );

    let opened = std::collections::BTreeMap::from([(
        sealed.hash(),
        share_body("The boiler service is on 12 March", Vec::new()),
    )]);
    let view = fold_knowledge_opened(
        &shared.roster,
        shared.lane,
        &shared.ops,
        &FakeVerifier,
        &opened,
    );
    assert!(view.unreadable.is_empty());
    let detail = &view.details[&sealed.hash()];
    assert_eq!(detail.shared_by, "Person 2");
    assert_eq!(view.search("boiler service", 5).len(), 1);

    // Sealing inside sealing, or a grant hidden inside a seal, is refused.
    let nested = std::collections::BTreeMap::from([(sealed.hash(), grant_body(9, &[2]))]);
    let view = fold_knowledge_opened(
        &shared.roster,
        shared.lane,
        &shared.ops,
        &FakeVerifier,
        &nested,
    );
    assert_eq!(
        view.rejected.get(&sealed.hash()),
        Some(&KnowledgeRejection::InvalidDetail)
    );
}

#[test]
fn sealed_bytes_travel_as_hex_and_are_bound_to_their_place() {
    let bytes = Bytes(vec![0, 15, 16, 255]);
    let text = serde_json::to_string(&bytes).expect("encodes");
    assert_eq!(text, "\"000f10ff\"");
    assert_eq!(
        serde_json::from_str::<Bytes>(&text).expect("decodes"),
        bytes
    );
    assert!(serde_json::from_str::<Bytes>("\"0g\"").is_err());
    assert!(serde_json::from_str::<Bytes>("\"abc\"").is_err());

    let log = family_of_three();
    let lane = LaneId::whole_group(&log.group);
    let here = sealed_aad(&log.group, &lane, &OpHash([1; 32]), &device(2));
    assert_ne!(
        here,
        sealed_aad(&log.group, &lane, &OpHash([2; 32]), &device(2))
    );
    assert_ne!(
        here,
        sealed_aad(&log.group, &lane, &OpHash([1; 32]), &device(3))
    );
}
