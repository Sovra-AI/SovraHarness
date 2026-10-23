//! Check the design contract where failures affect readability or real UI behaviour.

use gpui_kit::component::{ActiveTheme as _, Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, Rgba, TestAppContext, px, size};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn contrast(a: Hsla, b: Hsla) -> f32 {
    let luminance = |color: Hsla| {
        let color = Rgba::from(color);
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[gpui_kit::test]
fn both_modes_keep_native_components_and_product_surfaces_readable(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        for mode in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Dark] {
            Theme::change(mode, None, cx);
            let theme = cx.theme();
            let t = design::tokens(cx);
            assert!(theme.theme_name().starts_with("Sovra "));
            assert_eq!(
                gpui_kit::base::Theme::global(cx).tokens,
                theme.semantic_tokens()
            );
            for background in [
                t.canvas,
                t.chrome,
                t.glass,
                t.subtle,
                t.matte,
                t.elevated,
                t.code_background,
                t.assistant_background,
            ] {
                for foreground in [
                    theme.foreground,
                    theme.muted_foreground,
                    t.text_secondary,
                    theme.link,
                ] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{mode:?}: text contrast {}",
                        contrast(foreground, background)
                    );
                }
                assert!(
                    contrast(theme.ring, background) >= 3.0,
                    "focus ring contrast"
                );
                assert!(
                    contrast(theme.input, background) >= 3.0,
                    "input boundary contrast"
                );
            }
            for (foreground, background) in [
                (theme.primary_foreground, theme.primary),
                (theme.danger_foreground, theme.danger),
                (theme.success_foreground, theme.success),
                (theme.warning_foreground, theme.warning),
                (theme.info_foreground, theme.info),
            ] {
                assert!(
                    contrast(foreground, background) >= 4.5,
                    "filled control contrast"
                );
            }
            for token in [
                theme.tokens.button_primary,
                theme.tokens.button_primary_hover,
            ] {
                assert_ne!(
                    token.background,
                    token.color.into(),
                    "brand gradient was lost"
                );
                let background = serde_json::to_value(token.background).unwrap();
                for stop in background["colors"].as_array().unwrap() {
                    let color: Hsla = serde_json::from_value(stop["color"].clone()).unwrap();
                    assert!(
                        contrast(theme.button_primary_foreground, color) >= 4.5,
                        "gradient label contrast"
                    );
                }
            }
        }
        cx.set_reduce_motion(true);
        let motion = design::tokens(cx).motion;
        assert!(motion.enter.is_zero() && motion.exit.is_zero() && motion.press.is_zero());
        assert_eq!(motion.enter_distance, px(0.));
    });
}

#[gpui_kit::test]
fn appearance_control_works_at_minimum_and_default_window_sizes(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    for dimensions in [size(px(640.), px(420.)), size(px(1100.), px(720.))] {
        let window = cx.open_window(dimensions, |window, cx| {
            let shell = cx.new(|cx| Shell::new(window, cx));
            Root::new(shell, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let button = window.find("toggle-appearance");
            assert!(button.visible());
            assert!(
                button.bounds().left() >= px(0.) && button.bounds().right() <= dimensions.width
            );
            assert!(
                button.bounds().top() >= px(0.) && button.bounds().bottom() <= dimensions.height
            );
            assert_eq!(button.label(), Some("Use light appearance"));
            window.click("toggle-appearance", cx);
            assert!(!cx.theme().is_dark());
            assert_eq!(
                window.find("toggle-appearance").label(),
                Some("Use dark appearance")
            );
            // GPUI deliberately avoids pointer focus; keyboard traversal must work.
            window.press("tab", cx);
            assert_eq!(window.find("toggle-appearance").focused(), Some(true));
            window.press("space", cx);
            assert!(cx.theme().is_dark());
        })
        .unwrap();
    }
}
