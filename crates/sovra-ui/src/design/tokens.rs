//! Typed extensions for product roles not represented by GPUI's component theme.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Background, BoxShadow, Hsla, Pixels, Rems, point, px};

use super::palette::{
    self, BLACK, INDIGO, LILAC, VIOLET, VIOLET_DEEP, VIOLET_LIGHT, WHITE, colour,
};

/// Native conversation roles: quiet user bubbles, readable assistant text,
/// clear composer focus and a responsive reading column.
#[derive(Debug, Clone)]
pub struct ChatTokens {
    /// Prose measure: about 80 characters of 16px reading text (WCAG 1.4.8).
    pub column_width: Rems,
    pub avatar: Rems,
    /// The live ring sits this far outside the avatar.
    pub avatar_ring_offset: Rems,
    pub row_gap: Rems,
    /// Above and below each message; two of them separate consecutive messages.
    pub message_padding: Rems,
    pub user_bubble: Background,
    pub user_bubble_foreground: Hsla,
    pub user_bubble_radius: Pixels,
    pub user_bubble_tail_radius: Pixels,
    pub user_bubble_shadow: BoxShadow,
    pub avatar_ring: Hsla,
    pub thinking_pill: Hsla,
    pub thinking_rule: Hsla,
    pub thinking_max_height: Rems,
    pub composer_background: Hsla,
    pub composer_border: Hsla,
    pub composer_focus_border: Hsla,
    pub composer_focus_glow: BoxShadow,
    pub composer_radius: Pixels,
    pub send_size: Rems,
    pub send_ready_background: Hsla,
    pub send_ready_foreground: Hsla,
    pub send_idle_foreground: Hsla,
    pub stop_background: Hsla,
    pub code_radius: Pixels,
    pub action_idle_opacity: f32,
    pub welcome_width: Rems,
    pub chip_border: Hsla,
    pub chip_height: Rems,
}

/// Source accent ramp for charts/decorations. Text uses contrast-tested theme roles.
#[derive(Debug, Clone, Copy)]
pub struct AccentTokens {
    pub violet: Hsla,
    pub violet_deep: Hsla,
    pub violet_light: Hsla,
    pub lilac: Hsla,
    pub lilac_light: Hsla,
    pub indigo: Hsla,
    pub cyan: Hsla,
    pub pink: Hsla,
    pub green: Hsla,
    pub amber: Hsla,
    pub red: Hsla,
    pub sky: Hsla,
    pub slate: Hsla,
}

#[derive(Debug, Clone, Copy)]
pub struct SpaceTokens {
    pub hair: Rems,
    pub compact: Rems,
    pub snug: Rems,
    pub small: Rems,
    pub medium: Rems,
    pub large: Rems,
    pub extra: Rems,
    pub section: Rems,
}

#[derive(Debug, Clone, Copy)]
/// Corner radii in pixels (GPUI buttons take pixels), scaled with the root size.
pub struct RadiusTokens {
    pub small: Pixels,
    pub button: Pixels,
    pub input: Pixels,
    pub card: Pixels,
    pub panel: Pixels,
    pub pill: Pixels,
}

/// Geometry in rems so it follows the interface scale; see `docs/styling.md`
/// → Responsive layout. Only hairlines and OS window bounds are logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct LayoutTokens {
    /// Borders and dividers stay one logical pixel at every interface scale.
    pub hairline: Pixels,
    pub catalog_row_height: Rems,
    pub detail_width: Rems,
    pub filter_width: Rems,
    /// Forms and settings; prose uses the narrower `chat.column_width`.
    pub reading_width: Rems,
    /// Side panes yield (undock, rail) before the main pane drops below this.
    pub content_min_width: Rems,
    pub card_min_width: Rems,
    pub card_max_width: Rems,
    pub dialog_width: Rems,
    pub side_panel_width: Rems,
    pub list_row_min_height: Rems,
    pub command_menu_height: Rems,
    pub attachment_width: Rems,
    pub attachment_preview: Rems,
    pub page_width: Rems,
    pub control_height: Rems,
    pub compact_control_height: Rems,
    pub navigation_height: Rems,
    pub icon_small: Rems,
    pub icon_medium: Rems,
    pub profile_avatar: Rems,
    pub logo_small: Rems,
    pub logo_medium: Rems,
    pub logo_hero: Rems,
    pub status_height: Rems,
    pub sidebar_width: Rems,
    pub history_compact_width: Rems,
    pub inspector_width: Rems,
    /// Preferred first-launch size; clamped to the display's visible area.
    pub window_width: Pixels,
    pub window_height: Pixels,
    pub window_max_width: Pixels,
    pub window_max_height: Pixels,
    pub window_min_width: Pixels,
    pub window_min_height: Pixels,
}

