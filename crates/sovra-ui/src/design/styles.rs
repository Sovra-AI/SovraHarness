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

/// Keep wide code and tables inside the conversation's reading column.
pub fn markdown_style() -> gpui_kit::component::text::TextViewStyle {
    let mut scroll = gpui_kit::StyleRefinement::default();
    scroll.overflow.x = Some(gpui_kit::Overflow::Scroll);
    gpui_kit::component::text::TextViewStyle::default()
        .code_block(scroll.clone())
        .table(scroll)
}
