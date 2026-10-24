//! Multiline native chat input: Enter sends, Shift+Enter adds a line, Stop cancels.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName,
    button::ButtonVariants as _,
    h_flex,
    input::{Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::WorkspaceState;

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_composer(
    state: &WorkspaceState,
    composer_input: &Entity<TextareaState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let has_draft = !composer_input.read(cx).value().trim().is_empty();
    let generating = state.is_generating;
    v_flex()
        .id("sovra-composer-container")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .gap(t.space.compact)
        .children((!state.queued_prompts.is_empty()).then(|| {
            div()
                .id("composer-queue-strip")
                .sovra_text(TextRole::Caption, cx)
                .child(match state.queued_prompts.len() {
                    1 => "1 message queued".to_owned(),
                    count => format!("{count} messages queued"),
                })
        }))
        .child(
            h_flex()
                .id("sovra-composer-bar")
                .w_full()
                .items_end()
                .gap(t.space.small)
                .p(t.space.small)
                .sovra_surface(Surface::Matte, cx)
                .child(
                    div()
                        .id("chat-composer-input")
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .child(
                            Textarea::new(composer_input)
                                .aria_label("Message Sovra")
                                .appearance(false)
                                .bordered(false),
                        ),
                )
                .children(generating.then(|| {
                    design::button("composer-stop-btn", "", cx)
                        .accessibility_label("Stop generating")
                        .icon(IconName::Square)
                        .px(t.space.small)
                        .on_click(cx.listener(|shell, _, _, cx| shell.interrupt_active_session(cx)))
                }))
                .child(
                    design::button("composer-send-btn", "", cx)
                        .primary()
                        .accessibility_label(if generating {
                            "Queue message"
                        } else {
                            "Send message"
                        })
                        .icon(IconName::ArrowUp)
                        .px(t.space.small)
                        .disabled(!has_draft)
                        .on_click(
                            cx.listener(|shell, _, window, cx| shell.submit_composer(window, cx)),
                        ),
                ),
        )
        .child(
            h_flex().justify_center().child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().muted_foreground)
                    .child(if generating {
                        "Enter to queue · Shift+Enter for a new line"
                    } else {
                        "Enter to send · Shift+Enter for a new line"
                    }),
            ),
        )
}
