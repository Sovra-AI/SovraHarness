//! Center main chat column: "Built around you." hero banner, AG-UI conversation
//! stream, `SovraSRX` prompt timeline indicator, and bottom composer.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, h_flex, input::InputState, v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use sovra_core::{ChatMessage, MessageRole, WorkspaceState};

use crate::agui_widgets::render_agui_widget;
use crate::composer::render_composer;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_chat_column(
    state: &WorkspaceState,
    composer_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let user_prompt_count = state
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .count();

    v_flex()
        .id("sovra-chat-main")
        .flex_1()
        .min_w_0()
        .h_full()
        .justify_between()
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            v_flex()
                .id("chat-scroll-area") // overflow_y_scroll needs a stateful element
                .flex_1()
                .min_h_0()
                .gap(t.space.medium)
                .overflow_y_scroll()
                .child(render_hero_banner(user_prompt_count, cx))
                .child(
                    v_flex()
                        .id("chat-message-list")
                        .gap(t.space.medium)
                        .children(
                            state
                                .messages
                                .iter()
                                .enumerate()
                                .map(|(idx, msg)| render_message(msg, idx, cx)),
                        ),
                ),
        )
        .child(render_composer(state, composer_input, cx))
}

fn render_hero_banner(user_prompt_count: usize, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();

    h_flex()
        .id("chat-hero-banner")
        .w_full()
        .justify_between()
        .items_center()
        .p(t.space.large)
        .sovra_surface(Surface::HeroBanner, cx)
        .child(
            v_flex()
                .gap(px(4.))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(theme.link)
                        .child("YOUR AI. YOURS."),
                )
                .child(
                    div()
                        .sovra_text(TextRole::PageTitle, cx)
                        .child("Built around you."),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child("Your work. Your preferences. Your knowledge. Always private."),
                ),
        )
        .child(
            v_flex()
                .items_end()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .text_color(t.text_secondary)
                        .child("A more you"),
                )
                .child(
                    h_flex()
                        .px(t.space.small)
                        .py(px(3.))
                        .rounded(t.radius.pill)
                        .bg(t.subtle)
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("Timeline · {user_prompt_count} prompts")),
                ),
        )
}

fn render_message(
    msg: &ChatMessage,
    msg_idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    match msg.role {
        MessageRole::User => render_user_message(msg, msg_idx, cx).into_any_element(),
        MessageRole::Assistant => render_assistant_message(msg, msg_idx, cx).into_any_element(),
    }
}

fn render_user_message(
    msg: &ChatMessage,
    msg_idx: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();

    v_flex()
        .id(("user-msg", msg_idx))
        .w_full()
        .items_end()
        .gap(px(4.))
        .child(
            h_flex()
                .items_start()
                .justify_end()
                .gap(t.space.small)
                .child(
                    div()
                        .max_w(px(520.))
                        .px(t.space.medium)
                        .py(t.space.small)
                        .sovra_surface(Surface::UserBubble, cx)
                        .sovra_text(TextRole::Body, cx)
                        .child(msg.text.clone()),
                )
                .child(
                    h_flex()
                        .size(px(36.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.pill)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(msg.author.clone()),
                ),
        )
        .child(
            div()
                .pr(px(48.))
                .sovra_text(TextRole::Caption, cx)
                .text_color(theme.muted_foreground)
                .child(msg.timestamp.clone()),
        )
}

fn render_assistant_message(
    msg: &ChatMessage,
    msg_idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();

    h_flex()
        .id(("assistant-msg", msg_idx))
        .w_full()
        .items_start()
        .gap(t.space.small)
        .child(
            h_flex()
                .size(px(38.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(t.radius.pill)
                .bg(t.accent.violet)
                .child(
                    Icon::new(IconName::Sparkles)
                        .small()
                        .text_color(theme.primary_foreground),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.small)
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.compact)
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .child(msg.author.clone()),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(theme.muted_foreground)
                                .child(msg.timestamp.clone()),
                        ),
                )
                .children((!msg.text.is_empty()).then(|| {
                    div()
                        .max_w(px(560.))
                        .px(t.space.medium)
                        .py(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .sovra_text(TextRole::Body, cx)
                        .child(msg.text.clone())
                }))
                .children(
                    msg.widgets.iter().enumerate().map(|(w_idx, widget)| {
                        render_agui_widget(widget, msg_idx * 10 + w_idx, cx)
                    }),
                ),
        )
}

#[allow(dead_code)]
fn _keep_logo_import_used(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    logo("unused", t.layout.logo_small)
}
