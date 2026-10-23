//! Headless UI tests: real views in a GPUI test window, runtime replaced by loopback channels.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{AgUiWidget, Command, Event, NavTab, RuntimeStatus, ScreenRoute};
use sovra_runtime::Client;
use sovra_ui::Shell;

#[gpui_kit::test]
fn status_follows_runtime_events(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status(), &RuntimeStatus::Starting);
    });

    events
        .send_blocking(Event::Ready {
            version: "1.2.3".into(),
        })
        .unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status().label(), "Ready · v1.2.3");
    });

    events.send_blocking(Event::Stopped).unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status(), &RuntimeStatus::Stopped);
    });
}

#[gpui_kit::test]
fn main_chat_login_schedules_and_agui_surfaces_work_end_to_end(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell_entity = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell_entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell_entity.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("sovra-main-workspace").visible());
        assert!(window.find("chat-hero-banner").visible());
        assert!(window.find("agui-build-progress-card").visible());
        assert!(window.find("sovra-inspector").visible());

        // Click "Preview my AI" in the right inspector -> transitions building state to Ready.
        window.click("preview-my-ai-btn", cx);
        assert!(!shell.read(cx).workspace().ai_building);

        // Switch to Schedules tab -> renders Schedules Command Center.
        window.click("nav-schedules", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Schedules);
        assert!(window.find("sovra-schedules-view").visible());

        // Run a schedule in chat -> switches back to Chat and dispatches Command::RunPrompt.
        window.click(("schedule-run", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);

        // Sign out via user profile -> renders Login screen, then sign back in.
        window.click("sidebar-user-profile", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::Login);
        assert!(window.find("sovra-login-screen").visible());

        window.click("login-submit-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::MainWorkspace);
    })
    .unwrap();

    // Verify the schedule run dispatched a real Command::RunPrompt to the runtime.
    let dispatched = commands.recv_blocking().unwrap();
    assert!(matches!(dispatched, Command::RunPrompt { .. }));

    // Stream an AG-UI surface and text chunk back from the runtime.
    events
        .send_blocking(Event::AgUiSurface {
            session_id: "personal-ai-setup".into(),
            widget: AgUiWidget::SurfaceCard {
                surface_id: "personal-ai-summary".into(),
                title: "AG-UI Surface · personal-ai-summary".into(),
                summary: "Updated structured AG-UI surface (revision v1).".into(),
            },
        })
        .unwrap();
    events
        .send_blocking(Event::AgentChunk {
            session_id: "personal-ai-setup".into(),
            text: "Morning brief completed.".into(),
        })
        .unwrap();
    events
        .send_blocking(Event::AgentCompleted {
            session_id: "personal-ai-setup".into(),
        })
        .unwrap();
    cx.run_until_parked();

    shell.read_with(cx, |shell, _| {
        assert!(!shell.workspace().is_generating);
        let last = shell.workspace().messages.last().unwrap();
        assert!(last.text.contains("Morning brief completed."));
        assert_eq!(last.widgets.len(), 1);
    });
}
