//! Settings → Groups: start/join forms, the roster and shared knowledge, end to end
//! through the shell with a scripted runtime.

use std::collections::BTreeSet;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind,
};
use sovra_core::group::{
    AskRecord, DetailStatus, DeviceId, FederatedSettings, GroupAnswer, GroupCommand, GroupEvent,
    GroupId, GroupProblem, GroupState, MembershipBody, OpHash, PersonId, Role, SharedArtifact,
    SharedDetail, Signature, SignatureVerifier, UnsignedOp, fold,
};
use sovra_core::models::InterfaceScale;
use sovra_core::{Command, Event, NavTab, Timestamp};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

/// Rosters are folded for real; only signature checking is waived in this UI test.
struct AcceptAll;

impl SignatureVerifier for AcceptAll {
    fn verify(&self, _: &DeviceId, _: &[u8], _: &Signature) -> bool {
        true
    }
}

const MUM: DeviceId = DeviceId([1; 32]);
const DAD: DeviceId = DeviceId([2; 32]);

fn family() -> GroupState {
    let nonce = [9; 16];
    let group = GroupId::derive(&MUM, &nonce);
    let unsigned = UnsignedOp {
        group,
        author: MUM,
        seq: 1,
        prev: None,
        deps: Vec::new(),
        at: Timestamp(1),
        body: MembershipBody::Genesis {
            name: "Family".into(),
            nonce,
            founder: PersonId([10; 32]),
            founder_name: "Mum".into(),
            device_name: "Mum's laptop".into(),
        },
    };
    let mut ops = vec![unsigned.sign(Signature([0; 64]))];
    for body in [
        MembershipBody::AddPerson {
            person: PersonId([20; 32]),
            name: "Dad".into(),
            role: Role::Member,
        },
        MembershipBody::AddDevice {
            person: PersonId([20; 32]),
            device: DAD,
            name: "Dad's desktop".into(),
        },
    ] {
        let draft = fold(group, &ops, &AcceptAll).draft(MUM, Timestamp(2), body);
        ops.push(draft.sign(Signature([0; 64])));
    }
    fold(group, &ops, &AcceptAll)
}

fn dads_detail() -> SharedDetail {
    SharedDetail {
        id: OpHash([7; 32]),
        text: "The boiler service is booked for 12 March".into(),
        about: Vec::new(),
        author: DAD,
        person: Some(PersonId([20; 32])),
        shared_by: "Dad".into(),
        computer: "Dad's desktop".into(),
        shared_at: Timestamp(3),
        valid_at: None,
        replaces: Vec::new(),
        status: DetailStatus::Current,
        agreed_by: BTreeSet::new(),
        from_former_member: false,
        space: sovra_core::group::Space::Group,
    }
}

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    WindowHandle<Root>,
    Entity<Shell>,
    async_channel::Receiver<Command>,
    async_channel::Sender<Event>,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    settle(cx);
    (window, shell.expect("shell opened"), commands, events)
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn group_event(events: &async_channel::Sender<Event>, event: GroupEvent) {
    events
        .send_blocking(Event::Group(Box::new(event)))
        .expect("headless test fixture is available");
}

fn drain(commands: &async_channel::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

#[gpui_kit::test]
fn a_computer_outside_a_group_can_start_one_or_join_one(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: None,
        },
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(window.find("groups-start").visible());
        assert!(window.find("groups-join").visible());
        window.click("groups-create", cx);
        window.click("groups-join-button", cx);
    })
    .expect("window open");
    let sent = drain(&commands);
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::Group(GroupCommand::Create { .. })))
    );
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::Group(GroupCommand::Join { .. })))
    );
}

