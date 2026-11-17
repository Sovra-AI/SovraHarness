//! The new-chat hint stays readable beside the input at every supported size.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::models::InterfaceScale;
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn new_chat_help_hint_fits_both_themes_and_reduced_motion(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    for dimensions in [size(px(640.), px(420.)), size(px(2560.), px(1440.))] {
        for scale in [100, 200] {
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                for reduced in [false, true] {
                    cx.update(|cx| {
                        design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                        Theme::change(mode, None, cx);
                        cx.set_reduce_motion(reduced);
                    });
                    let window = cx.open_window(dimensions, |window, cx| {
                        let shell = cx.new(|cx| Shell::new(0, window, cx));
                        Root::new(shell, window, cx)
                    });
                    cx.update_window(window.into(), |_, window, cx| {
                        for _ in 0..3 {
                            window.render_frame(cx);
                        }
                        let hint = window.find("chat-welcome-help-hint");
                        assert!(hint.visible());
                        assert!(hint.label().unwrap().contains("/help"));
                        assert!(window.find("welcome-slash-pill-1").visible());
                        let bounds = hint.bounds();
                        assert!(bounds.left() >= px(0.) && bounds.right() <= dimensions.width);
                        assert!(bounds.top() >= px(0.) && bounds.bottom() <= dimensions.height);
                        assert!(
                            bounds.top() >= window.find("chat-composer-input").bounds().bottom()
                        );
                        assert!(window.find("composer-send-btn").visible());
                        window.remove_window();
                    })
                    .unwrap();
                }
            }
        }
    }
}
