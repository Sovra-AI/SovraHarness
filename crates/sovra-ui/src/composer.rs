//! Multiline native chat input: a quiet surface with violet focus treatment,
//! a round send button and a distinct stop button.
//! Enter sends, Shift+Enter adds a line, Stop cancels.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, button::ButtonVariants as _, h_flex,
    input::Textarea, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::files::AttachmentState;

use crate::design::{self, SovraStyled as _, TextRole};
use crate::shell::Shell;

pub(crate) fn render_composer(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let Shell {
        state,
        composer_input,
        chat,
        ..
    } = shell;
    let t = design::tokens(cx);
    let c = &t.chat;
    let files = state.file_drafts.get(&state.session_id);
    let has_files = files.is_some_and(|draft| !draft.files.is_empty());
    let files_ready = files.is_none_or(|draft| draft.ready().is_some());
    let has_draft = !composer_input.read(cx).value().trim().is_empty() || has_files;
    let can_send = has_draft && files_ready && !(has_files && state.is_generating);
    let generating = state.is_generating;
    let mut bar = h_flex()
        .id("sovra-composer-bar")
        .w_full()
        .items_end()
        .gap(t.space.hair)
        .pl(t.space.medium)
        .pr(t.space.small)
        .py(t.space.small)
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
        .disabled(!can_send)
        .on_click(cx.listener(|shell, _, window, cx| shell.submit_composer(window, cx)));
    let send = if can_send {
        send.bg(c.send_ready_background)
            .text_color(c.send_ready_foreground)
    } else {
        send.ghost().text_color(c.send_idle_foreground)
    };
    v_flex()
        .id("sovra-composer-container")
        .test_support()
        .relative()
        .w_full()
        .flex_shrink_0()
        .pt(t.space.medium)
        .gap(t.space.compact)
        .capture_key_down(cx.listener(Shell::skill_key))
        .capture_action(cx.listener(
            |shell, _: &gpui_kit::component::input::IndentInline, window, cx| {
                shell.skill_navigation("tab", window, cx);
            },
        ))
        .capture_action(cx.listener(
            |shell, _: &gpui_kit::component::input::MoveUp, window, cx| {
                shell.skill_navigation("up", window, cx);
            },
        ))
        .capture_action(cx.listener(
            |shell, _: &gpui_kit::component::input::MoveDown, window, cx| {
                shell.skill_navigation("down", window, cx);
            },
        ))
        .capture_action(cx.listener(
            |shell, _: &gpui_kit::component::input::Escape, window, cx| {
                shell.skill_navigation("escape", window, cx);
            },
        ))
        .capture_action(cx.listener(
            |shell, action: &gpui_kit::component::input::Enter, window, cx| {
                if action.shift || action.secondary {
                    cx.propagate();
                } else {
                    shell.skill_navigation("enter", window, cx);
                }
            },
        ))
        .children(shell.skill_menu.notice.as_ref().map(|notice| {
            div()
                .id("skill-command-notice")
                .test_support()
                .sovra_text(TextRole::Caption, cx)
                .child(notice.clone())
        }))
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
        .children(files.filter(|draft| !draft.files.is_empty()).map(|draft| {
            h_flex()
                .id("attachment-draft-strip")
                .test_support()
                .w_full()
                .gap(t.space.small)
                .overflow_x_scroll()
                .children(draft.files.iter().enumerate().map(|(index, file)| {
                    let id = file.id().to_owned();
                    let name = file.name().to_owned();
                    let details = match file {
                        AttachmentState::Ready(file) => {
                            crate::widgets::attachments::file_card(file, cx).into_any_element()
                        }
                        AttachmentState::Preparing { .. } | AttachmentState::Failed { .. } => {
                            v_flex()
                                .min_w_0()
                                .gap(t.space.compact)
                                .child(
                                    div()
                                        .sovra_text(TextRole::Body, cx)
                                        .text_ellipsis()
                                        .child(name.clone()),
                                )
                                .child(
                                    div()
                                        .sovra_text(TextRole::Caption, cx)
                                        .text_color(t.text_secondary)
                                        .child(
                                            if matches!(file, AttachmentState::Preparing { .. }) {
                                                "Getting ready…"
                                            } else {
                                                "Couldn't read this file"
                                            },
                                        ),
                                )
                                .into_any_element()
                        }
                    };
                    h_flex()
                        .w(t.layout.attachment_width)
                        .flex_shrink_0()
                        .items_center()
                        .gap(t.space.small)
                        .p(t.space.small)
                        .sovra_surface(design::Surface::Subtle, cx)
                        .child(div().flex_1().min_w_0().child(details))
                        .child(
                            design::button(("remove-attachment", index), "", cx)
                                .ghost()
                                .icon(IconName::Close)
                                .accessibility_label(format!("Remove {name}"))
                                .on_click(cx.listener(move |shell, _, _, cx| {
                                    shell.remove_attachment(&id, cx);
                                })),
                        )
                }))
        }))
        .children(files.and_then(|draft| draft.notice.as_ref()).map(|notice| {
            div()
                .sovra_text(TextRole::Caption, cx)
                .child(notice.clone())
        }))
        .children(
            files
                .and_then(|draft| {
                    draft.files.iter().find_map(|file| {
                        if let AttachmentState::Failed { message, .. } = file {
                            Some(message)
                        } else {
                            None
                        }
                    })
                })
                .map(|message| {
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(cx.theme().danger)
                        .child(message.clone())
                }),
        )
        .children(state.file_setup.as_ref().map(|setup| {
            let purpose = setup.purpose;
            v_flex()
                .id("attachment-model-guidance")
                .test_support()
                .w_full()
                .gap(t.space.small)
                .p(t.space.medium)
                .sovra_surface(design::Surface::Subtle, cx)
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child(setup.message.clone()),
                )
                .child(
                    design::primary_button("attachment-choose-model", "Help me choose an AI", cx)
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.open_file_model_setup(purpose, window, cx);
                        })),
                )
        }))
        .children(crate::team_view::composer_team_note(shell, cx))
        .child(
            bar.child(
                design::round_icon_button("composer-add-files", c.send_size)
                    .ghost()
                    .icon(gpui_kit::component::Icon::new(
                        gpui_kit::assets::IconName::Paperclip,
                    ))
                    .accessibility_label("Add files")
                    .on_click(cx.listener(|shell, _, window, cx| shell.choose_files(window, cx))),
            )
            .child(
                div()
                    .id("chat-composer-input")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .py(t.space.compact)
                    .child(
                        Textarea::new(composer_input)
                            .aria_label("Message Sovra")
                            .appearance(false)
                            .bordered(false),
                    ),
            )
            .child(crate::team_view::composer_team_button(shell, cx))
            .children(generating.then(|| {
                design::round_icon_button("composer-stop-btn", c.send_size)
                    .icon(IconName::Square)
                    .accessibility_label("Stop generating")
                    .bg(cx.theme().danger)
                    .text_color(cx.theme().danger_foreground)
                    .on_click(cx.listener(|shell, _, _, cx| shell.interrupt_active_session(cx)))
            }))
            .child(send),
        )
        .child(
            h_flex().justify_center().child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().muted_foreground)
                    .child(if has_files && generating {
                        "Send these files after this reply finishes"
                    } else if has_files {
                        "Up to 8 files · 20 MB each · Sent to your chosen AI when you send"
                    } else if generating {
                        "Type ahead — Enter queues it for when Sovra is done"
                    } else {
                        "Type / for skills · Enter to send · Shift+Enter for a new line"
                    }),
            ),
        )
}
