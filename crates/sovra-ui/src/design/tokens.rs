//! Typed extensions for product roles not represented by GPUI's component theme.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, BoxShadow, Hsla, Pixels, point, px};

use super::palette::{
    self, BLACK, INDIGO, LILAC, VIOLET, VIOLET_DEEP, VIOLET_LIGHT, WHITE, colour,
};

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

/// Logical pixels, never physical device pixels. Desktop caps from the source CSS.
#[derive(Debug, Clone, Copy)]
pub struct SpaceTokens {
    pub compact: Pixels,
    pub small: Pixels,
    pub medium: Pixels,
    pub large: Pixels,
    pub extra: Pixels,
    pub section: Pixels,
}

#[derive(Debug, Clone, Copy)]
pub struct RadiusTokens {
    pub button: Pixels,
    pub input: Pixels,
    pub card: Pixels,
    pub panel: Pixels,
    pub pill: Pixels,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutTokens {
    pub catalog_row_height: Pixels,
    pub detail_width: Pixels,
    pub detail_breakpoint: Pixels,
    pub filter_width: Pixels,
    pub reading_width: Pixels,
    pub wide_reading_width: Pixels,
    pub page_width: Pixels,
    pub control_height: Pixels,
    pub logo_small: Pixels,
    pub logo_hero: Pixels,
    pub status_height: Pixels,
    pub sidebar_width: Pixels,
    pub inspector_width: Pixels,
    pub window_width: Pixels,
    pub window_height: Pixels,
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

/// Derive from the current GPUI mode, avoiding a second mutable theme global.
pub fn tokens(cx: &App) -> SovraTokens {
    let p = palette::palette(cx.theme().mode);
    let dark = cx.theme().is_dark();
    let canvas = colour(p.canvas);
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
            compact: px(6.),
            small: px(12.),
            medium: px(18.),
            large: px(24.),
            extra: px(36.),
            section: px(48.),
        },
        radius: RadiusTokens {
            button: px(13.),
            input: px(12.),
            card: px(18.),
            panel: px(22.),
            pill: px(999.),
        },
        layout: LayoutTokens {
            catalog_row_height: px(116.),
            detail_width: px(360.),
            detail_breakpoint: px(1120.),
            filter_width: px(158.),
            reading_width: px(768.),
            wide_reading_width: px(880.),
            page_width: px(1440.),
            control_height: px(40.),
            logo_small: px(28.),
            logo_hero: px(96.),
            status_height: px(32.),
            sidebar_width: px(216.),
            inspector_width: px(320.),
            window_width: px(1280.),
            window_height: px(820.),
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
        text_secondary: colour(p.secondary_text),
        canvas,
        // Precompose glass on the canvas: stable contrast, no backdrop sampling
        // or platform-specific vibrancy, including remote/low-power desktops.
        chrome: if dark {
            canvas.blend(colour(0x0c_0a_11).alpha(0.78))
        } else {
            colour(0xfb_fa_ff)
        },
        glass: layer(if dark { 0.065 } else { 0.76 }),
        subtle: layer(if dark { 0.035 } else { 0.52 }),
        matte: colour(p.surface),
        elevated: if dark {
            canvas.blend(colour(p.hover).alpha(0.9))
        } else {
            layer(0.94)
        },
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
        button_shadow: shadow(8., 20., -10., 0.6),
        button_shadow_hover: shadow(10., 24., -10., 0.7),
        assistant_background: layer(if dark { 0.04 } else { 0.8 }),
        user_bubble_background: if dark {
            canvas.blend(colour(VIOLET).alpha(0.22))
        } else {
            colour(0xed_e4_ff)
        },
        hero_banner_background: if dark {
            canvas.blend(colour(INDIGO).alpha(0.18))
        } else {
            colour(0xef_e8_ff)
        },
        nav_active_background: if dark {
            canvas.blend(colour(VIOLET).alpha(0.24))
        } else {
            colour(0xe9_df_ff)
        },
        status_pill_background: if dark {
            canvas.blend(colour(0x22_c5_5e).alpha(0.18))
        } else {
            colour(0xdc_fc_e7)
        },
        code_background: if dark { layer(0.05) } else { colour(p.hover) },
    }
}
