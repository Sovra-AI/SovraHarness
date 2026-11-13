//! Training controls, acknowledgements and readable layouts in both themes.
use super::knowledge_profile::settle;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
use sovra_core::{
    Command, Event, NavTab,
    group::{DeviceId, GroupId},
    models::InterfaceScale,
    training::{
        HelpingPhase, TrainingCommand, TrainingEvent, TrainingGroup, TrainingHelp, TrainingPeer,
        TrainingPhase, TrainingProgress, TrainingSource, TrainingState,
    },
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    for _ in 0..180 {
        window.render_frame(cx);
        let element = window.find(id);
        let bounds = element.bounds();
        let pane = window.find("training-page").bounds();
        assert!(
            bounds.left() >= pane.left() - px(1.) && bounds.right() <= pane.right() + px(1.),
            "{id} overflows: {bounds:?} vs {pane:?}"
        );
        if element.visible() && bounds.top() >= pane.top() && bounds.bottom() <= pane.bottom() {
            return;
        }
        let direction = if bounds.top() < pane.top() { 32. } else { -32. };
        window.scroll(
            "training-page",
            ScrollDelta::Pixels(point(px(0.), px(direction))),
            cx,
        );
    }
    panic!("{id} could not be reached");
}
fn take(commands: &async_channel::Receiver<Command>) -> TrainingCommand {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|c| match c {
            Command::Training(c) => Some(c),
            _ => None,
        })
        .expect("training command")
}

#[gpui_kit::test]
fn training_start_pause_resume_and_saved_state_fit_each_layout(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Training, cx);
                    shell
                });
                Root::new(shell, window, cx)
            });
            assert_eq!(take(&commands), TrainingCommand::Load);
            let mut state = TrainingState {
                phase: TrainingPhase::Ready,
                ..TrainingState::default()
            };
            events
                .send_blocking(Event::Training(TrainingEvent(state.clone())))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "training-start", cx);
                window.click("training-start", cx);
                window.render_frame(cx);
                window.click("training-start", cx);
            })
            .unwrap();
            assert_eq!(
                take(&commands),
                TrainingCommand::Start(TrainingSource::Sample)
            );
            assert!(
                commands.try_recv().is_err(),
                "pending start must not submit twice"
            );
            state.phase = TrainingPhase::Running;
            state.progress = TrainingProgress {
                rounds: 2,
                total_rounds: 80,
                tokens: 400,
                loss_milli: Some(4123),
            };
            state.saved_at = Some(format!(
                "/test/very-long-training-folder/{}/run-model.json",
                "a".repeat(150)
            ));
            events
                .send_blocking(Event::Training(TrainingEvent(state.clone())))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "training-pause", cx);
                window.click("training-pause", cx);
            })
            .unwrap();
            assert_eq!(take(&commands), TrainingCommand::Pause);
            state.phase = TrainingPhase::Paused;
            events
                .send_blocking(Event::Training(TrainingEvent(state.clone())))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "training-resume", cx);
                window.click("training-resume", cx);
            })
            .unwrap();
            assert_eq!(take(&commands), TrainingCommand::Resume);
            state.phase = TrainingPhase::Complete;
            events
                .send_blocking(Event::Training(TrainingEvent(state.clone())))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "training-copy-path", cx);
                // A finished run cannot be resumed.
                reveal(window, "training-resume", cx);
                window.click("training-resume", cx);
                reveal(window, "training-copy-path", cx);
                window.click("training-copy-path", cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    state.saved_at.clone().unwrap()
                );
                reveal(window, "training-own-text", cx);
                window.click("training-own-text", cx);
                window.render_frame(cx);
                assert!(
                    window
                        .elements()
                        .iter()
                        .any(|element| element.label() == Some("Text for training"))
                );
            })
            .unwrap();
            assert!(
                commands.try_recv().is_err(),
                "completed training must not resume"
            );
            exercise_grid(window, &events, &commands, cx);
        }
    }
}

fn exercise_grid(
    window: gpui_kit::WindowHandle<Root>,
    events: &async_channel::Sender<Event>,
    commands: &async_channel::Receiver<Command>,
    cx: &mut TestAppContext,
) {
    let peer = TrainingPeer {
        device: DeviceId([2; 32]),
        name: "A family member's computer with a very long friendly name".repeat(3),
    };
    let mut state = TrainingState {
        phase: TrainingPhase::Ready,
        group: Some(TrainingGroup {
            id: GroupId([1; 32]),
            name: "Our family training group".into(),
            this_device: DeviceId([1; 32]),
            peers: vec![peer.clone()],
        }),
        ..TrainingState::default()
    };
    let publish = |state: &TrainingState, cx: &mut TestAppContext| {
        events
            .send_blocking(Event::Training(TrainingEvent(state.clone())))
            .expect("training test window and event channel remain open");
        settle(cx);
    };
    publish(&state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "training-sample", cx);
        window.click("training-sample", cx);
        reveal(window, "training-start-shared", cx);
        window.click("training-start-shared", cx);
    })
    .expect("training test window and event channel remain open");
    assert_eq!(
        take(commands),
        TrainingCommand::StartShared(TrainingSource::Sample)
    );
    state.phase = TrainingPhase::Running;
    state.sharing = Some(GroupId([1; 32]));
    state.helped_rounds = 2;
    publish(&state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "training-sharing-status", cx);
        assert!(
            window
                .find("training-sharing-status")
                .label()
                .expect("sharing status has an accessible label")
                .contains("2 rounds")
        );
        reveal(window, "training-stop-sharing", cx);
        window.click("training-stop-sharing", cx);
    })
    .expect("training test window and event channel remain open");
    assert_eq!(take(commands), TrainingCommand::StopSharing);
    state.phase = TrainingPhase::Ready;
    state.sharing = None;
    publish(&state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(
            window,
            "training-help-0202020202020202020202020202020202020202020202020202020202020202",
            cx,
        );
        window.click(
            "training-help-0202020202020202020202020202020202020202020202020202020202020202",
            cx,
        );
    })
    .expect("training test window and event channel remain open");
    assert_eq!(take(commands), TrainingCommand::Help(peer.device));
    state.helping = Some(TrainingHelp {
        group: GroupId([1; 32]),
        peer,
        phase: HelpingPhase::Waiting,
        accepted_rounds: 3,
        progress: None,
        notice: Some("The other computer is offline. Waiting for it to return.".into()),
    });
    publish(&state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "training-start", cx);
        window.click("training-start", cx);
        reveal(window, "training-helper-status", cx);
        reveal(window, "training-stop-help", cx);
        window.click("training-stop-help", cx);
    })
    .expect("training test window and event channel remain open");
    assert_eq!(take(commands), TrainingCommand::Pause);
    assert!(
        commands.try_recv().is_err(),
        "local training stays disabled while helping"
    );
}