#[gpui_kit::test]
fn the_group_shows_who_is_in_it_and_what_was_shared(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(&events, GroupEvent::Knowledge(vec![dads_detail()]));
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(window.find("groups-roster").visible());
        assert!(window.find(("groups-person", 0usize)).visible());
        assert!(window.find(("groups-person", 1usize)).visible());
        assert!(window.find(("groups-detail", 0usize)).visible());
        assert!(
            window.find("groups-invite-member").visible(),
            "Mum is an admin"
        );
        window.click("groups-share", cx);
        window.click(("groups-agree", 0usize), cx);
        window.click("groups-invite-computer", cx);
    })
    .expect("window open");
    let sent = drain(&commands);
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::Group(GroupCommand::Share { .. })))
    );
    assert!(sent.contains(&Command::Group(GroupCommand::Agree {
        detail: OpHash([7; 32])
    })));
    assert!(sent.contains(&Command::Group(GroupCommand::InviteComputer)));

    // An invite link appears with a Copy button.
    group_event(
        &events,
        GroupEvent::InviteReady {
            link: "sovra://join/example".into(),
            expires_at: Timestamp(i64::MAX / 2),
            qr: vec![vec![true, false], vec![false, true]],
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("groups-invite-link").visible());
        assert!(window.find("groups-copy-link").visible());
        assert!(
            window.find("groups-invite-qr").visible(),
            "the link as a QR code"
        );
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("sovra://join/example".into()),
            "the confirmed invite is copied automatically"
        );
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("other text".into()));
        window.click("groups-copy-link", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("sovra://join/example".into())
        );
    })
    .expect("window open");
}

#[gpui_kit::test]
fn groups_fit_the_minimum_window_a_wide_window_and_double_text(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (window, shell, _commands, events) = open(cx, width, height);
            group_event(
                &events,
                GroupEvent::Loaded {
                    this_computer: MUM,
                    group: Some(family()),
                },
            );
            group_event(&events, GroupEvent::Knowledge(vec![dads_detail()]));
            group_event(&events, GroupEvent::Audit(audit_summary()));
            settle(cx);
            group_event(
                &events,
                GroupEvent::InviteReady {
                    link: format!("sovra://join/{}", "a".repeat(1000)),
                    expires_at: Timestamp(i64::MAX / 2),
                    qr: Vec::new(),
                },
            );
            settle(cx);
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
                window.render_frame(cx);
                let column = window.find("settings-column").bounds();
                for id in [
                    "settings-groups",
                    "groups-roster",
                    "groups-knowledge",
                    "groups-invites",
                    "groups-copy-link",
                    "groups-invite-member",
                    "groups-federated",
                    "groups-audit",
                    "groups-audit-headline",
                ] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.size.width <= column.size.width + px(1.),
                        "{width}x{height}@{scale}: {id} is wider than the settings column"
                    );
                    assert!(
                        bounds.origin.x >= column.origin.x - px(1.),
                        "{width}x{height}@{scale}: {id} starts outside the settings column"
                    );
                }
            })
            .expect("window open");
        }
    }
}

fn audit_summary() -> sovra_core::swarm::audit::AuditSummary {
    sovra_core::swarm::audit::AuditSummary {
        records: 2,
        checked: true,
        fingerprint: "3f2a9c01".into(),
        shared: 1,
        given_out: 1,
        recent: vec![
            sovra_core::swarm::audit::AuditLine {
                at: Timestamp(6),
                words: format!("Gave a memory to {}'s helpers", "Dad ".repeat(80)),
            },
            sovra_core::swarm::audit::AuditLine {
                at: Timestamp(5),
                words: "Shared a detail with the group".into(),
            },
        ],
    }
}

#[gpui_kit::test]
fn the_owner_sees_this_computers_checked_history(cx: &mut TestAppContext) {
    let (window, shell, _commands, events) = open(cx, 1280., 3200.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(&events, GroupEvent::Audit(audit_summary()));
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(window.find("groups-audit").visible());
        assert!(window.find("groups-audit-headline").visible());
        assert!(window.find(("groups-audit-line", 1usize)).visible());
    })
    .expect("window open");
}

fn dads_list() -> SharedArtifact {
    SharedArtifact {
        id: OpHash([8; 32]),
        title: "Shopping list".into(),
        kind: ArtifactKind::Checklist,
        language: String::new(),
        content: "- milk\n- eggs".into(),
        author: DAD,
        person: Some(PersonId([20; 32])),
        shared_by: "Dad".into(),
        computer: "Dad's desktop".into(),
        shared_at: Timestamp(4),
        replaces: Vec::new(),
        status: DetailStatus::Current,
        from_former_member: false,
        origin: "art-dad".into(),
        space: sovra_core::group::Space::Group,
    }
}

