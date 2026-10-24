//! Small styling recipes; GPUI retains ownership of control behaviour and accessibility.

use gpui_kit::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{App, ElementId, FontWeight, SharedString, Styled, relative, rems};

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
            Surface::Subtle | Surface::UserBubble => element
                .rounded(t.radius.card)
                .border_1()
                .border_color(t.material_border),
            Surface::NavActive => element.rounded(t.radius.input),
            Surface::Glass | Surface::Matte | Surface::HeroBanner => element
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
            TextRole::Display => (3.25, 1.04, FontWeight::BOLD),
            TextRole::PageTitle => (2.0, 1.12, FontWeight::BOLD),
            TextRole::SectionTitle => (1.15, 1.25, FontWeight::SEMIBOLD),
            TextRole::Body => (0.9375, 1.5, FontWeight::NORMAL),
            TextRole::Reading => (1.0, 1.7, FontWeight::NORMAL),
            TextRole::Caption => (0.78, 1.4, FontWeight::NORMAL),
        };
        self.font_family(cx.theme().font_family.clone())
            .text_size(rems(size))
            .line_height(relative(leading))
            .font_weight(weight)
    }
}

impl<T: Styled> SovraStyled for T {}

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
        .px(t.space.large)
        .font_weight(FontWeight::SEMIBOLD)
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
pub fn round_icon_button(id: impl Into<ElementId>, size: gpui_kit::Pixels) -> Button {
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
    let mut code = gpui_kit::StyleRefinement::default();
    code.overflow.x = Some(gpui_kit::Overflow::Scroll);
    let code = code.rounded(t.chat.code_radius);
    let mut table = gpui_kit::StyleRefinement::default();
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    gpui_kit::component::text::TextViewStyle::default()
        .paragraph_gap(rems(1.0))
        // Relative to the 16px reading size, not the component's 14px heading base.
        .heading_font_size(|level, _| match level {
            1 => gpui_kit::px(18.4),
            2 => gpui_kit::px(16.8),
            _ => gpui_kit::px(15.2),
        })
        .code_block(code)
        .table(table)
}
