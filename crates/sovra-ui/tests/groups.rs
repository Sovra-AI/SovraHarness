//! Settings → Groups: start/join forms, the roster and shared knowledge, end to end
//! through the shell with a scripted runtime.

use std::collections::BTreeSet;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use sovra_core::group::{
    DetailStatus, DeviceId, GroupCommand, GroupEvent, GroupId, GroupState, MembershipBody, OpHash,
    PersonId, Role, SharedDetail, Signature, SignatureVerifier, UnsignedOp, fold,
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
        },
    );
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("groups-invite-link").visible());
        assert!(window.find("groups-copy-link").visible());
    })
    .expect("window open");
}

#[gpui_kit::test]
fn groups_fit_the_minimum_window_a_wide_window_and_double_text(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, _commands, events) = open(cx, width, height);
        group_event(
            &events,
            GroupEvent::Loaded {
                this_computer: MUM,
                group: Some(family()),
            },
        );
        group_event(&events, GroupEvent::Knowledge(vec![dads_detail()]));
        settle(cx);
        cx.update(|cx| design::set_interface_scale(InterfaceScale::from_percent(scale), cx));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
            window.render_frame(cx);
            let column = window.find("settings-column").bounds();
            for id in ["settings-groups", "groups-roster", "groups-knowledge"] {
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