fn mums_letter() -> Artifact {
    Artifact::from_draft(
        "art-letter".into(),
        ArtifactDraft {
            title: "Letter to the school".into(),
            kind: ArtifactKind::Document,
            language: String::new(),
            content: "Dear Ms Smith,".into(),
            source: sovra_core::artifacts::ArtifactSource::default(),
        },
        Timestamp(5),
    )
}

fn in_group_with_artifacts(events: &async_channel::Sender<Event>) {
    group_event(
        events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(events, GroupEvent::Artifacts(vec![dads_list()]));
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![mums_letter()])))
        .expect("headless test fixture is available");
}

#[gpui_kit::test]
fn artifacts_can_be_shared_with_the_group_and_shared_ones_kept(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 1600.);
    in_group_with_artifacts(&events);
    settle(cx);
    drain(&commands);

    // Dad's shared list shows under "Shared in Family"; Mum keeps a copy.
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        assert!(window.find("shared-artifacts").visible());
        window.click(("shared-artifact-open", 0usize), cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("shared-artifact-detail").visible());
        assert!(
            window.find("shared-artifact-withdraw").visible(),
            "Dad or an admin may stop sharing, and Mum is an admin"
        );
        window.click("shared-artifact-keep", cx);
    })
    .expect("window open");
    let sent = drain(&commands);
    let kept = sent.iter().find_map(|command| match command {
        Command::Artifacts(ArtifactCommand::Save(draft)) => Some(draft.clone()),
        _ => None,
    });
    let kept = kept.expect("Keep a copy saves to the library");
    assert_eq!(kept.title, "Shopping list");
    assert_eq!(kept.source.author, "Dad");
    assert_eq!(kept.source.shared_from, OpHash([8; 32]).to_hex());

    // Mum's own letter offers "Share with Family" and sends it with its library id.
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("art-letter", cx));
        window.render_frame(cx);
        window.click("artifact-share-group", cx);
    })
    .expect("window open");
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Group(GroupCommand::ShareArtifact {
            local_id,
            new_version_of: None,
            ..
        }) if local_id == "art-letter"
    )));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("artifact-share-feedback").visible());
    })
    .expect("window open");
}

#[gpui_kit::test]
fn shared_artifacts_fit_the_minimum_window_a_wide_window_and_double_text(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, _commands, events) = open(cx, width, height);
        in_group_with_artifacts(&events);
        settle(cx);
        cx.update(|cx| design::set_interface_scale(InterfaceScale::from_percent(scale), cx));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
            window.render_frame(cx);
            let page = window.find("sovra-artifacts-view").bounds();
            let shared = window.find("shared-artifacts").bounds();
            assert!(
                shared.size.width <= page.size.width + px(1.),
                "{width}x{height}@{scale}: shared artifacts overflow the page"
            );
            assert!(shared.origin.x >= page.origin.x - px(1.));
        })
        .expect("window open");
    }
}

#[gpui_kit::test]
fn group_members_can_ask_each_others_memories_with_consent_and_a_log(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 3200.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(
        &events,
        GroupEvent::Federated {
            settings: FederatedSettings::default(),
            recent: vec![AskRecord {
                asked_by: "Dad".into(),
                computer: "Dad's desktop".into(),
                question: "when is bin day".into(),
                answered: 1,
                at: Timestamp(5),
            }],
        },
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(window.find("groups-federated").visible());
        assert!(
            window.find(("groups-asked", 0usize)).visible(),
            "the owner sees who asked"
        );
        window.click("groups-federated-answer", cx);
        window.click("groups-ask-input", cx);
        window.input("who is our dentist", cx);
        window.click("groups-ask", cx);
    })
    .expect("window open");
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.contains(&Command::Group(GroupCommand::SetFederated(
        FederatedSettings {
            answer: true,
            ask: false,
        }
    ))));
    assert!(sent.contains(&Command::Group(GroupCommand::AskGroup {
        question: "who is our dentist".into()
    })));

    group_event(
        &events,
        GroupEvent::Answers {
            question: "who is our dentist".into(),
            answers: vec![GroupAnswer {
                statement: "Dad's dentist is Dr Shah".into(),
                from: "Dad".into(),
                computer: "Dad's desktop".into(),
                recorded_at: Timestamp(3),
            }],
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(("groups-answer", 0usize)).visible());
    })
    .expect("window open");
}

