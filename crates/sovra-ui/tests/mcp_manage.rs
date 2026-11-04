//! Managing many connected services: review before a program runs, tool and pause
//! switches, search, pasting settings and adding a program, at the minimum
//! window, a wide window and 200% interface size, in both themes.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AppContext as _, ScrollDelta, TestAppContext, Window, point, px, size};
use sovra_core::mcp::{
    McpCommand, McpEvent, McpPhase, McpService, McpState, McpToolInfo, McpTransport,
};
use sovra_core::models::InterfaceScale;
use sovra_core::{Command, Event, NavTab};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

/// Scrolls Connections until `id` is on screen (fully, unless it is taller than the pane).
fn reveal(window: &mut Window, id: &'static str, cx: &mut App) -> bool {
    for _ in 0..600 {
        window.render_frame(cx);
        let Some(item) = window.try_find(id) else {
            return false;
        };
        let bounds = item.bounds();
        let pane = window.find("connections-scroll").bounds();
        let inside = bounds.top() >= pane.top() && bounds.bottom() <= pane.bottom();
        let taller = bounds.size.height > pane.size.height;
        let overlaps = bounds.top() < pane.bottom() && bounds.bottom() > pane.top();
        if item.visible() && (inside || (taller && overlaps)) {
            return true;
        }
        let delta = if bounds.top() < pane.top() { 24. } else { -24. };
        window.scroll(
            "connections-scroll",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
    false
}

fn tool(name: &str, description: &str) -> McpToolInfo {
    McpToolInfo {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::Value::Null,
    }
}

fn services() -> McpState {
    let mut list = vec![
        McpService {
            id: "files".into(),
            name: "Files".into(),
            address: "npx -y @modelcontextprotocol/server-filesystem /home/ada/Documents".into(),
            phase: McpPhase::NeedsApproval,
            message: McpPhase::NeedsApproval.label().into(),
            transport: McpTransport::Program {
                command: "npx".into(),
                args: vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                    "/home/ada/Documents".into(),
                ],
                env_names: vec!["GITHUB_TOKEN".into()],
                working_dir: None,
            },
            ..McpService::default()
        },
        McpService {
            id: "notes".into(),
            name: "Notes".into(),
            address: "https://notes.example.com/mcp".into(),
            phase: McpPhase::Connected,
            message: "Connected.".into(),
            tools: vec![
                tool("search", "Find a note"),
                tool("delete", "Delete a note"),
            ],
            disabled_tools: vec!["delete".into()],
            ..McpService::default()
        },
        McpService {
            id: "archive".into(),
            name: "Archive".into(),
            address: "https://archive.example.com/mcp".into(),
            enabled: false,
            ..McpService::default()
        },
    ];
    for (id, name) in [("cal", "Calendar"), ("crm", "Customers"), ("wiki", "Wiki")] {
        list.push(McpService {
            id: id.into(),
            name: name.into(),
            address: format!("https://{id}.example.com/mcp"),
            phase: McpPhase::Connected,
            message: "Connected.".into(),
            tools: vec![tool("look_up", "Look something up")],
            ..McpService::default()
        });
    }
    McpState {
        services: list,
        notice: None,
    }
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn sent(commands: &async_channel::Receiver<Command>) -> Vec<McpCommand> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Mcp(command) => Some(command),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn many_services_can_be_reviewed_switched_searched_and_added(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let label = format!("{mode:?} {width}x{height} at {scale}%");
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
            events
                .send_blocking(Event::Mcp(McpEvent::Loaded(services())))
                .expect("event");
            settle(cx);
            let _ = sent(&commands);

            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                // At 200% in the smallest window the rail is collapsed; open the
                // tab directly as the messaging test does.
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                window.render_frame(cx);
                assert!(
                    reveal(window, "connection-action-web-address", cx),
                    "{label}: card"
                );
                window.click("connection-action-web-address", cx);
                window.render_frame(cx);

                assert!(reveal(window, "mcp-summary", cx), "{label}: summary");
                assert_eq!(
                    window.find("mcp-summary").label(),
                    None,
                    "the summary is plain text"
                );
                // The program waits for an OK, showing its exact command.
                assert!(
                    reveal(window, "mcp-review-command-files", cx),
                    "{label}: review"
                );
                assert!(reveal(window, "mcp-approve-files", cx), "{label}: run it");
                assert!(window.find("mcp-approve-files").bounds().right() <= px(width));
                window.click("mcp-approve-files", cx);

                // Tools start folded; the count reflects switched-off tools.
                assert!(
                    reveal(window, "mcp-tools-toggle-notes", cx),
                    "{label}: tools"
                );
                assert_eq!(
                    window.find("mcp-tools-toggle-notes").label(),
                    Some("1 of 2 tools on")
                );
                window.click("mcp-tools-toggle-notes", cx);
                window.render_frame(cx);
                assert!(
                    reveal(window, "mcp-tool-switch-notes-delete", cx),
                    "{label}: tool switch"
                );
                window.click("mcp-tool-switch-notes-delete", cx);
                assert!(
                    reveal(window, "mcp-enabled-notes", cx),
                    "{label}: pause switch"
                );
                window.click("mcp-enabled-notes", cx);
                assert!(reveal(window, "mcp-status-archive", cx));
                assert_eq!(window.find("mcp-status-archive").label(), Some("Paused"));
            })
            .expect("window");
            let commands_sent = sent(&commands);
            assert!(
                commands_sent.contains(&McpCommand::Approve { id: "files".into() }),
                "{label}"
            );
            assert!(
                commands_sent.contains(&McpCommand::SetToolEnabled {
                    id: "notes".into(),
                    tool: "delete".into(),
                    enabled: true,
                }),
                "{label}"
            );
            assert!(
                commands_sent.contains(&McpCommand::SetEnabled {
                    id: "notes".into(),
                    enabled: false,
                }),
                "{label}"
            );

            cx.update_window(window.into(), |_, window, cx| {
                // Search narrows a long list to what matches, tools included.
                assert!(reveal(window, "mcp-search", cx), "{label}: search");
                window.click("mcp-search", cx);
                window.input("archive", cx);
                window.render_frame(cx);
                assert!(window.try_find("mcp-service-archive").is_some());
                assert!(
                    window.try_find("mcp-service-notes").is_none(),
                    "{label}: filtered"
                );
                window.click("mcp-search", cx);
                for _ in 0.."archive".len() {
                    window.press("backspace", cx);
                }
                window.render_frame(cx);
                assert!(
                    window.try_find("mcp-service-notes").is_some(),
                    "{label}: cleared"
                );

                // Paste settings: a bad paste is explained and nothing is sent.
                assert!(reveal(window, "mcp-add-open", cx), "{label}: add");
                window.click("mcp-add-open", cx);
                window.render_frame(cx);
                assert!(reveal(window, "mcp-mode-paste", cx));
                window.click("mcp-mode-paste", cx);
                window.render_frame(cx);
                assert!(reveal(window, "mcp-paste", cx), "{label}: paste box");
                window.click("mcp-paste", cx);
                window.input("npx a && rm -rf ~", cx);
                assert!(reveal(window, "mcp-import", cx));
                window.click("mcp-import", cx);
                window.render_frame(cx);
                assert!(
                    reveal(window, "mcp-form-problem", cx),
                    "{label}: problem shown"
                );
            })
            .expect("window");
            assert!(
                !sent(&commands)
                    .iter()
                    .any(|c| matches!(c, McpCommand::Import { .. })),
                "{label}: a bad paste is not sent"
            );

            cx.update_window(window.into(), |_, window, cx| {
                // A program typed as a command, with a private setting.
                assert!(reveal(window, "mcp-mode-program", cx));
                window.click("mcp-mode-program", cx);
                window.render_frame(cx);
                assert!(
                    reveal(window, "mcp-program-command", cx),
                    "{label}: command box"
                );
                assert_eq!(window.find("mcp-add-program").disabled(), Some(true));
                window.click("mcp-program-command", cx);
                window.input("uvx mcp-server-git --repository /home/ada/code", cx);
                assert!(reveal(window, "mcp-program-settings", cx));
                window.click("mcp-program-settings", cx);
                window.input("GIT_TOKEN=abc123", cx);
                assert!(reveal(window, "mcp-add-program", cx));
                assert_ne!(window.find("mcp-add-program").disabled(), Some(true));
                window.click("mcp-add-program", cx);
                window.remove_window();
            })
            .expect("window");
            assert!(
                sent(&commands).contains(&McpCommand::AddProgram {
                    name: String::new(),
                    command: "uvx".into(),
                    args: vec![
                        "mcp-server-git".into(),
                        "--repository".into(),
                        "/home/ada/code".into()
                    ],
                    env: vec![("GIT_TOKEN".into(), "abc123".into())],
                    working_dir: None,
                }),
                "{label}: program added with its setting"
            );
        }
    }
}
