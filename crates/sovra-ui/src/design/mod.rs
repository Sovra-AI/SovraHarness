//! Sovra's native design system. See `docs/styling.md` for provenance and usage.
//!
//! GPUI components read `cx.theme()`; app-owned surfaces use [`tokens`] and
//! [`SovraStyled`]. All raw visual values belong in this module tree.

mod palette;
mod responsive;
mod styles;
mod tokens;

use std::rc::Rc;

use gpui_kit::base::{Easing, Spring};
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, highlighter::HighlightTheme};
use gpui_kit::{App, Global};
use sovra_core::models::{ChromeKey, UiLanguage};

pub use responsive::{
    WidthClass, initial_window_bounds, interface_scale, set_interface_scale, width_probe,
};
pub use styles::{
    FitsPane, SovraStyled, Surface, TextRole, button, markdown_style, page_header, primary_button,
    round_icon_button, title_button,
};
pub use tokens::{
    AccentTokens, ChatTokens, GraphTokens, LayoutTokens, MotionTokens, RadiusTokens, SovraTokens,
    SpaceTokens, tokens,
};

struct UiLanguageState(UiLanguage);

impl Global for UiLanguageState {}

/// Current chrome/agent language. Defaults to Arabic before preferences load.
#[must_use]
pub fn ui_language(cx: &App) -> UiLanguage {
    cx.try_global::<UiLanguageState>()
        .map_or_else(UiLanguage::default, |state| state.0.clone())
}

pub fn set_ui_language(language: UiLanguage, cx: &mut App) {
    cx.set_global(UiLanguageState(language));
}

#[must_use]
pub fn chrome(cx: &App, key: ChromeKey) -> &'static str {
    sovra_core::models::chrome(&ui_language(cx), key)
}

#[must_use]
pub fn is_rtl(cx: &App) -> bool {
    ui_language(cx).is_rtl()
}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(UiLanguageState(UiLanguage::default()));
    let config = |mode| {
        Rc::new(ThemeConfig {
            name: match mode {
                ThemeMode::Dark => "Sovra Dark".into(),
                ThemeMode::Light => "Sovra Light".into(),
            },
            mode,
            font_size: Some(responsive::BASE_FONT_SIZE),
            mono_font_size: Some(responsive::BASE_MONO_FONT_SIZE),
            // Keep GPUI's platform font resolution: SF on macOS and installed
            // native fallbacks elsewhere, without network font downloads.
            radius: Some(10),
            radius_lg: Some(18),
            shadow: Some(true),
            colors: palette::config(mode),
            // Without a mode-specific style the component keeps its light code theme,
            // which is unreadable on the dark canvas.
            highlight: Some(
                match mode {
                    ThemeMode::Dark => HighlightTheme::default_dark(),
                    ThemeMode::Light => HighlightTheme::default_light(),
                }
                .style
                .clone(),
            ),
            ..ThemeConfig::default()
        })
    };
    Theme::update(cx, |theme| {
        theme.light_theme = config(ThemeMode::Light);
        theme.dark_theme = config(ThemeMode::Dark);
    });
    Theme::change(ThemeMode::Dark, None, cx);
    let motion = tokens(cx).motion;
    Theme::update(cx, |theme| {
        theme.motion.duration_fast = motion.press;
        theme.motion.duration_normal = motion.control;
        theme.motion.duration_slow = motion.enter;
        theme.motion.easing_enter = Easing::cubic_bezier(0.23, 1., 0.32, 1.)
            .expect("the Sovra enter curve has valid control points");
        theme.motion.spring_control = Spring::new(theme.motion.duration_normal);
        theme.motion.spring_move = Spring::new(theme.motion.duration_slow);
    });
}