#[gpui_kit::test]
fn a_detail_can_go_to_just_one_person_and_private_ones_say_so(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    let mut private = dads_detail();
    private.space = sovra_core::group::Space::pair(PersonId([10; 32]), PersonId([20; 32]));
    group_event(&events, GroupEvent::Knowledge(vec![private]));
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        let tag = window.find(("groups-detail-space", 0usize));
        assert!(tag.visible());
        assert_eq!(tag.label(), Some("Just you and Dad"));
        window.click(("groups-share-just", 0usize), cx);
        window.render_frame(cx);
        window.click("groups-share", cx);
    })
    .expect("window open");
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Group(GroupCommand::Share { with: Some(person), .. }) if *person == PersonId([20; 32])
    )));
}

#[gpui_kit::test]
fn same_room_codes_show_digits_and_ask_both_people_to_compare_pictures(cx: &mut TestAppContext) {
    // The joining side: no group yet, types a code.
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: DAD,
            group: None,
        },
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        window.click("groups-join-code", cx);
        window.input("123 456", cx);
        window.click("groups-join-code-button", cx);
    })
    .expect("window open");
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Group(GroupCommand::JoinWithCode { code, .. }) if code == "123 456"
    )));

    let pictures = sovra_core::group::short_auth_string(b"transcript", &MUM, &DAD);
    group_event(
        &events,
        GroupEvent::Pairing(sovra_core::group::PairingStep::PeerConnected),
    );
    group_event(
        &events,
        GroupEvent::Pairing(sovra_core::group::PairingStep::CodeReady(pictures.clone())),
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("groups-picture-check").visible());
        assert_eq!(
            window.find("groups-pictures").label(),
            Some(pictures.spoken().as_str()),
            "the pictures are read aloud by name"
        );
        window.click("groups-pictures-match", cx);
    })
    .expect("window open");
    settle(cx);
    assert!(
        drain(&commands).contains(&Command::Group(GroupCommand::ConfirmCode { matches: true }))
    );

    // The inviting side shows its code.
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(
        &events,
        GroupEvent::ShortCodeReady {
            code: "654 321".into(),
            expires_at: Timestamp(i64::MAX / 2),
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("groups-short-code").label(), Some("654 321"));
    })
    .expect("window open");
}

#[gpui_kit::test]
fn a_shared_helper_or_schedule_becomes_your_own_copy_when_you_choose(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 3200.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    let template = |n: u8, template| sovra_core::group::SharedTemplate {
        id: OpHash([n; 32]),
        template,
        author: DAD,
        person: Some(PersonId([20; 32])),
        shared_by: "Dad".into(),
        computer: "Dad's desktop".into(),
        shared_at: Timestamp(i64::from(n)),
        replaces: Vec::new(),
        status: DetailStatus::Current,
        from_former_member: false,
        space: sovra_core::group::Space::Group,
    };
    group_event(
        &events,
        GroupEvent::Templates(vec![
            template(
                31,
                sovra_core::group::Template::Helper {
                    name: "Gardener".into(),
                    icon: "🌱".into(),
                    role: "Garden helper".into(),
                    tagline: "Knows our garden".into(),
                    instructions: "Water the roses on Sundays.".into(),
                    expertise: Vec::new(),
                },
            ),
            template(
                30,
                sovra_core::group::Template::Schedule {
                    name: "Bin reminder".into(),
                    cadence: "every Monday at 7pm".into(),
                    prompt: "Remind us to put the bins out.".into(),
                },
            ),
        ]),
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(
            window.find(("groups-template-words", 0usize)).visible(),
            "read before use"
        );
        window.click(("groups-template-use", 0usize), cx);
        window.click(("groups-template-use", 1usize), cx);
    })
    .expect("window open");
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Teams(sovra_core::teams::TeamCommand::SaveMember(member))
            if member.name == "Gardener" && member.id.starts_with("shared-") && !member.builtin
    )));
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Schedule(sovra_core::ScheduleCommand::Create { name, send: None, .. })
            if name == "Bin reminder"
    )));
}

