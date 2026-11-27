use super::*;

#[test]
fn official_models_have_unique_profiles_sources_and_useful_setup() {
    let mut ids = std::collections::BTreeSet::new();
    for board in BOARDS {
        assert!(ids.insert(board.id));
        assert!(board.source.starts_with("https://www.raspberrypi.com/"));
        assert!(
            board
                .setup_url()
                .starts_with("https://www.raspberrypi.com/")
        );
        assert_eq!(board.setup_steps().len(), 5);
        assert!(board.starter_project().contains("```python"));
        assert!(search(board.name, Some(board.family)).contains(&board));
        let setup = PiSetup {
            board_id: board.id.into(),
            host: String::new(),
            user: String::new(),
        };
        assert_eq!(setup.validate(), Ok(board));
        let args = setup.arguments().expect("valid setup");
        assert_eq!(args[0], SERVER_FLAG);
        assert_eq!(
            serde_json::from_str::<PiSetup>(&args[1]).expect("round trip"),
            setup
        );
    }
    assert_eq!(BOARDS.len(), 39);
    for (family, count) in Family::ALL.into_iter().zip([10, 5, 3, 13, 8]) {
        assert_eq!(search("", Some(family)).len(), count, "{family:?}");
    }
    for name in [
        "500+",
        "Zero WH",
        "Compute Module Zero",
        "Pico 2 W with headers",
        "1 Model A",
    ] {
        assert!(!search(name, None).is_empty(), "{name}");
    }
}

#[test]
fn memory_variants_find_their_shared_model_profile() {
    assert!(search("3GB", Some(Family::Computer)).contains(&board("pi-4-b").expect("Pi 4")));
    assert!(search("16GB", Some(Family::Computer)).contains(&board("pi-5").expect("Pi 5")));
}

#[test]
fn connection_targets_cannot_become_options_commands_or_credentials() {
    let mut setup = PiSetup {
        board_id: "pi-5".into(),
        host: "garden.local".into(),
        user: "ada".into(),
    };
    assert!(setup.validate().is_ok());
    for host in [
        "-oProxyCommand=bad",
        "x;id",
        "$(id)",
        "x\ny",
        "user@pi",
        "https://pi",
        "pi/path",
        "..",
        ".local",
        "pi.",
        "pi.-bad",
        "",
    ] {
        setup.host = host.into();
        assert!(setup.validate().is_err(), "{host}");
    }
    setup.host = "192.168.1.20".into();
    for user in ["-bad", "ada;id", "ada\n", "a b", ""] {
        setup.user = user.into();
        assert!(setup.validate().is_err(), "{user}");
    }
    setup.user = "ada".into();
    setup.board_id = "pico".into();
    assert!(setup.validate().is_err());
    setup.board_id = "unknown".into();
    assert!(setup.arguments().is_err());
    assert!(board("unknown").is_none());
}

#[test]
fn pico_projects_do_not_offer_linux_setup_or_external_pin_wiring() {
    let pico = board("pico-w").expect("pico");
    assert!(pico.setup_steps()[2].contains("BOOTSEL"));
    assert!(pico.starter_project().contains("Pin('LED'"));
    assert!(!pico.starter_project().contains("Pin(25"));
    assert!(board("cm-3-lite").expect("CM").setup_steps()[0].contains("carrier"));
    assert_eq!(search("nothing-matches", None), [] as [&Board; 0]);
    assert_eq!(RESEARCHED_ON, "2026-11-26");
}

#[test]
fn keyboard_setup_preserves_preinstalled_systems() {
    for id in ["pi-400", "pi-500", "pi-500-plus"] {
        let keyboard = board(id).expect("keyboard computer");
        let steps = keyboard.setup_steps();
        assert!(steps[0].contains("already installed"));
        assert!(steps[1].starts_with("Only if"));
        assert!(keyboard.setup_url().contains("keyboard-computers"));
    }
}

#[test]
fn local_results_do_not_capture_chat_and_removal_clears_pending_checks() {
    use crate::mcp::{McpEvent, McpService, McpTransport};
    let setup = PiSetup {
        board_id: "pi-5".into(),
        host: String::new(),
        user: String::new(),
    };
    let service = McpService {
        id: "pi".into(),
        transport: McpTransport::Program {
            command: "/sovra".into(),
            args: setup.arguments().expect("args"),
            env_names: vec![],
            working_dir: None,
        },
        ..McpService::default()
    };
    assert_eq!(service_setup(&service), Some(setup));
    assert!(service_setup(&McpService::default()).is_none());
    let mut state = PiResults::default();
    state.begin("pi");
    let mut event = McpEvent::ToolResult {
        service_id: "pi".into(),
        service_name: "Pi".into(),
        tool: "setup_guide".into(),
        text: "steps".into(),
        widget: None,
        session_id: Some("chat".into()),
    };
    state.apply(&event);
    assert!(state.text.is_empty());
    if let McpEvent::ToolResult { session_id, .. } = &mut event {
        *session_id = None;
    }
    state.apply(&event);
    assert_eq!(state.text.get("pi").map(String::as_str), Some("steps"));
    assert!(state.pending.is_empty());
    state.begin("pi");
    state.apply(&McpEvent::Removed { id: "pi".into() });
    assert!(state.pending.is_empty());
    assert!(state.text.is_empty());
}

#[test]
fn raspberry_pi_chat_has_a_persistable_target_without_inventing_connection_success() {
    let mut state = crate::WorkspaceState::new(123);
    assert!(!begin_chat(&mut state, "pi", "unknown"));
    assert!(begin_chat(&mut state, "pi", "pi-5"));
    let message = state.messages.last().unwrap();
    assert!(!message.text.contains("Connected"));
    let encoded = serde_json::to_string(message).unwrap();
    let restored: crate::ChatMessage = serde_json::from_str(&encoded).unwrap();
    assert!(
        matches!(&restored.widgets[0], crate::AgUiWidget::RaspberryPi { service_id, board_id } if service_id == "pi" && board_id == "pi-5")
    );
    assert!(crate::mcp::parse_tool_widget(&serde_json::json!({"structuredContent":{"protocol":"ag_ui","widget":restored.widgets[0]}})).is_none());
}
#[test]
fn gpio_tool_diagrams_are_validated_without_granting_a_device_selection() {
    let diagram = serde_json::json!({"structuredContent":{"protocol":"ag_ui","widget":{"kind":"gpio_diagram","board_id":"pi-5"}}});
    assert_eq!(
        crate::mcp::parse_tool_widget(&diagram),
        Some(crate::AgUiWidget::GpioDiagram {
            board_id: "pi-5".into()
        })
    );
    let unknown = serde_json::json!({"structuredContent":{"protocol":"ag_ui","widget":{"kind":"gpio_diagram","board_id":"unknown"}}});
    assert!(crate::mcp::parse_tool_widget(&unknown).is_none());
    let binding = serde_json::json!({"structuredContent":{"protocol":"ag_ui","widget":{"kind":"raspberry_pi","service_id":"other-device","board_id":"pi-5"}}});
    assert!(crate::mcp::parse_tool_widget(&binding).is_none());
}
