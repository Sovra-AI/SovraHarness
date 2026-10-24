//! Multiline native chat input styled after `SovraSRX`'s composer: a glass shell that
//! glows violet when focused, a round send button and a red stop button.
//! Enter sends, Shift+Enter adds a line, Stop cancels.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName,
    button::ButtonVariants as _,
    h_flex,
    input::{Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div, px,
};
use sovra_core::WorkspaceState;

use crate::chat_view::ChatPresentation;
use crate::design::{self, SovraStyled as _, TextRole};
use crate::shell::Shell;

pub(crate) fn render_composer(
    state: &WorkspaceState,
    composer_input: &Entity<TextareaState>,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let c = &t.chat;
    let has_draft = !composer_input.read(cx).value().trim().is_empty();
    let generating = state.is_generating;
    let mut bar = h_flex()
        .id("sovra-composer-bar")
        .w_full()
        .items_end()
        .gap(px(2.))
        .pl(px(16.))
        .pr(px(6.))
        .py(px(6.))
        .bg(c.composer_background)
        .rounded(c.composer_radius)
        .border_1()
        .border_color(if chat.composer_focused {
            c.composer_focus_border
        } else {
            c.composer_border
        });
    if chat.composer_focused {
        bar = bar.shadow(vec![c.composer_focus_glow.clone()]);
    }
    let send = design::round_icon_button("composer-send-btn", c.send_size)
        .icon(IconName::ArrowUp)
        .accessibility_label(if generating {
            "Queue message"
        } else {
            "Send message"
        })
        .disabled(!has_draft)
        .on_click(cx.listener(|shell, _, window, cx| shell.submit_composer(window, cx)));
    let send = if has_draft {
        send.bg(c.send_ready_background)
            .text_color(c.send_ready_foreground)
    } else {
        send.ghost().text_color(c.send_idle_foreground)
    };
    v_flex()
        .id("sovra-composer-container")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .pt(px(8.))
        .gap(t.space.compact)
        .children((!state.queued_prompts.is_empty()).then(|| {
            div()
                .id("composer-queue-strip")
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(match state.queued_prompts.len() {
                    1 => "1 message waiting to send".to_owned(),
                    count => format!("{count} messages waiting to send"),
                })
        }))
        .child(
            bar.child(
                div()
                    .id("chat-composer-input")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .py(px(4.))
                    .child(
                        Textarea::new(composer_input)
                            .aria_label("Message Sovra")
                            .appearance(false)
                            .bordered(false),
                    ),
            )
            .children(generating.then(|| {
                design::round_icon_button("composer-stop-btn", c.send_size)
                    .icon(IconName::Square)
                    .accessibility_label("Stop generating")
                    .bg(c.stop_background)
                    .text_color(c.user_bubble_foreground)
                    .on_click(cx.listener(|shell, _, _, cx| shell.interrupt_active_session(cx)))
            }))
            .child(send),
        )
        .child(
            h_flex().justify_center().child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().muted_foreground)
                    .child(if generating {
                        "Type ahead — Enter queues it for when Sovra is done"
                    } else {
                        "Enter to send · Shift+Enter for a new line"
                    }),
            ),
        )
}
