//! The connect screen sends commands and only shows what the runtime reports.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AppContext as _, ScrollDelta, TestAppContext, Window, point, px, size};
use sovra_core::mcp::{McpCommand, McpEvent, McpPhase, McpService, McpToolInfo};
use sovra_core::{Command, Event};
use sovra_runtime::Client;
use sovra_ui::Shell;

fn open_shell(cx: &mut TestAppContext, client: Client) -> gpui_kit::WindowHandle<Root> {
    cx.update(|cx| sovra_ui::init(client, cx));
    cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        Root::new(cx.new(|cx| Shell::new(window, cx)), window, cx)
    })
}

fn reveal(window: &mut Window, id: &'static str, cx: &mut App) {
    for _ in 0..100 {
        window.render_frame(cx);
        let item = window.find(id);
        let bounds = item.bounds();
        let viewport = window.find("settings-sections").bounds();
        if item.visible() && bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom()
        {
            return;
        }
        let delta = if bounds.top() < viewport.top() {
            60.
        } else {
            -60.
        };
        window.scroll(
            "settings-sections",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
    panic!("{id} must be reachable in Settings");
}

fn service(phase: McpPhase, message: &str, tools: Vec<McpToolInfo>) -> McpService {
    McpService {
        id: "recipe-box".into(),
        name: "Recipe box".into(),
        address: "https://recipes.example.com".into(),
        phase,
        message: message.into(),
        tools,
    }
}

fn apply(cx: &mut TestAppContext, events: &async_channel::Sender<Event>, event: Event) {
    events.send_blocking(event).expect("event delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn connect_screen_sends_commands_and_shows_sign_in_then_connected(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let window = open_shell(cx, client);
    apply(
        cx,
        &events,
        Event::Ready {
            version: "test".into(),
        },
    );
    let startup: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        startup
            .iter()
            .any(|command| matches!(command, Command::Mcp(McpCommand::Load))),
        "opening the app asks the runtime for saved services"
    );

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-settings", cx);
        window.render_frame(cx);
        reveal(window, "mcp-connect-section", cx);
        reveal(window, "mcp-service-address", cx);
        assert_eq!(
            window.find("mcp-service-address").label(),
            Some("Web address")
        );
        reveal(window, "mcp-connect", cx);
        assert_eq!(window.find("mcp-connect").label(), Some("Connect"));
        assert_eq!(window.find("mcp-connect").disabled(), Some(true));
        window.click("mcp-service-name", cx);
        window.input("Recipe box", cx);
        window.click("mcp-service-address", cx);
        window.input("https://recipes.example.com", cx);
        reveal(window, "mcp-connect", cx);
        assert_ne!(window.find("mcp-connect").disabled(), Some(true));
        window.click("mcp-connect", cx);
        assert!(window.try_find("mcp-service-recipe-box").is_none());
        assert!(window.try_find("mcp-status-recipe-box").is_none());
    })
    .expect("window exists");

    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter().any(|command| matches!(
            command,
            Command::Mcp(McpCommand::Connect {
                name,
                address,
                secret: None,
            }) if name == "Recipe box" && address == "https://recipes.example.com"
        )),
        "Connect must send the typed address instead of marking the service connected"
    );

    apply(
        cx,
        &events,
        Event::Mcp(McpEvent::Upsert(service(
            McpPhase::NeedsSignIn,
            "This service needs you to sign in.",
            Vec::new(),
        ))),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        reveal(window, "mcp-sign-in-recipe-box", cx);
        assert_eq!(
            window.find("mcp-sign-in-recipe-box").label(),
            Some("Sign in")
        );
        assert_eq!(
            window.find("mcp-message-recipe-box").label(),
            Some("This service needs you to sign in.")
        );
        assert!(window.try_find("mcp-status-recipe-box").is_none());
    })
    .expect("window exists");

    apply(
        cx,
        &events,
        Event::Mcp(McpEvent::Upsert(service(
            McpPhase::Connected,
            "Connected.",
            vec![McpToolInfo {
                name: "echo".into(),
                description: "Repeat a short note.".into(),
                input_schema: serde_json::Value::Null,
            }],
        ))),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        reveal(window, "mcp-status-recipe-box", cx);
        assert_eq!(
            window.find("mcp-status-recipe-box").label(),
            Some("Connected")
        );
        reveal(window, "mcp-tool-recipe-box-echo", cx);
        assert_eq!(
            window.find("mcp-tool-recipe-box-echo").label(),
            Some("Repeat a short note.")
        );
        reveal(window, "mcp-remove-recipe-box", cx);
        assert_eq!(window.find("mcp-remove-recipe-box").label(), Some("Remove"));
        assert!(window.try_find("mcp-sign-in-recipe-box").is_none());
        window.click("mcp-remove-recipe-box", cx);
        assert!(window.find("mcp-service-recipe-box").visible());
    })
    .expect("window exists");

    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter().any(|command| matches!(
            command,
            Command::Mcp(McpCommand::Remove { id }) if id == "recipe-box"
        )),
        "Remove must ask the runtime to drop the service"
    );

    apply(
        cx,
        &events,
        Event::Mcp(McpEvent::Removed {
            id: "recipe-box".into(),
        }),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mcp-service-recipe-box").is_none());
        assert!(window.try_find("mcp-status-recipe-box").is_none());
    })
    .expect("window exists");
}
