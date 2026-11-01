//! Development visibility and its off switch survive small and scaled windows.
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
use sovra_core::{
    Command, Event,
    development::{DevelopmentCommand, DevelopmentEvent, DevelopmentState},
    models::InterfaceScale,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn development_badge_and_toggle_work_at_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [
            (640., 420., 100),
            (2560., 1440., 100),
            (640., 420., 200),
            (2560., 1440., 200),
        ] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                Root::new(cx.new(|cx| Shell::new(window, cx)), window, cx)
            });
            events
                .send_blocking(Event::Development(DevelopmentEvent::State(
                    DevelopmentState {
                        enabled: true,
                        generation: Some("test".into()),
                        error: None,
                        connection_command: "/path/to/sovra --development-mcp".into(),
                    },
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("chat-composer-input").visible());
                let badge = window.find("development-mode-badge");
                assert!(badge.visible());
                assert!(badge.bounds().right() <= px(width));
                assert!(badge.bounds().bottom() <= window.find("title-bar").bounds().bottom());
                window.click("development-mode-badge", cx);
                for _ in 0..100 {
                    window.render_frame(cx);
                    let toggle = window.find("development-mode-toggle");
                    let pane = window.find("settings-sections").bounds();
                    if toggle.visible()
                        && toggle.bounds().top() >= pane.top()
                        && toggle.bounds().bottom() <= pane.bottom()
                    {
                        break;
                    }
                    window.scroll(
                        "settings-sections",
                        ScrollDelta::Pixels(point(px(0.), px(-30.))),
                        cx,
                    );
                }
                let toggle = window.find("development-mode-toggle");
                assert!(toggle.visible());
                assert!(toggle.bounds().right() <= px(width));
                assert_eq!(toggle.checked(), Some(true));
                window.click("development-mode-toggle", cx);
            })
            .unwrap();
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok()).any(|command| command
                    == Command::Development(DevelopmentCommand::SetEnabled(false)))
            );
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}
