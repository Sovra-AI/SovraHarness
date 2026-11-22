//! The tip line under the message box stays inside its column, and a window short
//! for its text size gives that line's room to the conversation.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::models::InterfaceScale;
use sovra_core::{ChatMessage, ChatThread, Event, Timestamp};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn tips_stay_in_the_column_and_make_way_in_a_short_window(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale, tips) in [
            (640., 420., 100, true),
            (2560., 1440., 200, true),
            // Narrow but tall: the tips show and must wrap inside the column.
            (640., 1440., 200, true),
            (640., 420., 200, false),
        ] {
            let (client, _commands, events) = Client::loopback();
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
            let shell = shell.expect("shell was created");
            let mut thread = ChatThread::new("talk", Timestamp(1));
            thread
                .messages
                .push(ChatMessage::assistant("reply", "Tea.", "Earlier", vec![]));
            events
                .send_blocking(Event::ChatHistoryLoaded {
                    threads: vec![thread],
                })
                .expect("loopback accepts events");
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update(|cx| shell.update(cx, |shell, cx| shell.select_thread("talk", cx)));
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let label = format!("{width}x{height} at {scale}%");
                let column = window.find("sovra-composer-container").bounds();
                match window.try_find("composer-hint") {
                    Some(hint) => {
                        assert!(tips, "{label}: tips make way in a short window");
                        let hint = hint.bounds();
                        assert!(
                            hint.left() >= column.left() && hint.right() <= column.right(),
                            "{label}: the tip line wraps inside the column"
                        );
                    }
                    None => assert!(!tips, "{label}: tips show when there is room"),
                }
                assert!(window.find("composer-send-btn").visible(), "{label}");
                let conversation = window.find("chat-scroll-area").bounds().size.height;
                assert!(
                    conversation >= window.find("composer-send-btn").bounds().size.height,
                    "{label}: the conversation keeps room to read ({conversation:?})"
                );
                window.remove_window();
            })
            .expect("window is open");
        }
    }
}
