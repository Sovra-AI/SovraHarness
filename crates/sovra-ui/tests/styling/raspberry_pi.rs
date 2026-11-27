//! Pi setup and real result controls stay usable at the minimum size and 200%.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AppContext as _, ScrollDelta, TestAppContext, Window, point, px, size};
use sovra_core::mcp::{
    McpCommand, McpEvent, McpPhase, McpService, McpState, McpToolInfo, McpTransport,
};
use sovra_core::models::InterfaceScale;
use sovra_core::raspberry_pi::PiSetup;
use sovra_core::{Command, Event, NavTab};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn reveal(window: &mut Window, id: &'static str, cx: &mut App) {
    for _ in 0..800 {
        window.render_frame(cx);
        let item = window.find(id);
        let bounds = item.bounds();
        let pane = window.find("connections-scroll").bounds();
        if item.visible() && bounds.top() >= pane.top() && bounds.bottom() <= pane.bottom() {
            return;
        }
        let delta = if bounds.top() < pane.top() { 24. } else { -24. };
        window.scroll(
            "connections-scroll",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
    panic!("could not reach {id}");
}

fn reveal_chat(window: &mut Window, id: impl Into<gpui_kit::ElementId>, cx: &mut App) {
    let id = id.into();
    for _ in 0..240 {
        window.render_frame(cx);
        let Some(item) = window.try_find(id.clone()) else {
            // Later replies are not rendered until the native list reaches them.
            window.scroll(
                "chat-scroll-area",
                ScrollDelta::Pixels(point(px(0.), px(-100.))),
                cx,
            );
            continue;
        };
        let bounds = item.bounds();
        let pane = window.find("chat-scroll-area").bounds();
        if item.visible() && bounds.top() >= pane.top() && bounds.bottom() <= pane.bottom() {
            return;
        }
        let delta = if bounds.top() < pane.top() { 24. } else { -24. };
        window.scroll(
            "chat-scroll-area",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
    panic!(
        "could not reach chat control {id:?}: item {:?}, pane {:?}",
        window.find(id.clone()).bounds(),
        window.find("chat-scroll-area").bounds()
    );
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn pi_discovery_selects_a_host_and_recovers_from_empty_or_failed_searches(cx: &mut TestAppContext) {
    use sovra_core::raspberry_pi::{DiscoveryEvent, NetworkCandidate};
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                Root::new(shell, window, cx)
            });
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "connection-action-web-address", cx);
                window.click("connection-action-web-address", cx);
                reveal(window, "mcp-mode-pi", cx);
                window.click("mcp-mode-pi", cx);
                window.render_frame(cx);
                assert_eq!(
                    window.find("mcp-connect-section-title").label(),
                    Some("Raspberry Pi")
                );
                assert!(window.try_find("mcp-add-sample").is_none());
                assert!(window.try_find("mcp-mode-web").is_none());
                reveal(window, "pi-model-pi-5", cx);
                window.click("pi-model-pi-5", cx);
                reveal(window, "pi-find", cx);
                assert_eq!(window.find("pi-find").disabled(), Some(true));
            })
            .expect("start discovery");
            let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(
                sent.iter().any(|command| matches!(
                    command,
                    Command::Mcp(McpCommand::DiscoverRaspberryPis)
                ))
            );
            assert!(
                !sent
                    .iter()
                    .any(|command| matches!(command, Command::Mcp(McpCommand::AddRaspberryPi(_))))
            );
            for result in [Err("Couldn't search this network.".into()), Ok(vec![])] {
                events
                    .send_blocking(Event::Mcp(McpEvent::RaspberryPiDiscovery(
                        DiscoveryEvent::Finished(result),
                    )))
                    .expect("result");
                settle(cx);
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, "pi-find", cx);
                    assert_ne!(window.find("pi-find").disabled(), Some(true));
                    assert!(window.find("pi-discovery-status").label().is_some());
                    window.click("pi-find", cx);
                })
                .expect("retry");
                assert!(
                    std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                        command,
                        Command::Mcp(McpCommand::DiscoverRaspberryPis)
                    ))
                );
            }
            events
                .send_blocking(Event::Mcp(McpEvent::RaspberryPiDiscovery(
                    DiscoveryEvent::Finished(Ok(vec![
                        NetworkCandidate::from_hostname("gardenpi.local.").expect("host"),
                    ])),
                )))
                .expect("found");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-nearby-gardenpi.local", cx);
                let bounds = window.find("pi-nearby-gardenpi.local").bounds();
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                window.click("pi-nearby-gardenpi.local", cx);
                reveal(window, "pi-user", cx);
                window.click("pi-user", cx);
                window.input("gardener", cx);
                reveal(window, "pi-add-plugin", cx);
                window.click("pi-add-plugin", cx);
            })
            .expect("use discovered host");
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Mcp(McpCommand::AddRaspberryPi(setup)) if setup.host == "gardenpi.local" && setup.user == "gardener" && setup.board_id == "pi-5")));
        }
    }
}

