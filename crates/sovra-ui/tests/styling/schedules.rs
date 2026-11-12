//! Schedule entry and retry stay reachable with large text and narrow panes.
use super::*;

pub(super) fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    for _ in 0..40 {
        window.render_frame(cx);
        let pane = window.find("sovra-schedules-view").bounds();
        let item = window.find(id).bounds();
        assert!(
            item.left() >= pane.left() && item.right() <= pane.right(),
            "{id} overflows horizontally"
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
            "sovra-schedules-view",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id} must be reachable");
}

#[gpui_kit::test]
fn schedule_labels_examples_and_retry_fit_each_size(cx: &mut TestAppContext) {
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
                shell.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Schedules, cx);
                });
                Root::new(shell, window, cx)
            });
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "schedule-empty-create-btn", cx);
                window.click("schedule-empty-create-btn", cx);
                for id in [
                    "schedule-name-label",
                    "schedule-cadence-label",
                    "schedule-cadence-help",
                    "schedule-prompt-label",
                ] {
                    reveal(window, id, cx);
                }
                for (id, value) in [
                    ("schedule-name-input", "Garden"),
                    ("schedule-prompt-input", "Check the garden"),
                ] {
                    reveal(window, id, cx);
                    window.click(id, cx);
                    window.input(value, cx);
                }
                reveal(window, "schedule-create-toggle-btn", cx);
                window.click("schedule-create-toggle-btn", cx);
                window.click("schedule-create-toggle-btn", cx);
                reveal(window, "schedule-save-btn", cx);
                window.click("schedule-save-btn", cx);
                assert!(window.try_find("schedule-create-form").is_some());
            })
            .unwrap();
            events
                .send_blocking(sovra_core::Event::Schedule(
                    sovra_core::ScheduleEvent::Error(
                        "Could not save the task. Your entries are still here; try again.".into(),
                    ),
                ))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "schedule-error", cx);
                reveal(window, "schedule-save-btn", cx);
                window.click("schedule-save-btn", cx);
            })
            .unwrap();
            let creates: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    sovra_core::Command::Schedule(sovra_core::ScheduleCommand::Create {
                        name,
                        prompt,
                        ..
                    }) => Some((name, prompt)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                creates,
                vec![("Garden".into(), "Check the garden".into()); 2]
            );
        }
    }
}