#[gpui_kit::test]
fn your_own_helper_can_be_shared_from_the_team_page(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    events
        .send_blocking(Event::Teams(sovra_core::teams::TeamEvent::Loaded {
            members: vec![sovra_core::teams::TeamMember {
                id: "gardener".into(),
                name: "Gardener".into(),
                icon: "🌱".into(),
                role: "Garden helper".into(),
                tagline: "Knows our garden".into(),
                instructions: "Water the roses on Sundays.".into(),
                expertise: Vec::new(),
                conductor: false,
                active: true,
                builtin: false,
            }],
        }))
        .expect("headless test fixture is available");
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        window.click(("team-member-share", 0usize), cx);
    })
    .expect("window open");
    settle(cx);
    assert!(drain(&commands).iter().any(|command| matches!(
        command,
        Command::Group(GroupCommand::ShareTemplate {
            template: sovra_core::group::Template::Helper { name, .. },
            with: None,
        }) if name == "Gardener"
    )));
}

#[gpui_kit::test]
fn a_shared_skill_is_kept_without_tools_until_you_grant_them(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(
        &events,
        GroupEvent::Templates(vec![sovra_core::group::SharedTemplate {
            id: OpHash([40; 32]),
            template: sovra_core::group::Template::Skill {
                name: "Plan a weekly menu".into(),
                description: "Makes a week of dinners from what is in the fridge.".into(),
                instructions: "Ask what is in the fridge, then plan seven dinners.".into(),
            },
            author: DAD,
            person: Some(PersonId([20; 32])),
            shared_by: "Dad".into(),
            computer: "Dad's desktop".into(),
            shared_at: Timestamp(5),
            replaces: Vec::new(),
            status: DetailStatus::Current,
            from_former_member: false,
            space: sovra_core::group::Space::Group,
        }]),
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        window.click(("groups-template-use", 0usize), cx);
    })
    .expect("window open");
    settle(cx);
    assert!(drain(&commands).iter().any(|command| matches!(
        command,
        Command::Skills(sovra_core::skills::SkillCommand::Save(skill))
            if skill.title == "Plan a weekly menu"
                && skill.tools.is_empty()
                && !skill.auto_apply
                && !skill.builtin
    )));
}

#[gpui_kit::test]
fn this_computers_ai_can_be_offered_to_the_group_and_its_jobs_are_listed(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 3600.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    group_event(
        &events,
        GroupEvent::Work {
            settings: sovra_core::group::WorkSettings::default(),
            recent: vec![sovra_core::group::WorkRecord {
                asked_by: "Dad".into(),
                computer: "Old laptop".into(),
                at: Timestamp(1),
                answered: true,
            }],
        },
    );
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find(("groups-work-job", 0usize)).label(),
            Some("Dad (Old laptop) · answered")
        );
        window.click("groups-work-help", cx);
    })
    .expect("window open");
    settle(cx);
    assert!(
        drain(&commands).contains(&Command::Group(GroupCommand::SetWork(
            sovra_core::group::WorkSettings {
                help_others: true,
                use_group: false,
            }
        )))
    );
}

#[gpui_kit::test]
fn member_invite_copies_only_after_confirmation_and_recovers_from_errors(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 2400.);
    group_event(
        &events,
        GroupEvent::Loaded {
            this_computer: MUM,
            group: Some(family()),
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("keep me".into()));
        window.click("groups-invite-name", cx);
        window.input("Sam", cx);
        window.render_frame(cx);
        window.click("groups-invite-member", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("keep me".into())
        );
        window.render_frame(cx);
        assert_eq!(window.find("groups-invite-member").disabled(), Some(true));
    })
    .expect("window");
    assert!(
        drain(&commands).contains(&Command::Group(GroupCommand::InvitePerson {
            name: "Sam".into(),
            role: Role::Member
        }))
    );
    group_event(&events, GroupEvent::Problem(GroupProblem::Unreachable));
    settle(cx);
    group_event(
        &events,
        GroupEvent::InviteReady {
            link: "sovra://join/unrequested".into(),
            expires_at: Timestamp(i64::MAX / 2),
            qr: Vec::new(),
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_ne!(window.find("groups-invite-member").disabled(), Some(true));
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("keep me".into())
        );
        window.click("groups-invite-member", cx);
    })
    .expect("retry");
    group_event(
        &events,
        GroupEvent::InviteReady {
            link: "sovra://join/confirmed".into(),
            expires_at: Timestamp(i64::MAX / 2),
            qr: Vec::new(),
        },
    );
    settle(cx);
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("sovra://join/confirmed".into())
        );
    });
}