#[gpui_kit::test]
fn pi_plugins_fit_and_route_through_mcp_at_every_supported_layout(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("shell");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                reveal(window, "connection-action-web-address", cx);
                window.click("connection-action-web-address", cx);
                reveal(window, "mcp-mode-pi", cx);
                window.click("mcp-mode-pi", cx);
                reveal(window, "pi-family-Pico", cx);
                window.click("pi-family-Pico", cx);
                reveal(window, "pi-model-pico-2-w-headers", cx);
                let choice = window.find("pi-model-pico-2-w-headers");
                assert!(choice.bounds().left() >= px(0.) && choice.bounds().right() <= px(width));
                assert_eq!(choice.label(), Some("Raspberry Pi Pico 2 W with headers"));
                window.click("pi-model-pico-2-w-headers", cx);
                reveal(window, "pi-add-plugin", cx);
                assert!(window.find("pi-add-plugin").bounds().right() <= px(width));
                assert!(
                    window.try_find("pi-host").is_none(),
                    "Pico must not ask for a Linux destination"
                );
                window.click("pi-add-plugin", cx);
            })
            .expect("window");
            let emitted: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(emitted.iter().any(|command| matches!(command, Command::Mcp(McpCommand::AddRaspberryPi(setup)) if setup.board_id == "pico-2-w-headers" && setup.host.is_empty())));

            let setup = PiSetup {
                board_id: "pico-2-w-headers".into(),
                host: String::new(),
                user: String::new(),
            };
            let service = McpService {
                id: "test-pi".into(),
                name: "My Pico".into(),
                phase: McpPhase::Connected,
                transport: McpTransport::Program {
                    command: "/sovra".into(),
                    args: setup.arguments().expect("args"),
                    env_names: vec![],
                    working_dir: None,
                },
                tools: [
                    "connect_device",
                    "deploy_project",
                    "setup_guide",
                    "starter_project",
                    "check_usb_connection",
                    "gpio_guide",
                    "led_project",
                    "button_project",
                    "troubleshoot",
                ]
                .map(|name| McpToolInfo {
                    name: name.into(),
                    description: name.into(),
                    input_schema: serde_json::json!({"type":"object"}),
                })
                .to_vec(),
                ..McpService::default()
            };
            events
                .send_blocking(Event::Mcp(McpEvent::Loaded(McpState {
                    services: vec![service],
                    notice: None,
                })))
                .expect("loaded");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-details-test-pi", cx);
                assert!(
                    window.try_find("pi-check_usb_connection-test-pi").is_none(),
                    "details should start collapsed"
                );
                window.click("pi-details-test-pi", cx);
                reveal(window, "pi-check_usb_connection-test-pi", cx);
                window.click("pi-check_usb_connection-test-pi", cx);
            })
            .expect("check click");
            let emitted: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(emitted.iter().any(|command| matches!(command, Command::Mcp(McpCommand::CallTool { service_id, tool, session_id: None, .. }) if service_id == "test-pi" && tool == "check_usb_connection")));
            events
                .send_blocking(Event::Mcp(McpEvent::ToolResult {
                    service_id: "test-pi".into(),
                    service_name: "My Pico".into(),
                    tool: "check_usb_connection".into(),
                    text: "No Raspberry Pi USB serial device was found.".into(),
                    widget: None,
                    session_id: None,
                }))
                .expect("result");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-result-test-pi", cx);
                assert_eq!(
                    window.find("pi-result-test-pi").label(),
                    Some("No Raspberry Pi USB serial device was found.")
                );
                assert!(window.find("pi-result-test-pi").bounds().right() <= px(width));
                reveal(window, "pi-save-test-pi", cx);
                window.click("pi-save-test-pi", cx);
            })
            .expect("result screen");
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Artifacts(sovra_core::artifacts::ArtifactCommand::Save(draft)) if draft.content == "No Raspberry Pi USB serial device was found.")));
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-starter_project-test-pi", cx);
                window.click("pi-starter_project-test-pi", cx);
            })
            .expect("project click");
            events
                .send_blocking(Event::Mcp(McpEvent::ToolResult {
                    service_id: "test-pi".into(),
                    service_name: "My Pico".into(),
                    tool: "starter_project".into(),
                    text: sovra_core::raspberry_pi::board("pico-2-w-headers")
                        .expect("profile")
                        .starter_project(),
                    widget: None,
                    session_id: None,
                }))
                .expect("project result");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-save-code", cx);
                assert!(window.find("pi-save-code").bounds().right() <= px(width));
                window.click("pi-save-code", cx);
            })
            .expect("code actions");
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Artifacts(sovra_core::artifacts::ArtifactCommand::Save(draft)) if draft.kind == sovra_core::artifacts::ArtifactKind::Code && draft.language == "python" && draft.content.contains("Pin('LED'") && !draft.content.contains("```"))));
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-gpio_guide-test-pi", cx);
                window.click("pi-gpio_guide-test-pi", cx);
            })
            .expect("GPIO click");
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Mcp(McpCommand::CallTool { tool, arguments, .. }) if tool == "gpio_guide" && arguments == serde_json::json!({}))));
            events
                .send_blocking(Event::Mcp(McpEvent::ToolResult {
                    service_id: "test-pi".into(),
                    service_name: "My Pico".into(),
                    tool: "gpio_guide".into(),
                    text: sovra_core::raspberry_pi::board("pico-2-w-headers")
                        .expect("board")
                        .gpio_guide(),
                    widget: None,
                    session_id: None,
                }))
                .expect("GPIO result");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-pin-test-pi-pico-11", cx);
                let pin = window.find("pi-pin-test-pi-pico-11");
                assert!(pin.bounds().left() >= px(0.) && pin.bounds().right() <= px(width));
                assert!(pin.label().expect("pin label").contains("GPIO 8"));
                window.click("pi-pin-test-pi-pico-11", cx);
                window.render_frame(cx);
                assert!(
                    window
                        .find("pi-pin-detail-test-pi-pico")
                        .label()
                        .expect("explanation")
                        .contains("Use 8 in code, not 11")
                );
                reveal(window, "pi-pin-test-pi-pico-40", cx);
                assert!(
                    window
                        .find("pi-pin-test-pi-pico-40")
                        .label()
                        .expect("power label")
                        .contains("not a programmable GPIO")
                );
                window.click("pi-pin-test-pi-pico-40", cx);
            })
            .expect("pin map");
            assert!(
                commands.try_recv().is_err(),
                "looking at a pin must not send hardware commands"
            );
            for (id, tool) in [
                ("pi-led_project-test-pi", "led_project"),
                ("pi-button_project-test-pi", "button_project"),
                ("pi-troubleshoot-test-pi", "troubleshoot"),
            ] {
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, id, cx);
                    window.click(id, cx);
                })
                .expect("skill click");
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Mcp(McpCommand::CallTool { tool: sent, .. }) if sent == tool)));
                events
                    .send_blocking(Event::Mcp(McpEvent::ToolResult {
                        service_id: "test-pi".into(),
                        service_name: "My Pico".into(),
                        tool: tool.into(),
                        text: if tool == "led_project" {
                            sovra_core::raspberry_pi::board("pico-2-w-headers")
                                .expect("board")
                                .led_project()
                        } else {
                            "Project instructions".into()
                        },
                        widget: None,
                        session_id: None,
                    }))
                    .expect("result");
                settle(cx);
                if tool == "led_project" {
                    cx.update_window(window.into(), |_, window, cx| {
                        reveal(window, "pi-save-code", cx);
                        window.click("pi-save-code", cx);
                    })
                    .expect("save LED project");
                    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Artifacts(sovra_core::artifacts::ArtifactCommand::Save(draft)) if draft.title.contains("Blink an external light") && draft.content.contains("Pin(15, Pin.OUT"))));
                }
            }
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-chat-test-pi", cx);
                window.click("pi-chat-test-pi", cx);
            })
            .expect("open Pi chat");
            settle(cx);
            let session = shell.read_with(cx, |shell, _| {
                let state = shell.workspace();
                assert_eq!(state.active_tab, NavTab::Chat);
                assert!(state.messages.iter().flat_map(|message| &message.widgets).any(|widget| matches!(widget, sovra_core::AgUiWidget::RaspberryPi { service_id, board_id } if service_id == "test-pi" && board_id == "pico-2-w-headers")));
                state.session_id.clone()
            });
            let emitted: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(emitted.iter().any(|command| matches!(command, Command::Mcp(McpCommand::SelectRaspberryPi { service_id, session_id }) if service_id == "test-pi" && session_id == &session)));
            assert!(emitted.iter().any(|command| matches!(command, Command::Mcp(McpCommand::CallTool { tool, session_id: Some(id), .. }) if tool == "connect_device" && id == &session)));
            cx.update_window(window.into(), |_, window, cx| {
                reveal_chat(window, ("pi-chat-gpio", 0_usize), cx);
                window.click(("pi-chat-gpio", 0_usize), cx);
            })
            .expect("expand chat GPIO");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal_chat(window, "pi-pin-test-pi-pico-11", cx);
                let pin = window.find("pi-pin-test-pi-pico-11");
                assert!(pin.bounds().left() >= px(0.) && pin.bounds().right() <= px(width));
                assert!(pin.label().unwrap().contains("GPIO 8"));
                window.click("pi-pin-test-pi-pico-11", cx);
                window.render_frame(cx);
                assert!(
                    window
                        .find("pi-pin-detail-test-pi-pico")
                        .label()
                        .unwrap()
                        .contains("Use 8 in code, not 11")
                );
            })
            .expect("GPIO in chat");
            events
                .send_blocking(Event::Mcp(McpEvent::ToolResult {
                    service_id: "test-pi".into(),
                    service_name: "My Pico".into(),
                    tool: "gpio_guide".into(),
                    text: "Here is your Pico's pin diagram.".into(),
                    widget: Some(sovra_core::AgUiWidget::GpioDiagram {
                        board_id: "pico-2-w-headers".into(),
                    }),
                    session_id: Some(session),
                }))
                .expect("GPIO tool response");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal_chat(window, "pi-pin-reference-10-pico-11", cx);
                let pin = window.find("pi-pin-reference-10-pico-11");
                assert!(pin.bounds().left() >= px(0.) && pin.bounds().right() <= px(width));
                window.click("pi-pin-reference-10-pico-11", cx);
                window.render_frame(cx);
                assert!(
                    window
                        .find("pi-pin-detail-reference-10-pico")
                        .label()
                        .expect("pin detail")
                        .contains("Use 8 in code, not 11")
                );
            })
            .expect("tool diagram appears without an expand action");
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::Mcp(
                        McpCommand::SelectRaspberryPi { .. } | McpCommand::CallTool { .. }
                    )
                )),
                "reference pins must not initiate a connection or device action"
            );
        }
    }
}

