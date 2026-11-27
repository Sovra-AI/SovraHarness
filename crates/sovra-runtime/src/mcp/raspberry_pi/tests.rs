use super::*;
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;

#[tokio::test]
async fn every_profile_serves_real_mcp_tools_and_no_cross_family_device_actions() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for board in sovra_core::raspberry_pi::BOARDS {
            let setup = PiSetup {
                board_id: board.id.into(),
                host: String::new(),
                user: String::new(),
            };
            let encoded = serde_json::to_string(&setup).expect("setup");
            let server = PiServer::new(&encoded).expect("profile");
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            let task = tokio::runtime::Handle::current().spawn(async move {
                server
                    .serve(server_io)
                    .await
                    .expect("server")
                    .waiting()
                    .await
                    .expect("server stopped")
            });
            let client = ().serve(client_io).await.expect("client");
            let tools = client.list_all_tools().await.expect("tools");
            let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
            assert_eq!(names.len(), 9);
            let is_pico = board.family == Family::Pico;
            assert_eq!(names.contains(&"check_usb_connection"), is_pico);
            assert_eq!(names.contains(&"check_health"), !is_pico);
            let result = client
                .call_tool(CallToolRequestParams::new("setup_guide"))
                .await
                .expect("guide");
            assert_ne!(result.is_error, Some(true));
            let result = serde_json::to_value(result).expect("result");
            assert!(result.to_string().contains(board.name));
            let project = client
                .call_tool(CallToolRequestParams::new("starter_project"))
                .await
                .expect("project");
            assert_ne!(project.is_error, Some(true));
            for name in [
                "gpio_guide",
                "led_project",
                "button_project",
                "troubleshoot",
            ] {
                assert!(names.contains(&name));
                let result = client
                    .call_tool(CallToolRequestParams::new(name))
                    .await
                    .expect("GPIO skill");
                assert_ne!(result.is_error, Some(true), "{}: {name}", board.name);
                let text = serde_json::to_value(result).expect("result").to_string();
                assert!(text.contains(board.name));
                if name == "gpio_guide" {
                    assert!(text.contains("not live"));
                    let value: serde_json::Value =
                        serde_json::from_str(&text).expect("tool result");
                    assert_eq!(value["structuredContent"]["protocol"], "ag_ui");
                    assert_eq!(value["structuredContent"]["widget"]["kind"], "gpio_diagram");
                    assert_eq!(value["structuredContent"]["widget"]["board_id"], board.id);
                    assert!(sovra_core::mcp::parse_tool_widget(&value).is_some());
                }
                let rejected = client
                    .call_tool(
                        CallToolRequestParams::new(name).with_arguments(
                            json!({"pin":17,"value":1})
                                .as_object()
                                .expect("object")
                                .clone(),
                        ),
                    )
                    .await
                    .expect("reject pin writes");
                assert_eq!(rejected.is_error, Some(true));
            }
            let invalid = client
                .call_tool(
                    CallToolRequestParams::new("setup_guide").with_arguments(
                        json!({"host":"elsewhere"})
                            .as_object()
                            .expect("object")
                            .clone(),
                    ),
                )
                .await
                .expect("rejected call");
            assert_eq!(invalid.is_error, Some(true));
            client.cancel().await.expect("stop client");
            task.await.expect("join");
        }
    })
    .await
    .expect("bounded test");
}

