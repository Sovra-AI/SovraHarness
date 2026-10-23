//! Bottom chat composer with `SovraSRX` queue strip, attachment button, GPUI text input,
//! voice controls, and send/interrupt button.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use sovra_core::WorkspaceState;

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_composer(
    state: &WorkspaceState,
    composer_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    // Copy out before helpers take `&mut cx`; holding the theme borrow across them is E0502.
    let muted_foreground = cx.theme().muted_foreground;

    v_flex()
        .id("sovra-composer-container")
        .w_full()
        .flex_shrink_0()
        .gap(t.space.compact)
        .pt(t.space.compact)
        .children(render_queue_strip(&state.queued_prompts, cx))
        .child(
            h_flex()
                .id("sovra-composer-bar")
                .w_full()
                .items_center()
                .gap(t.space.small)
                .px(t.space.small)
                .py(px(6.))
                .rounded(t.radius.pill)
                .sovra_surface(Surface::Matte, cx)
                .child(
                    h_flex()
                        .id("composer-attach-btn")
                        .role(Role::Button)
                        .aria_label("Attach files or knowledge")
                        .cursor_pointer()
                        .size(px(32.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.pill)
                        .bg(t.subtle)
                        .child(
                            Icon::new(IconName::Plus)
                                .small()
                                .text_color(t.text_secondary),
                        )
                        .on_click(cx.listener(|shell, _, window, cx| {
                            shell.fill_composer_draft(
                                "Connect my latest workspace notes and summarize key priorities.",
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(composer_input)
                            .id("chat-composer-input")
                            .aria_label("Message your SOVRA")
                            .appearance(false)
                            .bordered(false),
                    ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.compact)
                        .child(
                            h_flex()
                                .id("composer-voice-btn")
                                .role(Role::Button)
                                .aria_label("Voice input")
                                .cursor_pointer()
                                .size(px(30.))
                                .items_center()
                                .justify_center()
                                .rounded(t.radius.pill)
                                .child(
                                    Icon::new(IconName::Mic)
                                        .small()
                                        .text_color(t.text_secondary),
                                ),
                        )
                        .child(render_send_or_stop_button(state.is_generating, cx)),
                ),
        )
        .child(
            h_flex().w_full().justify_center().child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(muted_foreground)
                    .child("SOVRA can make mistakes. Always verify important information."),
            ),
        )
}

fn render_send_or_stop_button(
    is_generating: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (id, label, icon, bg) = if is_generating {
        (
            "composer-stop-btn",
            "Stop generating",
            IconName::Square,
            t.accent.violet_deep,
        )
    } else {
        (
            "composer-send-btn",
            "Send message",
            IconName::ArrowUp,
            t.accent.violet,
        )
    };

    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .size(px(36.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(t.radius.pill)
        .bg(bg)
        .child(Icon::new(icon).small().text_color(theme.primary_foreground))
        .on_click(cx.listener(move |shell, _, window, cx| {
            if is_generating {
                shell.interrupt_active_session(cx);
            } else {
                shell.submit_composer(window, cx);
            }
        }))
}

fn render_queue_strip(queued: &[String], cx: &Context<Shell>) -> Option<impl IntoElement + use<>> {
    if queued.is_empty() {
        return None;
    }
    let t = design::tokens(cx);
    Some(
        h_flex()
            .id("composer-queue-strip")
            .w_full()
            .px(t.space.small)
            .py(px(4.))
            .items_center()
            .justify_between()
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(format!("Queued ({}) · {}", queued.len(), queued[0])),
            ),
    )
}
