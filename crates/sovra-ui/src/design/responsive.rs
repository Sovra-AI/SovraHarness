//! Adaptive layout primitives: width classes, container measurement, interface
//! scale and display-aware window bounds. Rules and sources: `docs/styling.md`.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeConfig};
use gpui_kit::{
    App, Bounds, Context, IntoElement, Pixels, Size, Styled as _, WeakEntity, Window, canvas, size,
};
use sovra_core::models::InterfaceScale;

use super::tokens;

/// Root sizes the Sovra themes are authored at; the interface scale multiplies both.
pub(super) const BASE_FONT_SIZE: f32 = 16.;
pub(super) const BASE_MONO_FONT_SIZE: f32 = 13.;

/// Material 3 window size classes, measured in rems on the space a view actually
/// receives, so they also move when the interface scale changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WidthClass {
    /// Below 600 reference px: one pane, side panels as overlays.
    Compact,
    /// 600–839: one primary pane, sidebar as an icon rail.
    Medium,
    /// 840–1199: sidebar plus one content pane.
    Expanded,
    /// 1200–1599: room for a docked detail or inspector pane.
    Large,
    /// 1600 and up: same panes; content stays capped and centred.
    ExtraLarge,
}

impl WidthClass {
    /// Upper bounds in rems (16px reference) for every class but the last.
    const BOUNDS: [(f32, Self); 4] = [
        (37.5, Self::Compact),
        (52.5, Self::Medium),
        (75., Self::Expanded),
        (100., Self::Large),
    ];

    #[must_use]
    pub fn from_rems(width: f32) -> Self {
        Self::BOUNDS
            .iter()
            .find(|(bound, _)| width < *bound)
            .map_or(Self::ExtraLarge, |(_, class)| *class)
    }

    #[must_use]
    pub fn of(width: Pixels, window: &Window) -> Self {
        Self::from_rems(width / window.rem_size())
    }

    /// The whole window: only the shell's top-level panes may decide from this.
    #[must_use]
    pub fn of_window(window: &Window) -> Self {
        Self::of(window.viewport_size().width, window)
    }

    /// For the whole window: whether a side pane fits beside the sidebar and a
    /// main pane that keeps its minimum width.
    #[must_use]
    pub fn fits_side_pane(self) -> bool {
        self >= Self::Large
    }

    /// For a measured pane (sidebar already excluded): room for a detail pane
    /// (`layout.detail_width`) beside a main pane of `layout.content_min_width`.
    #[must_use]
    pub fn fits_detail_pane(self) -> bool {
        self >= Self::Expanded
    }
}

/// Measures the element it is placed in (container query) and stores the class
/// in the owning view, notifying only when the class changes. Place it as an
/// absolutely positioned child of the container: `.child(width_probe(..))`.
pub fn width_probe<V: 'static>(
    view: &Context<V>,
    class: fn(&mut V) -> &mut WidthClass,
) -> impl IntoElement + use<V> {
    let view: WeakEntity<V> = view.weak_entity();
    canvas(
        move |bounds: Bounds<Pixels>, window, cx| {
            let measured = WidthClass::of(bounds.size.width, window);
            view.update(cx, |view, cx| {
                let current = class(view);
                if *current != measured {
                    *current = measured;
                    cx.notify();
                }
            })
            .ok();
        },
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Applies a zoom level by changing the root font size of both registered themes.
/// GPUI derives `rem` from it, so every token, text role and component scales.
pub fn set_interface_scale(scale: InterfaceScale, cx: &mut App) {
    let font_size = BASE_FONT_SIZE * scale.factor();
    let mono_font_size = BASE_MONO_FONT_SIZE * scale.factor();
    if (cx.theme().font_size.as_f32() - font_size).abs() < f32::EPSILON {
        return;
    }
    let scaled = |config: &Rc<ThemeConfig>| {
        Rc::new(ThemeConfig {
            font_size: Some(font_size),
            mono_font_size: Some(mono_font_size),
            ..(**config).clone()
        })
    };
    let mode = cx.theme().mode;
    Theme::update(cx, |theme| {
        theme.light_theme = scaled(&theme.light_theme);
        theme.dark_theme = scaled(&theme.dark_theme);
    });
    // Re-applying the mode copies the scaled config into the live theme.
    Theme::change(mode, None, cx);
}

/// Current zoom level, read back from the live theme.
#[must_use]
pub fn interface_scale(cx: &App) -> InterfaceScale {
    let percent = (cx.theme().font_size.as_f32() / BASE_FONT_SIZE * 100.).round();
    // Rounded and clamped by `from_percent`; the cast cannot lose a valid level.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded, non-negative percentage well inside u16"
    )]
    InterfaceScale::from_percent(percent.clamp(0., 1000.) as u16)
}

/// First-launch window: the preferred size, never larger than 90% of the
/// display's visible area (dock/taskbar excluded), centred on it.
#[must_use]
pub fn initial_window_bounds(cx: &App) -> Bounds<Pixels> {
    let layout = tokens(cx).layout;
    let preferred = size(layout.window_width, layout.window_height);
    let fitted = cx.primary_display().map_or(preferred, |display| {
        let visible = display.visible_bounds().size;
        fit_window(
            preferred,
            size(visible.width * 0.9, visible.height * 0.9),
            size(layout.window_min_width, layout.window_min_height),
        )
    });
    Bounds::centered(None, fitted, cx)
}

/// Large displays get proportionally more room (up to the preferred size),
/// small ones shrink toward the minimum.
fn fit_window(preferred: Size<Pixels>, room: Size<Pixels>, min: Size<Pixels>) -> Size<Pixels> {
    size(
        preferred.width.min(room.width).max(min.width),
        preferred.height.min(room.height).max(min.height),
    )
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::*;

    #[test]
    fn width_classes_follow_material_breakpoints_in_rems() {
        assert_eq!(WidthClass::from_rems(30.), WidthClass::Compact);
        assert_eq!(WidthClass::from_rems(37.5), WidthClass::Medium);
        assert_eq!(WidthClass::from_rems(52.5), WidthClass::Expanded);
        assert_eq!(WidthClass::from_rems(75.), WidthClass::Large);
        assert_eq!(WidthClass::from_rems(100.), WidthClass::ExtraLarge);
        assert!(!WidthClass::Expanded.fits_side_pane());
        assert!(WidthClass::Large.fits_side_pane());
        assert!(!WidthClass::Medium.fits_detail_pane());
        assert!(WidthClass::Expanded.fits_detail_pane());
    }

    #[test]
    fn first_window_fits_small_displays_and_keeps_the_minimum() {
        let preferred = size(px(1280.), px(820.));
        let min = size(px(640.), px(420.));
        assert_eq!(
            fit_window(preferred, size(px(2000.), px(1200.)), min),
            preferred
        );
        assert_eq!(
            fit_window(preferred, size(px(1100.), px(700.)), min),
            size(px(1100.), px(700.))
        );
        assert_eq!(fit_window(preferred, size(px(500.), px(300.)), min), min);
    }
}
