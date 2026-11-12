//! Small styling recipes; GPUI retains ownership of control behaviour and accessibility.

use gpui_kit::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{
    App, Div, ElementId, FontWeight, InteractiveElement as _, ParentElement as _, SharedString,
    Styled, div, relative, rems,
};

use super::tokens;

#[derive(Debug, Clone, Copy)]
pub enum Surface {
    Canvas,
    Chrome,
    Glass,
    Subtle,
    Matte,
    Elevated,
    UserBubble,
    HeroBanner,
    NavActive,
}

#[derive(Debug, Clone, Copy)]
pub enum TextRole {
    Display,
    PageTitle,
    SectionTitle,
    Body,
    /// Assistant replies: `SovraSRX`'s 16px / 1.75 chat paragraphs.
    Reading,
    Caption,
}

/// Shared appearance only: does not install click/focus handlers on a plain div.
pub trait SovraStyled: Styled + Sized {
    #[must_use]
    fn sovra_surface(self, surface: Surface, cx: &App) -> Self {
        let t = tokens(cx);
        let background = match surface {
            Surface::Canvas => t.canvas,
            Surface::Chrome => t.chrome,
            Surface::Glass => t.glass,
            Surface::Subtle => t.subtle,
            Surface::Matte => t.matte,
            Surface::Elevated => t.elevated,
            Surface::UserBubble => t.user_bubble_background,
            Surface::HeroBanner => t.hero_banner_background,
            Surface::NavActive => t.nav_active_background,
        };
        let element = self.bg(background).text_color(cx.theme().foreground);
        match surface {
            Surface::Canvas | Surface::Chrome => element,
            Surface::Subtle | Surface::UserBubble | Surface::Matte | Surface::HeroBanner => element
                .rounded(t.radius.card)
                .border_1()
                .border_color(t.material_border),
            Surface::NavActive => element.rounded(t.radius.input),
            Surface::Glass => element
                .rounded(t.radius.card)
                .border_1()
                .border_color(t.material_border)
                .shadow(vec![t.material_shadow]),
            Surface::Elevated => element
                .rounded(t.radius.panel)
                .border_1()
                .border_color(t.material_border_strong)
                .shadow(vec![t.elevated_shadow]),
        }
    }

    #[must_use]
    fn sovra_text(self, role: TextRole, cx: &App) -> Self {
        let (size, leading, weight) = match role {
            TextRole::Display => (2.25, 1.2, FontWeight::SEMIBOLD),
            TextRole::PageTitle => (1.75, 1.2, FontWeight::SEMIBOLD),
            TextRole::SectionTitle => (1.0, 1.4, FontWeight::SEMIBOLD),
            TextRole::Body => (0.875, 1.5, FontWeight::NORMAL),
            TextRole::Reading => (1.0, 1.7, FontWeight::NORMAL),
            TextRole::Caption => (0.75, 1.5, FontWeight::NORMAL),
        };
        self.font_family(cx.theme().font_family.clone())
            .text_size(rems(size))
            .line_height(relative(leading))
            .font_weight(weight)
    }
}

impl<T: Styled> SovraStyled for T {}

/// A wrapping row's preferred width. The item may shrink to the pane, so a
/// 200% text size on the minimum window does not force a horizontal overflow.
/// A hard `min_w` of the same token cannot.
pub trait FitsPane: Styled + Sized {
    #[must_use]
    fn prefer_width(self, width: impl Into<gpui_kit::Length>) -> Self {
        self.flex_grow_1()
            .flex_shrink_1()
            .flex_basis(width)
            .min_w_0()
            .max_w_full()
    }
}

impl<T: Styled> FitsPane for T {}

/// Apply Sovra's geometry to a real GPUI button; variants still come from the theme.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let t = tokens(cx);
    let label = label.into();
    let mut button = Button::new(id);
    if !label.is_empty() {
        button = button.label(label);
    }
    button
        .cursor_pointer()
        .rounded(t.radius.button)
        .min_h(t.layout.control_height)
        .max_w_full()
        .min_w_0()
        .px(t.space.medium)
        .font_weight(FontWeight::MEDIUM)
}

/// Title-bar controls fit the native chrome even at 200% interface size.
/// Caption typography preserves room for both the label and keyboard focus.
pub fn title_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    button(id, label, cx)
        .min_h_0()
        .h_full()
        .sovra_text(TextRole::Caption, cx)
        .line_height(relative(1.0))
}

/// Brand gradient and neutral elevation, keeping GPUI's focus and input handling.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    let t = tokens(cx);
    button(id, label, cx)
        .primary()
        .shadow(vec![t.button_shadow])
}

/// Round icon-only button (composer send/stop). GPUI keeps focus, hover and
/// disabled handling; callers pick the fill from `tokens(cx).chat`.
pub fn round_icon_button(id: impl Into<ElementId>, size: gpui_kit::Rems) -> Button {
    Button::new(id)
        .cursor_pointer()
        .rounded_full()
        .size(size)
        .min_h(size)
        .p_0()
}

/// `SovraSRX` chat Markdown: 16px reading text with roomy paragraphs, compact
/// headings (1.15 / 1.05 / 0.95rem) and rounded code blocks that scroll sideways
/// instead of widening the reading column.
pub fn markdown_style(cx: &App) -> gpui_kit::component::text::TextViewStyle {
    let t = tokens(cx);
    let root = cx.theme().font_size;
    let mut code = gpui_kit::StyleRefinement::default();
    code.overflow.x = Some(gpui_kit::Overflow::Scroll);
    let code = code.rounded(t.chat.code_radius);
    let mut table = gpui_kit::StyleRefinement::default();
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    gpui_kit::component::text::TextViewStyle::default()
        .paragraph_gap(rems(1.0))
        // Relative to the reading size (1rem), not the component's 14px heading
        // base; resolved from the live root so headings follow the interface scale.
        .heading_font_size(move |level, _| match level {
            1 => root * 1.15,
            2 => root * 1.05,
            _ => root * 0.95,
        })
        .code_block(code)
        .table(table)
}

/// A shared title rhythm for workspace pages; actions sit beside this at wide sizes.
pub fn page_header(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let t = tokens(cx);
    let mut header = div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .max_w_full()
        .gap(t.space.small);
    if super::is_rtl(cx) {
        header = header.text_right();
    }
    header
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::PageTitle, cx)
                .child(super::paragraph(title).id("page-title")),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(super::paragraph(description).id("page-description")),
        )
}