#[tokio::test]
async fn missing_setup_cannot_claim_a_connection_or_run_commands() {
    let server = PiServer::new(r#"{"board_id":"pi-5","host":"","user":""}"#).expect("profile");
    assert!(
        server
            .answer("check_health")
            .await
            .expect_err("no destination")
            .contains("hasn't been checked")
    );
    assert!(server.answer("run_command").await.is_err());
    assert!(
        PiServer::new(r#"{"board_id":"pi-5","host":"-oProxyCommand=oops","user":"ada"}"#).is_err()
    );
    assert!(PiServer::new(&"x".repeat(2049)).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn client_registration_preserves_tool_choices_pause_and_saved_profiles() {
    use sovra_core::mcp::{McpCommand, McpEvent, McpPhase};
    use sovra_core::{Command, Event};
    use std::os::unix::fs::PermissionsExt as _;
    async fn until(
        events: &async_channel::Receiver<Event>,
        mut pick: impl FnMut(&McpEvent) -> bool,
    ) -> McpEvent {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Event::Mcp(event) = events.recv().await.expect("events")
                    && pick(&event)
                {
                    return event;
                }
            }
        })
        .await
        .expect("MCP event before deadline")
    }

    let directory = tempfile::tempdir().expect("test directory");
    let executable = directory.path().join("fixture-sovra");
    std::fs::write(
        &executable,
        format!("#!/bin/sh\n{}", crate::mcp::tests::FAKE_SERVER),
    )
    .expect("fixture");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("permissions");
    let catalogue = directory.path().join("services.json");
    let start = || {
        crate::start_with_services(
            "pi-test",
            sovra_core::AgentConfig::with_api_key(None),
            crate::RuntimeServices {
                mcp_sample_program: Some(executable.clone()),
                mcp_catalogue: Some(catalogue.clone()),
                ..Default::default()
            },
        )
        .expect("runtime")
    };
    let client = start();
    let events = client.events();
    let setup = PiSetup {
        board_id: "pi-5".into(),
        host: "garden.local".into(),
        user: "ada".into(),
    };
    client
        .send(Command::Mcp(McpCommand::AddRaspberryPi(PiSetup {
            host: "-oProxyCommand=bad".into(),
            ..setup.clone()
        })))
        .expect("invalid request");
    until(
        &events,
        |event| matches!(event, McpEvent::Notice(text) if text.contains("Enter your Pi")),
    )
    .await;
    client
        .send(Command::Mcp(McpCommand::AddRaspberryPi(setup.clone())))
        .expect("add");
    let connected = until(
        &events,
        |event| matches!(event, McpEvent::Upsert(service) if service.phase == McpPhase::Connected),
    )
    .await;
    let McpEvent::Upsert(service) = connected else {
        panic!("connected service");
    };
    assert_eq!(
        sovra_core::raspberry_pi::service_setup(&service),
        Some(setup.clone())
    );
    client
        .send(Command::Mcp(McpCommand::SetToolEnabled {
            id: service.id.clone(),
            tool: "wipe".into(),
            enabled: false,
        }))
        .expect("disable tool");
    until(&events, |event| matches!(event, McpEvent::Upsert(service) if service.disabled_tools.contains(&"wipe".into()))).await;
    client
        .send(Command::Mcp(McpCommand::SetEnabled {
            id: service.id.clone(),
            enabled: false,
        }))
        .expect("pause");
    until(
        &events,
        |event| matches!(event, McpEvent::Upsert(service) if !service.enabled),
    )
    .await;
    client
        .send(Command::Mcp(McpCommand::AddRaspberryPi(setup)))
        .expect("duplicate");
    until(
        &events,
        |event| matches!(event, McpEvent::Notice(text) if text.contains("already in your list")),
    )
    .await;
    tokio::task::spawn_blocking(move || client.shutdown_blocking())
        .await
        .expect("join")
        .expect("shutdown");
    let client = start();
    let events = client.events();
    client.send(Command::Mcp(McpCommand::Load)).expect("load");
    let loaded = until(&events, |event| matches!(event, McpEvent::Loaded(_))).await;
    let McpEvent::Loaded(state) = loaded else {
        panic!("loaded");
    };
    assert_eq!(state.services.len(), 1);
    assert!(!state.services[0].enabled);
    assert_eq!(state.services[0].disabled_tools, ["wipe"]);
    client
        .send(Command::Mcp(McpCommand::Remove { id: service.id }))
        .expect("remove");
    until(&events, |event| matches!(event, McpEvent::Removed { .. })).await;
    tokio::task::spawn_blocking(move || client.shutdown_blocking())
        .await
        .expect("join")
        .expect("shutdown");
}