/// Zero durations and displacement when GPUI's reduced-motion setting is enabled.
#[derive(Debug, Clone, Copy)]
pub struct MotionTokens {
    pub enter: Duration,
    pub exit: Duration,
    pub press: Duration,
    pub control: Duration,
    pub material: Duration,
    pub enter_distance: Pixels,
}

#[derive(Debug, Clone)]
pub struct SovraTokens {
    pub accent: AccentTokens,
    pub space: SpaceTokens,
    pub radius: RadiusTokens,
    pub layout: LayoutTokens,
    pub motion: MotionTokens,
    pub chat: ChatTokens,
    pub text_secondary: Hsla,
    pub canvas: Hsla,
    pub chrome: Hsla,
    pub glass: Hsla,
    pub subtle: Hsla,
    pub matte: Hsla,
    pub elevated: Hsla,
    pub material_border: Hsla,
    pub material_border_strong: Hsla,
    pub material_shadow: BoxShadow,
    pub elevated_shadow: BoxShadow,
    pub button_shadow: BoxShadow,
    pub button_shadow_hover: BoxShadow,
    pub assistant_background: Hsla,
    pub user_bubble_background: Hsla,
    pub hero_banner_background: Hsla,
    pub nav_active_background: Hsla,
    pub status_pill_background: Hsla,
    pub code_background: Hsla,
}

/// Authored in the 16px reference pixels of the source design, stored as rems.
fn rem(reference_px: f32) -> Rems {
    Rems(reference_px / 16.)
}

