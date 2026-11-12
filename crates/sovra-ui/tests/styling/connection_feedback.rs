//! Connection failures and retry controls remain reachable at every supported size.
use super::*;
use sovra_core::models::{ModelConnection, ModelEvent, ModelPreferences, ProviderKind};
use sovra_core::{Event, NavTab};

fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    for _ in 0..60 {
        window.render_frame(cx);
        let pane = window.find("connections-scroll").bounds();
        let item = window.find(id).bounds();
        assert!(
            item.left() >= pane.left() && item.right() <= pane.right(),
            "{id}: {item:?} outside {pane:?}"
        );
        if item.top() >= pane.top() && item.bottom() <= pane.bottom() {
            return;
        }
        let delta = if item.top() < pane.top() {
            pane.top() - item.top()
        } else {
            pane.bottom() - item.bottom()
        };
        window.scroll(
            "connections-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id} must be reachable");
}

#[gpui_kit::test]
fn connection_errors_and_retry_fit_each_size(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                Root::new(shell, window, cx)
            });
            let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
            connection.environment_credential = true;
            events
                .send_blocking(Event::Models(ModelEvent::Loaded {
                    connections: vec![connection],
                    preferences: ModelPreferences {
                        appearance: if mode.is_dark() {
                            sovra_core::models::Appearance::Dark
                        } else {
                            sovra_core::models::Appearance::Light
                        },
                        interface_scale: sovra_core::models::InterfaceScale::from_percent(scale),
                        ..ModelPreferences::default()
                    },
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            for action in [
                "connection-action-online-ai",
                "connection-action-compatible",
            ] {
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, action, cx);
                    window.click(action, cx);
                    reveal(window, "connection-test", cx);
                    window.click("connection-test", cx);
                })
                .unwrap();
                events.send_blocking(Event::Models(ModelEvent::Error(
                    "Could not reach the AI service. Check the address and your internet connection, then try again. Your entries are still here.".into(),
                ))).unwrap();
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(16));
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, "connection-model-error", cx);
                    reveal(window, "connection-test", cx);
                    assert_ne!(window.find("connection-test").disabled(), Some(true));
                    window.click("connection-test", cx);
                })
                .unwrap();
                events
                    .send_blocking(Event::Models(ModelEvent::Notice(
                        "Connection checked successfully.".into(),
                    )))
                    .unwrap();
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(16));
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, "connection-model-notice", cx);
                    reveal(window, "connections-back", cx);
                    window.click("connections-back", cx);
                })
                .unwrap();
            }
            let checks = std::iter::from_fn(|| commands.try_recv().ok())
                .filter(|command| {
                    matches!(
                        command,
                        sovra_core::Command::Models(
                            sovra_core::models::ModelCommand::TestConnection { .. }
                        )
                    )
                })
                .count();
            assert_eq!(checks, 4);
            cx.update(|cx| {
                design::set_interface_scale(sovra_core::models::InterfaceScale::default(), cx);
            });
        }
    }
}
