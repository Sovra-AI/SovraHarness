//! Native UI automation uses the same channel contract as the local MCP bridge.
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    Command, Event, NavTab,
    development::{DevelopmentCommand, DevelopmentEvent, DevelopmentState, UiRequest},
};
use sovra_runtime::Client;
use sovra_ui::Shell;
use std::time::{Duration, Instant};

fn deliver(
    cx: &mut TestAppContext,
    events: &async_channel::Sender<Event>,
    event: DevelopmentEvent,
) {
    events
        .send_blocking(Event::Development(event))
        .expect("UI event channel");
    cx.executor().advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
}

fn request(
    cx: &mut TestAppContext,
    events: &async_channel::Sender<Event>,
    commands: &async_channel::Receiver<Command>,
    generation: &str,
    request: UiRequest,
) -> Result<serde_json::Value, String> {
    deliver(
        cx,
        events,
        DevelopmentEvent::UiRequest {
            generation: generation.into(),
            request_id: 7,
            request,
            deadline: Instant::now() + Duration::from_secs(3),
        },
    );
    loop {
        if let Command::Development(DevelopmentCommand::UiReply {
            request_id, result, ..
        }) = commands.try_recv().expect("UI must answer every request")
        {
            assert_eq!(request_id, 7);
            return result;
        }
    }
}

#[gpui_kit::test]
fn inspect_click_and_keyboard_drive_real_controls_and_reject_stale_requests(
    cx: &mut TestAppContext,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    assert!(request(cx, &events, &commands, "session", UiRequest::Inspect {}).is_err());
    deliver(
        cx,
        &events,
        DevelopmentEvent::State(DevelopmentState {
            enabled: true,
            generation: Some("session".into()),
            error: None,
            connection_command: "sovra --development-mcp".into(),
        }),
    );
    let snapshot = request(cx, &events, &commands, "session", UiRequest::Inspect {}).unwrap();
    assert_eq!(snapshot["input_values"], "omitted for privacy");
    let composer = snapshot["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| {
            element["target"]
                .as_str()
                .unwrap()
                .ends_with("Name(\"chat-composer-input\")]")
        })
        .unwrap()["target"]
        .as_str()
        .unwrap()
        .to_owned();
    request(
        cx,
        &events,
        &commands,
        "session",
        UiRequest::Click { target: composer },
    )
    .unwrap();
    request(
        cx,
        &events,
        &commands,
        "session",
        UiRequest::Type {
            text: "Development test 🌱".into(),
        },
    )
    .unwrap();
    let typed = request(cx, &events, &commands, "session", UiRequest::Inspect {}).unwrap();
    assert!(
        !typed.to_string().contains("Development test"),
        "input values stay private"
    );
    request(
        cx,
        &events,
        &commands,
        "session",
        UiRequest::Key {
            key: "enter".into(),
        },
    )
    .unwrap();
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .workspace()
                .messages
                .iter()
                .any(|message| message.text == "Development test 🌱")
        );
    });
    let target = snapshot["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["target"].as_str().unwrap().contains("nav-settings"))
        .unwrap()["target"]
        .as_str()
        .unwrap()
        .to_owned();
    request(
        cx,
        &events,
        &commands,
        "session",
        UiRequest::Click { target },
    )
    .unwrap();
    cx.update(|cx| assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Settings));
    let snapshot = request(cx, &events, &commands, "session", UiRequest::Inspect {}).unwrap();
    assert!(
        snapshot["elements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|element| element["target"]
                .as_str()
                .unwrap()
                .contains("settings-development"))
    );
    request(
        cx,
        &events,
        &commands,
        "session",
        UiRequest::Key { key: "tab".into() },
    )
    .unwrap();
    assert!(
        request(
            cx,
            &events,
            &commands,
            "stale",
            UiRequest::Click {
                target: "anything".into()
            }
        )
        .is_err()
    );
    assert!(
        request(
            cx,
            &events,
            &commands,
            "session",
            UiRequest::Click {
                target: "missing".into()
            }
        )
        .is_err()
    );
    deliver(
        cx,
        &events,
        DevelopmentEvent::State(DevelopmentState::default()),
    );
    assert!(request(cx, &events, &commands, "session", UiRequest::Inspect {}).is_err());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("development-mode-badge").is_none());
    })
    .unwrap();
}