#[gpui_kit::test]
fn pi_pairing_uses_private_input_and_explicit_identity_confirmation(cx: &mut TestAppContext) {
    use sovra_core::raspberry_pi::PairingEvent;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                Root::new(shell, window, cx)
            });
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                for id in [
                    "connection-action-web-address",
                    "mcp-mode-pi",
                    "pi-model-pi-5",
                ] {
                    reveal(window, id, cx);
                    window.click(id, cx);
                }
                for (id, value) in [
                    ("pi-host", "gardenpi.local"),
                    ("pi-user", "gardener"),
                    ("pi-password", "private-test-password"),
                ] {
                    reveal(window, id, cx);
                    window.click(id, cx);
                    window.input(value, cx);
                }
                reveal(window, "pi-add-plugin", cx);
                window.click("pi-add-plugin", cx);
            })
            .expect("submit private sign-in");
            let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(!format!("{sent:?}").contains("private-test-password"));
            let setup = sent
                .into_iter()
                .find_map(|command| match command {
                    Command::Mcp(McpCommand::PairRaspberryPi { setup, password }) => {
                        assert_eq!(password.0, "private-test-password");
                        Some(setup)
                    }
                    _ => None,
                })
                .expect("pairing command");
            events
                .send_blocking(Event::Mcp(McpEvent::RaspberryPiPairing(
                    PairingEvent::Started {
                        request_id: 7,
                        setup,
                    },
                )))
                .expect("started");
            events
                .send_blocking(Event::Mcp(McpEvent::RaspberryPiPairing(
                    PairingEvent::ConfirmIdentity {
                        request_id: 7,
                        fingerprint: "SHA256:only-a-test-fingerprint".into(),
                    },
                )))
                .expect("confirm");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-trust-identity", cx);
                let bounds = window.find("pi-trust-identity").bounds();
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                assert_eq!(window.find("pi-change-model").disabled(), Some(true));
                window.click("pi-trust-identity", cx);
            })
            .expect("confirm observed identity");
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::Mcp(McpCommand::ConfirmPiIdentity {
                        request_id: 7,
                        accept: true
                    })
                ))
            );
            events
                .send_blocking(Event::Mcp(McpEvent::RaspberryPiPairing(
                    PairingEvent::Finished {
                        request_id: 7,
                        result: Err("The Pi did not accept that password.".into()),
                    },
                )))
                .expect("failed");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "pi-pairing-status", cx);
                assert_eq!(
                    window.find("pi-pairing-status").label(),
                    Some("The Pi did not accept that password.")
                );
                reveal(window, "pi-password", cx);
                window.click("pi-password", cx);
                window.input("replacement", cx);
                reveal(window, "pi-add-plugin", cx);
                window.click("pi-add-plugin", cx);
            })
            .expect("retry");
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Mcp(McpCommand::PairRaspberryPi { password, .. }) if password.0 == "replacement")));
        }
    }
}