/// Derive from the current GPUI mode, avoiding a second mutable theme global.
pub fn tokens(cx: &App) -> SovraTokens {
    let p = palette::palette(cx.theme().mode);
    let dark = cx.theme().is_dark();
    let canvas = colour(p.canvas);
    let root = cx.theme().font_size / px(super::responsive::BASE_FONT_SIZE);
    let radius = |reference_px: f32| px(reference_px * root);
    let layer = |opacity| canvas.blend(colour(WHITE).alpha(opacity));
    let shadow = |y, blur, spread, opacity| BoxShadow {
        color: colour(if dark { BLACK } else { 0x4c_1d_95 }).alpha(opacity),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    };
    let duration = |ms| {
        if cx.reduce_motion() {
            Duration::ZERO
        } else {
            Duration::from_millis(ms)
        }
    };
    SovraTokens {
        accent: AccentTokens {
            violet: colour(VIOLET),
            violet_deep: colour(VIOLET_DEEP),
            violet_light: colour(VIOLET_LIGHT),
            lilac: colour(LILAC),
            lilac_light: colour(0xd7_c2_f7),
            indigo: colour(INDIGO),
            cyan: colour(0x22_d3_ee),
            pink: colour(0xf4_72_b6),
            green: colour(0x22_c5_5e),
            amber: colour(0xf5_9e_0b),
            red: colour(0xef_44_44),
            sky: colour(0x0e_a5_e9),
            slate: colour(0x64_74_8b),
        },
        space: SpaceTokens {
            hair: rem(2.),
            compact: rem(4.),
            snug: rem(6.),
            small: rem(8.),
            medium: rem(16.),
            large: rem(24.),
            extra: rem(32.),
            section: rem(48.),
        },
        radius: RadiusTokens {
            small: radius(6.),
            button: radius(10.),
            input: radius(10.),
            card: radius(14.),
            panel: radius(18.),
            pill: radius(999.),
        },
        layout: LayoutTokens {
            hairline: px(1.),
            catalog_row_height: rem(116.),
            detail_width: rem(360.),
            filter_width: rem(160.),
            reading_width: rem(768.),
            content_min_width: rem(400.),
            card_min_width: rem(224.),
            card_max_width: rem(560.),
            dialog_width: rem(432.),
            side_panel_width: rem(260.),
            list_row_min_height: rem(72.),
            command_menu_height: rem(128.),
            attachment_width: rem(240.),
            attachment_preview: rem(40.),
            page_width: rem(1080.),
            control_height: rem(36.),
            compact_control_height: rem(28.),
            // The macOS default (medium) sidebar row is 28pt (HIG → Sidebars); nine
            // destinations must leave the chat history visible at the default window.
            navigation_height: rem(28.),
            icon_small: rem(20.),
            icon_medium: rem(24.),
            profile_avatar: rem(32.),
            logo_small: rem(28.),
            logo_medium: rem(44.),
            logo_hero: rem(56.),
            status_height: rem(26.),
            sidebar_width: rem(256.),
            history_compact_width: rem(56.),
            inspector_width: rem(320.),
            window_width: px(1280.),
            window_height: px(820.),
            window_max_width: px(1600.),
            window_max_height: px(1040.),
            window_min_width: px(640.),
            window_min_height: px(420.),
        },
        motion: MotionTokens {
            enter: duration(240),
            exit: duration(160),
            press: duration(100),
            control: duration(180),
            material: duration(220),
            enter_distance: px(if cx.reduce_motion() { 0. } else { 8. }),
        },
        chat: ChatTokens {
            column_width: rem(736.),
            avatar: rem(28.),
            avatar_ring_offset: rem(3.),
            row_gap: rem(16.),
            message_padding: rem(11.),
            // Authorship is conveyed by alignment and the avatar; a quiet fill
            // keeps long conversations from becoming a wall of brand colour.
            user_bubble: colour(p.hover).into(),
            user_bubble_foreground: colour(p.text),
            user_bubble_radius: radius(14.),
            user_bubble_tail_radius: radius(14.),
            user_bubble_shadow: shadow(0., 0., 0., 0.),
            avatar_ring: colour(VIOLET_LIGHT).alpha(0.55),
            thinking_pill: if dark {
                canvas.blend(colour(VIOLET_LIGHT).alpha(0.12))
            } else {
                colour(0xf1_ea_ff)
            },
            thinking_rule: colour(VIOLET_LIGHT).alpha(0.3),
            thinking_max_height: rem(240.),
            composer_background: colour(p.surface),
            composer_border: colour(if dark { WHITE } else { 0x4c_1d_95 }).alpha(if dark {
                0.1
            } else {
                0.14
            }),
            composer_focus_border: colour(VIOLET).alpha(0.6),
            composer_focus_glow: BoxShadow {
                color: colour(VIOLET).alpha(0.2),
                offset: point(px(0.), px(0.)),
                blur_radius: px(15.),
                spread_radius: px(0.),
                inset: false,
            },
            composer_radius: radius(18.),
            send_size: rem(36.),
            send_ready_background: colour(if dark { WHITE } else { 0x0a_0a_0f }),
            send_ready_foreground: colour(if dark { 0x0a_0a_0f } else { WHITE }),
            send_idle_foreground: colour(p.secondary_text),
            stop_background: colour(0xdc_26_26),
            code_radius: radius(6.),
            action_idle_opacity: 1.0,
            welcome_width: rem(680.),
            chip_border: p.border,
            chip_height: rem(36.),
        },
        text_secondary: colour(p.secondary_text),
        canvas,
        // Precompose glass on the canvas: stable contrast, no backdrop sampling
        // or platform-specific vibrancy, including remote/low-power desktops.
        chrome: if dark {
            colour(0x16_17_1c)
        } else {
            colour(0xf5_f5_f9)
        },
        glass: colour(p.surface),
        subtle: if dark {
            layer(0.025)
        } else {
            colour(0xf5_f5_f9)
        },
        matte: colour(p.surface),
        elevated: if dark { colour(p.hover) } else { layer(0.94) },
        material_border: colour(if dark { WHITE } else { 0x4c_1d_95 }).alpha(if dark {
            0.14
        } else {
            0.11
        }),
        material_border_strong: colour(if dark { WHITE } else { 0x4c_1d_95 }).alpha(if dark {
            0.22
        } else {
            0.2
        }),
        material_shadow: shadow(18., 44., -24., if dark { 0.72 } else { 0.2 }),
        elevated_shadow: shadow(30., 72., -30., if dark { 0.82 } else { 0.28 }),
        button_shadow: shadow(2., 6., -2., if dark { 0.3 } else { 0.12 }),
        button_shadow_hover: shadow(3., 8., -2., if dark { 0.4 } else { 0.16 }),
        assistant_background: layer(if dark { 0.04 } else { 0.8 }),
        user_bubble_background: if dark {
            canvas.blend(colour(VIOLET).alpha(0.22))
        } else {
            colour(p.hover)
        },
        hero_banner_background: if dark {
            canvas.blend(colour(INDIGO).alpha(0.18))
        } else {
            colour(0xef_e8_ff)
        },
        nav_active_background: if dark {
            canvas.blend(colour(VIOLET_LIGHT).alpha(0.16))
        } else {
            colour(0xed_e9_f9)
        },
        status_pill_background: if dark {
            canvas.blend(colour(0x22_c5_5e).alpha(0.18))
        } else {
            colour(0xdc_fc_e7)
        },
        code_background: if dark { layer(0.05) } else { colour(p.hover) },
    }
}
