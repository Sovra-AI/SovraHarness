//! Product palette from `SovraSRXProduction`. Raw colours stay in this module.

use gpui_kit::component::{Colorize as _, ThemeConfigColors, ThemeMode};
use gpui_kit::{Hsla, SharedString, rgb};

pub(super) const VIOLET: u32 = 0x7c_3a_ed;
pub(super) const VIOLET_DEEP: u32 = 0x6d_28_d9;
pub(super) const VIOLET_LIGHT: u32 = 0x9f_67_ff;
pub(super) const LILAC: u32 = 0xb9_8c_f0;
pub(super) const INDIGO: u32 = 0x5b_6c_f0;
// Slightly deeper than brand indigo so white control labels pass 4.5:1.
pub(super) const INDIGO_FILL: u32 = 0x55_66_e6;
pub(super) const WHITE: u32 = 0xff_ff_ff;
pub(super) const BLACK: u32 = 0x00_00_00;

#[derive(Debug, Clone, Copy)]
pub(super) struct Palette {
    pub canvas: u32,
    pub surface: u32,
    pub hover: u32,
    pub text: u32,
    pub secondary_text: u32,
    pub muted_text: u32,
    pub link: u32,
    pub border: Hsla,
    pub control_border: u32,
    pub danger: u32,
    pub success: u32,
    pub warning: u32,
    pub status_ink: u32,
}

pub(super) fn palette(mode: ThemeMode) -> Palette {
    match mode {
        ThemeMode::Dark => Palette {
            canvas: BLACK,
            surface: 0x14_12_1c,
            hover: 0x20_1d_29,
            text: 0xf5_f2_fb,
            secondary_text: 0xc8_bf_e0,
            muted_text: 0x97_8c_b2,
            link: LILAC,
            border: colour(WHITE).alpha(0.12),
            // Decorative hairlines are not sufficient to identify an input.
            control_border: 0x81_75_8f,
            danger: 0xef_44_44,
            success: 0x22_c5_5e,
            warning: 0xf5_9e_0b,
            status_ink: BLACK,
        },
        ThemeMode::Light => Palette {
            canvas: 0xfa_f5_ff,
            surface: WHITE,
            hover: 0xf3_e8_ff,
            text: 0x58_1c_87,
            secondary_text: 0x78_38_b0,
            // Source 50%-alpha purple fails body-text contrast. Use opaque ink.
            muted_text: 0x80_55_9e,
            link: VIOLET_DEEP,
            border: colour(VIOLET).alpha(0.1),
            control_border: 0x91_67_b0,
            danger: 0xdc_26_26,
            success: 0x15_80_3d,
            warning: 0xb4_53_09,
            status_ink: WHITE,
        },
    }
}

pub(super) fn colour(value: u32) -> Hsla {
    rgb(value).into()
}

fn hex(value: u32) -> SharedString {
    format!("#{value:06x}").into()
}

fn alpha(value: Hsla) -> SharedString {
    value.to_hex().into()
}

