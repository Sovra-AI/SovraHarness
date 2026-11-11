//! Check the design contract where failures affect readability or real UI behaviour.

#[path = "styling/development.rs"]
mod development;
#[path = "styling/file_knowledge.rs"]
mod file_knowledge;
#[path = "styling/knowledge_profile.rs"]
mod knowledge_profile;
#[path = "styling/memory_citations.rs"]
mod memory_citations;
#[path = "styling/memory_directory.rs"]
mod memory_directory;
#[path = "styling/memory_exploration.rs"]
mod memory_exploration;
#[path = "styling/memory_graph.rs"]
mod memory_graph;
#[path = "styling/memory_sources.rs"]
mod memory_sources;
#[path = "styling/messaging.rs"]
mod messaging;
#[path = "styling/research_board.rs"]
mod research_board;
#[path = "styling/terminal.rs"]
mod terminal;
#[path = "styling/workspace_fit.rs"]
mod workspace_fit;

use gpui_kit::component::{ActiveTheme as _, Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, Rgba, TestAppContext, px, size};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

/// Clicks a sidebar tab. In small windows the icon rail scrolls, so first bring the
/// icon fully into view, as a person would.
fn click_nav(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &'static str) {
    if let Some(rail) = window.try_find("chat-history-rail-collapsed") {
        for _ in 0..40 {
            let (rail, icon) = (rail.bounds(), window.find(id).bounds());
            if icon.top() >= rail.top() && icon.bottom() <= rail.bottom() {
                break;
            }
            let dy = if icon.top() < rail.top() { 40. } else { -40. };
            window.scroll(
                "chat-history-rail-collapsed",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(dy))),
                cx,
            );
            window.render_frame(cx);
        }
    }
    window.click(id, cx);
}

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
fn appearance_lives_in_settings_at_minimum_and_default_window_sizes(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    for dimensions in [size(px(640.), px(420.)), size(px(1100.), px(720.))] {
        let window = cx.open_window(dimensions, |window, cx| {
            let shell = cx.new(|cx| Shell::new(0, window, cx));
            Root::new(shell, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            // The title bar stays empty: no product name, no appearance switch.
            assert!(window.try_find("toggle-appearance").is_none());
            assert!(window.try_find("title-logo").is_none());
            click_nav(window, cx, "nav-settings");
            window.render_frame(cx);
            for id in ["settings-theme-light", "settings-theme-dark"] {
                let button = window.find(id);
                assert!(button.visible());
                assert!(
                    button.bounds().left() >= px(0.) && button.bounds().right() <= dimensions.width
                );
                assert!(
                    button.bounds().top() >= px(0.)
                        && button.bounds().bottom() <= dimensions.height
                );
            }
            assert_eq!(
                window.find("settings-theme-light").label(),
                Some(sovra_core::models::chrome(
                    &sovra_core::models::UiLanguage::default(),
                    sovra_core::models::ChromeKey::SettingsLightA11y,
                ))
            );
            window.click("settings-theme-light", cx);
            assert!(!cx.theme().is_dark());
            window.render_frame(cx);
            // Keyboard users reach the switch through native traversal too.
            for _ in 0..30 {
                window.press("tab", cx);
                if window.find("settings-theme-dark").focused() == Some(true) {
                    break;
                }
            }
            assert_eq!(window.find("settings-theme-dark").focused(), Some(true));
            window.press("space", cx);
            assert!(cx.theme().is_dark());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn buttons_and_clickable_controls_use_pointer_cursor(cx: &mut TestAppContext) {
    use gpui_kit::{CursorStyle, Styled as _};

    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        let mut standard = design::button("test-btn", "Action", cx);
        assert_eq!(
            standard.style().mouse_cursor,
            Some(CursorStyle::PointingHand),
            "design::button must set a pointer cursor"
        );
        let mut primary = design::primary_button("test-primary-btn", "Primary", cx);
        assert_eq!(
            primary.style().mouse_cursor,
            Some(CursorStyle::PointingHand),
            "design::primary_button must set a pointer cursor"
        );
    });
}

#[gpui_kit::test]
fn routing_choices_are_keyboard_operable(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell_entity = None;
    let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        shell_entity = Some(shell.clone());
        Root::new(shell, window, cx)
    });
    let shell = shell_entity.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-my-ais", cx);
        window.render_frame(cx);
        for _ in 0..50 {
            window.press("tab", cx);
            if window.find(("routing-mode-btn", 1usize)).focused() == Some(true) {
                break;
            }
        }
        assert_eq!(
            window.find(("routing-mode-btn", 1usize)).focused(),
            Some(true)
        );
        window.press("space", cx);
        assert_eq!(
            shell.read(cx).workspace().routing.mode,
            sovra_core::SelectionMode::PerAgentModel
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn workspace_pages_and_actions_fit_compact_windows(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let dimensions = size(px(640.), px(420.));
    let window = cx.open_window(dimensions, |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-schedules", cx);
        window.render_frame(cx);
        window.click("schedule-create-toggle-btn", cx);
        window.render_frame(cx);
        window.click("schedule-name-input", cx);
        window.input("Weekday briefing", cx);
        window.click("schedule-prompt-input", cx);
        window.input("Give me a short briefing of what's on today", cx);
        window.click("schedule-save-btn", cx);
        window.render_frame(cx);
        assert!(window.find("schedules-timeline").visible());
        assert!(window.find(("schedule-lane", 0usize)).visible());
        assert!(window.find("schedules-now-marker").visible());
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            for (tab, page) in [
                ("nav-my-ais", "sovra-my-ais-view"),
                ("nav-knowledge", "sovra-knowledge-view"),
                ("nav-schedules", "sovra-schedules-view"),
                ("nav-skills", "sovra-skills-view"),
                ("nav-settings", "sovra-settings-view"),
                ("nav-connections", "connections-page"),
                ("nav-groups", "sovra-groups-view"),
                ("nav-models", "models-setup-home"),
            ] {
                click_nav(window, cx, tab);
                window.render_frame(cx);
                let bounds = window.find(page).bounds();
                assert!(
                    bounds.left() >= px(0.) && bounds.right() <= dimensions.width,
                    "{page}"
                );
                assert!(
                    bounds.top() >= px(0.) && bounds.bottom() <= dimensions.height,
                    "{page}"
                );
                if tab == "nav-knowledge" {
                    assert!(
                        bounds.size.width >= px(500.),
                        "reading space must survive at minimum width"
                    );
                }
                if tab == "nav-schedules" {
                    let action = window.find(("schedule-run", 0usize));
                    assert!(
                        action.bounds().right() <= dimensions.width,
                        "schedule action overflow"
                    );
                    assert!(
                        action.focused().is_some(),
                        "schedule action needs native keyboard focus"
                    );
                }
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn login_controls_stay_inside_the_minimum_window(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let dimensions = size(px(640.), px(420.));
    let window = cx.open_window(dimensions, |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("sidebar-user-profile", cx);
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            let page = window.find("sovra-login-screen").bounds();
            for id in ["login-glass-card", "login-submit-btn", "login-google-btn"] {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.top() >= page.top() && bounds.bottom() <= page.bottom(),
                    "{id} must fit the login viewport"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn interface_scale_resizes_the_layout_and_survives_theme_changes(cx: &mut TestAppContext) {
    use sovra_core::models::InterfaceScale;
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let full = window.find("sovra-sidebar").bounds().size.width;
        assert!(full >= px(240.), "full sidebar at 100%: {full:?}");

        design::set_interface_scale(InterfaceScale::from_percent(150), cx);
        window.render_frame(cx);
        assert!((window.rem_size() - px(24.)).abs() < px(0.01));
        // 1100px is under 52.5rem at 24px/rem, so the sidebar yields to a rail.
        let rail = window.find("sovra-sidebar").bounds().size.width;
        assert!(rail < full, "zoomed sidebar collapses to a rail: {rail:?}");
        assert!(window.try_find("chat-history-rail-collapsed").is_some());
        let main = window.find("sovra-chat-main").bounds().size.width;
        let floor = design::tokens(cx)
            .layout
            .content_min_width
            .to_pixels(window.rem_size());
        assert!(
            main >= floor,
            "side panes yield before the main pane: {main:?}"
        );

        Theme::change(ThemeMode::Light, Some(window), cx);
        window.render_frame(cx);
        assert_eq!(design::interface_scale(cx).percent(), 150);

        design::set_interface_scale(InterfaceScale::default(), cx);
        window.render_frame(cx);
        assert_eq!(window.find("sovra-sidebar").bounds().size.width, full);
    })
    .unwrap();
}

#[gpui_kit::test]
fn wide_windows_keep_a_centred_readable_column(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let dimensions = size(px(2560.), px(1440.));
    let window = cx.open_window(dimensions, |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let main = window.find("sovra-chat-main").bounds();
        let composer = window.find("chat-composer-input").bounds();
        let cap = design::tokens(cx)
            .chat
            .column_width
            .to_pixels(window.rem_size());
        assert!(composer.size.width <= cap, "{composer:?} exceeds {cap:?}");
        let left = composer.left() - main.left();
        let right = main.right() - composer.right();
        assert!(
            (left - right).abs() < px(80.),
            "column is centred: {left:?} vs {right:?}"
        );
        window.click("nav-connections", cx);
        window.render_frame(cx);
        let connections = window.find("connections-page").bounds();
        assert!(
            connections.left() >= px(0.) && connections.right() <= dimensions.width,
            "connections page fits a wide window: {connections:?}"
        );
        assert!(window.find("connection-card-online-ai").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn workspace_pages_fit_at_double_interface_size(cx: &mut TestAppContext) {
    use sovra_core::models::InterfaceScale;
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let dimensions = size(px(1280.), px(820.));
    let window = cx.open_window(dimensions, |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        design::set_interface_scale(InterfaceScale::LARGEST, cx);
        window.render_frame(cx);
        for (tab, page) in [
            ("nav-chat", "sovra-chat-main"),
            ("nav-my-ais", "sovra-my-ais-view"),
            ("nav-knowledge", "sovra-knowledge-view"),
            ("nav-schedules", "sovra-schedules-view"),
            ("nav-skills", "sovra-skills-view"),
            ("nav-settings", "sovra-settings-view"),
            ("nav-connections", "connections-page"),
            ("nav-groups", "sovra-groups-view"),
            ("nav-models", "models-setup-home"),
        ] {
            click_nav(window, cx, tab);
            window.render_frame(cx);
            let bounds = window.find(page).bounds();
            assert!(
                bounds.left() >= px(0.) && bounds.right() <= dimensions.width,
                "{page} overflows horizontally at 200%: {bounds:?}"
            );
        }
        click_nav(window, cx, "nav-settings");
        window.render_frame(cx);
        let larger = window.find("settings-text-larger");
        assert!(larger.bounds().right() <= dimensions.width);
        window.click("settings-text-reset", cx);
        assert_eq!(design::interface_scale(cx).percent(), 100);
    })
    .unwrap();
}
