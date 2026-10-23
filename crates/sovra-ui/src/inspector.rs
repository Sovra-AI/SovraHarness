//! Right "My Personal AI" builder inspector column matching the reference design.
//!
//! Includes `⌘ K` search input, "My Personal AI" status header, persona card,
//! 6 interactive configuration cards, and the "Preview my AI" primary CTA button.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
};
use sovra_core::{ConfigCardStatus, PersonalAiConfigCard, WorkspaceState};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_inspector(
    state: &WorkspaceState,
    search_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();

    v_flex()
        .id("sovra-inspector")
        .test_support()
        .w(t.layout.inspector_width)
        .h_full()
        .flex_shrink_0()
        .justify_between()
        .p(t.space.medium)
        .sovra_surface(Surface::Chrome, cx)
        .border_l_1()
        .border_color(theme.border)
        .overflow_y_scroll()
        .child(
            v_flex()
                .gap(t.space.small)
                .child(render_search_bar(search_input, cx))
                .child(render_inspector_header(state, cx))
                .child(render_persona_card(state, cx))
                .child(
                    v_flex().gap(t.space.compact).children(
                        state
                            .config_cards
                            .iter()
                            .enumerate()
                            .map(|(idx, card)| render_config_card(card, idx, cx)),
                    ),
                ),
        )
        .child(render_preview_cta(state, cx))
}

fn render_search_bar(
    search_input: &Entity<InputState>,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    h_flex()
        .id("inspector-search-bar")
        .w_full()
        .items_center()
        .gap(t.space.compact)
        .px(t.space.small)
        .py(px(4.))
        .rounded(t.radius.pill)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            Icon::new(IconName::Search)
                .small()
                .text_color(t.text_secondary),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(search_input)
                    .id("inspector-search-input")
                    .aria_label("Search anything")
                    .appearance(false)
                    .bordered(false),
            ),
        )
        .child(
            h_flex()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(6.))
                .bg(t.matte)
                .border_1()
                .border_color(theme.border)
                .sovra_text(TextRole::Caption, cx)
                .text_color(theme.muted_foreground)
                .child("⌘ K"),
        )
}

fn render_inspector_header(
    state: &WorkspaceState,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let status_label = if state.ai_building {
        "● Building..."
    } else {
        "● Ready"
    };
    v_flex()
        .gap(px(2.))
        .pt(t.space.compact)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(state.ai_name.clone()),
                        )
                        .child(
                            h_flex()
                                .id("inspector-status-pill")
                                .px(t.space.small)
                                .py(px(2.))
                                .rounded(t.radius.pill)
                                .bg(t.status_pill_background)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(cx.theme().success)
                                .child(status_label),
                        ),
                )
                .child(
                    h_flex()
                        .size(px(28.))
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.input)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            Icon::new(IconName::EllipsisVertical)
                                .small()
                                .text_color(t.text_secondary),
                        ),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Your AI, configured for you."),
        )
}

fn render_persona_card(state: &WorkspaceState, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("inspector-persona-card")
        .w_full()
        .items_center()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(logo("inspector-persona-orb", px(46.)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.compact)
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child(state.ai_name.clone()),
                        )
                        .child(
                            Icon::new(IconName::SquarePen)
                                .xsmall()
                                .text_color(t.text_secondary),
                        ),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(state.ai_description.clone()),
                ),
        )
}

fn render_config_card(
    card: &PersonalAiConfigCard,
    idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let icon = match card.key.as_str() {
        "knowledge" | "memory" => IconName::Database,
        "behaviour" => IconName::User,
        "guardrails" | "model" => IconName::Box,
        _ => IconName::Cloud,
    };
    let (status_icon, status_color) = match card.status {
        ConfigCardStatus::Finalizing => (IconName::LoaderCircle, t.accent.violet),
        _ => (IconName::CircleCheck, theme.success),
    };
    let key = card.key.clone();

    h_flex()
        .id(("inspector-card", idx))
        .role(Role::Button)
        .aria_label(card.title.clone())
        .cursor_pointer()
        .w_full()
        .items_center()
        .justify_between()
        .p(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(
                    h_flex()
                        .size(px(30.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.input)
                        .bg(t.subtle)
                        .child(Icon::new(icon).small().text_color(t.text_secondary)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child(card.title.clone()),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(card.subtitle.clone()),
                        ),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(4.))
                .child(Icon::new(status_icon).small().text_color(status_color))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(card.status.badge_text()),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                ),
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.handle_config_card_click(&key, cx);
        }))
}

fn render_preview_cta(state: &WorkspaceState, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let label = if state.ai_building {
        "▷ Preview my AI"
    } else {
        "✓ Personal AI Live · Previewing"
    };
    v_flex()
        .w_full()
        .pt(t.space.small)
        .gap(t.space.compact)
        .items_center()
        .child(
            design::primary_button("preview-my-ai-btn", label, cx)
                .w_full()
                .on_click(cx.listener(|shell, _, _, cx| shell.preview_personal_ai(cx))),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().muted_foreground)
                .child("Test your AI and make refinements before going live."),
        )
}