pub(super) fn config(mode: ThemeMode) -> ThemeConfigColors {
    let p = palette(mode);
    let active = colour(VIOLET).alpha(if mode.is_dark() { 0.24 } else { 0.10 });
    let mut config = ThemeConfigColors::default();
    config.background = Some(hex(p.canvas));
    config.foreground = Some(hex(p.text));
    config.border = Some(alpha(p.border));
    config.primary = Some(hex(VIOLET));
    config.primary_hover = Some(hex(VIOLET_DEEP));
    config.primary_active = Some(hex(VIOLET_DEEP));
    config.primary_foreground = Some(hex(WHITE));
    config.button_primary =
        Some(format!("linear-gradient(102deg, #{VIOLET:06x}, #{INDIGO_FILL:06x})").into());
    config.button_primary_hover =
        Some(format!("linear-gradient(102deg, #{VIOLET_DEEP:06x}, #{INDIGO_FILL:06x})").into());
    config.button_primary_active = Some(hex(VIOLET_DEEP));
    config.button = Some(hex(p.surface));
    config.button_hover = Some(hex(p.hover));
    config.button_active = Some(alpha(active));
    config.button_foreground = Some(hex(p.text));
    config.secondary = Some(hex(p.hover));
    config.secondary_hover = Some(hex(p.surface));
    config.secondary_active = Some(alpha(active));
    config.secondary_foreground = Some(hex(p.text));
    config.accent = Some(hex(p.hover));
    config.accent_foreground = Some(hex(p.text));
    config.muted = Some(hex(p.hover));
    config.muted_foreground = Some(hex(p.muted_text));
    config.popover = Some(hex(p.surface));
    config.popover_foreground = Some(hex(p.text));
    config.accordion = Some(hex(p.surface));
    config.group_box = Some(hex(p.surface));
    config.group_box_foreground = Some(hex(p.text));
    config.group_box_title_foreground = Some(hex(p.secondary_text));
    config.input = Some(hex(p.control_border));
    config.ring = Some(hex(if mode.is_dark() {
        VIOLET_LIGHT
    } else {
        VIOLET_DEEP
    }));
    config.caret = Some(hex(p.link));
    config.link = Some(hex(p.link));
    config.link_hover = Some(hex(p.link));
    config.link_active = Some(hex(p.link));
    config.drag_border = Some(hex(p.link));
    config.drop_target = Some(alpha(active));
    config.selection = Some(alpha(active));
    config.progress_bar = Some(hex(VIOLET));
    config.slider_bar = Some(hex(VIOLET));
    config.slider_thumb = Some(hex(WHITE));
    config.skeleton = Some(hex(p.hover));
    config.switch = Some(hex(p.control_border));
    config.switch_thumb = Some(hex(WHITE));
    config.description_list_label = Some(hex(p.hover));
    config.description_list_label_foreground = Some(hex(p.secondary_text));
    config.sidebar = Some(hex(p.surface));
    config.sidebar_foreground = Some(hex(p.text));
    config.sidebar_border = Some(alpha(p.border));
    config.sidebar_accent = Some(hex(p.hover));
    config.sidebar_accent_foreground = Some(hex(p.text));
    config.sidebar_primary = Some(hex(VIOLET));
    config.sidebar_primary_foreground = Some(hex(WHITE));
    config.tab = Some(hex(p.canvas));
    config.tab_foreground = Some(hex(p.secondary_text));
    config.tab_active = Some(hex(p.surface));
    config.tab_active_foreground = Some(hex(p.text));
    config.tab_bar = Some(hex(p.canvas));
    config.tab_bar_segmented = Some(hex(p.hover));
    config.scrollbar = Some(hex(p.surface));
    config.scrollbar_thumb = Some(hex(p.control_border));
    config.scrollbar_thumb_hover = Some(hex(p.link));
    config.chart_1 = Some(hex(VIOLET));
    config.chart_2 = Some(hex(INDIGO));
    config.chart_3 = Some(hex(LILAC));
    config.chart_4 = Some(hex(p.warning));
    config.chart_5 = Some(hex(p.success));
    config.chart_bullish = Some(hex(p.success));
    config.chart_bearish = Some(hex(p.danger));
    config.chart_grid = Some(alpha(p.border));
    config.overlay = Some(alpha(colour(BLACK).alpha(0.6)));
    config.window_border = Some(alpha(p.border));
    config.list = Some(hex(p.canvas));
    config.list_active = Some(alpha(active));
    config.list_active_border = Some(hex(p.link));
    config.list_even = Some(hex(p.surface));
    config.list_head = Some(hex(p.surface));
    config.list_hover = Some(hex(p.hover));
    config.table = Some(hex(p.canvas));
    config.table_active = Some(alpha(active));
    config.table_active_border = Some(hex(p.link));
    config.table_even = Some(hex(p.surface));
    config.table_head = Some(hex(p.surface));
    config.table_hover = Some(hex(p.hover));
    config.table_head_foreground = Some(hex(p.secondary_text));
    config.table_foot = Some(hex(p.surface));
    config.table_foot_foreground = Some(hex(p.secondary_text));
    config.table_row_border = Some(alpha(p.border));
    config.title_bar = Some(hex(p.surface));
    config.title_bar_border = Some(alpha(p.border));
    config.status_bar = Some(hex(p.surface));
    config.status_bar_border = Some(alpha(p.border));
    config.danger = Some(hex(p.danger));
    config.danger_hover = Some(hex(p.danger));
    config.danger_active = Some(hex(p.danger));
    config.danger_foreground = Some(hex(p.status_ink));
    config.success = Some(hex(p.success));
    config.success_hover = Some(hex(p.success));
    config.success_active = Some(hex(p.success));
    config.success_foreground = Some(hex(p.status_ink));
    config.warning = Some(hex(p.warning));
    config.warning_hover = Some(hex(p.warning));
    config.warning_active = Some(hex(p.warning));
    config.warning_foreground = Some(hex(p.status_ink));
    config.info = Some(hex(INDIGO_FILL));
    config.info_hover = Some(hex(INDIGO_FILL));
    config.info_active = Some(hex(INDIGO_FILL));
    config.info_foreground = Some(hex(WHITE));
    config
}
