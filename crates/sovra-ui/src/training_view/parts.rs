//! Small building blocks every card on the Training page shares.

use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};

/// One card of the page. Compact panes get tighter padding so the words keep
/// most of the width. Callers give it an id once its children are in place.
pub(super) fn card(compact: bool, cx: &App) -> Div {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .p(if compact {
            t.space.medium
        } else {
            t.space.large
        })
        .sovra_surface(Surface::Matte, cx)
}

pub(super) fn title(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .min_w_0()
        .whitespace_normal()
        .sovra_text(TextRole::SectionTitle, cx)
        .child(text.into())
}

pub(super) fn body(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .min_w_0()
        .whitespace_normal()
        .sovra_text(TextRole::Body, cx)
        .child(text.into())
}

pub(super) fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .min_w_0()
        .whitespace_normal()
        .sovra_text(TextRole::Caption, cx)
        .child(text.into())
}

/// A line that changes while the page is open, announced politely.
pub(super) fn status<T: Into<SharedString>>(
    id: &'static str,
    text: T,
    cx: &App,
) -> impl IntoElement + use<T> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(text.clone())
        .min_w_0()
        .whitespace_normal()
        .sovra_text(TextRole::Body, cx)
        .child(text)
}

/// Something that stops the person: said at once, in the danger colour and in words.
pub(super) fn alert<T: Into<SharedString>>(
    id: &'static str,
    text: T,
    cx: &App,
) -> impl IntoElement + use<T> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .role(Role::Alert)
        .aria_label(text.clone())
        .min_w_0()
        .whitespace_normal()
        .sovra_text(TextRole::Body, cx)
        .text_color(cx.theme().danger)
        .child(text)
}

/// A named fact or live number. A row of them wraps when the pane narrows.
pub(super) fn tile<T: Into<SharedString>>(
    id: &'static str,
    label: &'static str,
    value: T,
    cx: &App,
) -> impl IntoElement + use<T> {
    let t = design::tokens(cx);
    let value = value.into();
    v_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(format!("{label}: {value}"))
        .prefer_width(t.training.tile_width)
        .gap(t.space.hair)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(caption(label, cx))
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(value),
        )
}
