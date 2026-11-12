//! Phone input uses native controls while Development mode remains off.
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    Command, Event,
    development::UiRequest,
    group::DeviceId,
    remote_control::{
        ErrorCode, PairedPhone, RemoteCommand, RemoteEvent, RemoteFailure, RemoteState,
    },
};
use sovra_runtime::Client;
use sovra_ui::Shell;
use std::time::{Duration, Instant};

const ID: DeviceId = DeviceId([1; 32]);

fn deliver(cx: &mut TestAppContext, events: &async_channel::Sender<Event>, event: RemoteEvent) {
    events
        .send_blocking(Event::RemoteControl(Box::new(event)))
        .expect("UI event channel");
    cx.executor().advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
}

fn request(
    cx: &mut TestAppContext,
    events: &async_channel::Sender<Event>,
    commands: &async_channel::Receiver<Command>,
    revision: Option<&str>,
    request: UiRequest,
) -> Result<serde_json::Value, RemoteFailure> {
    deliver(
        cx,
        events,
        RemoteEvent::UiRequest {
            generation: ID,
            phone: ID,
            session: ID,
            request_id: 1,
            revision: revision.map(str::to_owned),
            request,
            deadline: Instant::now() + Duration::from_secs(3),
        },
    );
    loop {
        if let Command::RemoteControl(RemoteCommand::UiReply { result, .. }) =
            commands.try_recv().expect("reply")
        {
            return result;
        }
    }
}

fn active() -> RemoteState {
    RemoteState {
        generation: Some(ID),
        connected_phone: Some(ID),
        session: Some(ID),
        phones: vec![PairedPhone {
            device: ID,
            name: "Test phone".into(),
        }],
        ..RemoteState::default()
    }
}

#[gpui_kit::test]
fn phone_composes_a_chat_and_local_stop_rejects_queued_state_and_input(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    assert!(request(cx, &events, &commands, None, UiRequest::Inspect {}).is_err());
    deliver(cx, &events, RemoteEvent::State(active()));
    let snapshot = request(cx, &events, &commands, None, UiRequest::Inspect {}).unwrap();
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
    assert_eq!(
        request(
            cx,
            &events,
            &commands,
            Some("old"),
            UiRequest::Click {
                target: composer.clone()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleObservation
    );
    request(
        cx,
        &events,
        &commands,
        snapshot["revision"].as_str(),
        UiRequest::Click { target: composer },
    )
    .unwrap();
    let snapshot = request(cx, &events, &commands, None, UiRequest::Inspect {}).unwrap();
    request(
        cx,
        &events,
        &commands,
        snapshot["revision"].as_str(),
        UiRequest::Type {
            text: "Phone control test".into(),
        },
    )
    .unwrap();
    let snapshot = request(cx, &events, &commands, None, UiRequest::Inspect {}).unwrap();
    assert!(!snapshot.to_string().contains("Phone control test"));
    request(
        cx,
        &events,
        &commands,
        snapshot["revision"].as_str(),
        UiRequest::Key {
            key: "enter".into(),
        },
    )
    .unwrap();
    cx.update(|cx| {
        assert!(!shell.read(cx).workspace().development.enabled);
        assert!(
            shell
                .read(cx)
                .workspace()
                .messages
                .iter()
                .any(|message| message.text == "Phone control test")
        );
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("remote-control-badge", cx);
        window.render_frame(cx);
        window.click("remote-control-toggle", cx);
    })
    .unwrap();
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| command == Command::RemoteControl(RemoteCommand::Stop))
    );
    // An earlier network status was already queued when Stop was clicked locally.
    deliver(cx, &events, RemoteEvent::State(active()));
    assert_eq!(
        request(cx, &events, &commands, None, UiRequest::Inspect {})
            .unwrap_err()
            .code,
        ErrorCode::Disabled
    );
}

#[gpui_kit::test]
fn remote_control_skill_starts_without_a_model(cx: &mut TestAppContext) {
    let (client, commands, _) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        Root::new(cx.new(|cx| Shell::new(0, window, cx)), window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("chat-composer-input", cx);
        window.input("/remote-control", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.find("remote-control-section").visible());
    })
    .unwrap();
    let commands: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(commands.contains(&Command::RemoteControl(RemoteCommand::Start)));
    assert!(!commands.iter().any(|command| matches!(
        command,
        Command::RunPrompt { .. } | Command::RunWithFiles { .. }
    )));
}
