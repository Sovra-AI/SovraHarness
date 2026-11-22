//! Multiline native chat input: a quiet surface with violet focus treatment,
//! a round send button and a distinct stop button.
//! Enter sends, Shift+Enter adds a line, Stop cancels.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, button::ButtonVariants as _, h_flex,
    input::Textarea, v_flex,
};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    div,
};
use sovra_core::files::AttachmentState;
use sovra_core::models::ChromeKey;

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
    if design::is_rtl(cx) {
        bar = bar.flex_row_reverse();
    }
    let send = design::round_icon_button("composer-send-btn", c.send_size)
        .icon(IconName::ArrowUp)
        .accessibility_label(if generating {
            design::chrome(cx, ChromeKey::ChatQueue)
        } else {
            design::chrome(cx, ChromeKey::ChatSend)
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
                // Announced, and readable over remote control ("Usage: /table …").
                .role(Role::Alert)
                .aria_label(notice.clone())
                .sovra_text(TextRole::Caption, cx)
                .child(design::text_with_slash_pills("skill-notice", notice, cx))
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
                    .accessibility_label(design::chrome(cx, ChromeKey::ChatAddFiles))
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
                            .aria_label(design::chrome(cx, ChromeKey::ChatComposerA11y))
                            .appearance(false)
                            .bordered(false),
                    ),
            )
            .child(crate::team_view::composer_team_button(shell, cx))
            .children(generating.then(|| {
                design::round_icon_button("composer-stop-btn", c.send_size)
                    .icon(IconName::Square)
                    .accessibility_label(design::chrome(cx, ChromeKey::ChatStop))
                    .bg(cx.theme().danger)
                    .text_color(cx.theme().danger_foreground)
                    .on_click(cx.listener(|shell, _, _, cx| shell.interrupt_active_session(cx)))
            }))
            .child(send),
        )
        .children(render_hint(shell, has_files, cx))
}

/// The line under the message box. An empty chat always shows how to get help.
/// Once the conversation shows anything, tips are secondary: a window short for its
/// text size leaves them out so the conversation keeps room to read. Notes about
/// attached files always stay.
fn render_hint(shell: &Shell, has_files: bool, cx: &Context<Shell>) -> Option<AnyElement> {
    let state = &shell.state;
    let generating = state.is_generating;
    let row = h_flex().w_full().justify_center();
    let conversation = !state.messages.is_empty()
        || state
            .connections
            .outbound_messages
            .iter()
            .any(|message| message.session_id == state.session_id)
        || state.terminal.calls_for(&state.session_id).next().is_some();
    if shell.short_window && conversation && !has_files {
        return None;
    }
    if state.messages.is_empty()
        && !generating
        && !has_files
        && shell.composer_input.read(cx).value().is_empty()
        && !shell.started_chat_drafts.contains(&state.session_id)
    {
        return Some(
            row.child(render_help_hint(&state.session_id, cx))
                .into_any_element(),
        );
    }
    let hint = if has_files && generating {
        design::chrome(cx, ChromeKey::ChatHintFilesGenerating)
    } else if has_files {
        design::chrome(cx, ChromeKey::ChatHintFiles)
    } else if generating {
        design::chrome(cx, ChromeKey::ChatHintGenerating)
    } else {
        design::chrome(cx, ChromeKey::ChatHintIdle)
    };
    Some(
        row.child(
            div()
                .id("composer-hint")
                .test_support()
                // Wrap inside the column; without a width limit the hint keeps its
                // one-line width and spills past both edges at large text sizes.
                .min_w_0()
                .max_w_full()
                .aria_label(hint)
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().muted_foreground)
                .child(design::text_with_slash_pills("composer", hint, cx)),
        )
        .into_any_element(),
    )
}

fn render_help_hint(session_id: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let hint = design::chrome(cx, ChromeKey::ChatWelcomeHelpHint);
    let motion = design::tokens(cx).motion;
    let label = div()
        .id("chat-welcome-help-hint")
        .test_support()
        // Keep the complete instruction available to screen readers throughout the loop.
        .aria_label(hint)
        .relative()
        .w_full()
        .min_w_0()
        .text_center()
        .sovra_text(TextRole::Caption, cx)
        .text_color(cx.theme().muted_foreground);
    let Some((before, after)) = hint.split_once("/help") else {
        return label
            .child(design::text_with_slash_pills("welcome", hint, cx))
            .into_any_element();
    };
    // The pill stays put. Only the letters after it are typed, and the hidden
    // "help" reserves the width so the message box does not jump.
    let typed = h_flex()
        .items_center()
        .child(design::slash_pill(
            gpui_kit::SharedString::from("welcome-slash-pill-1"),
            cx,
        ))
        .child(
            div()
                .relative()
                .child(div().opacity(0.).child("help▏"))
                .child(div().absolute().top_0().left_0().with_animation(
                    gpui_kit::SharedString::from(format!("help-hint-{session_id}")),
                    Animation::new(motion.help_hint).repeat(),
                    move |text, progress| text.child(design::help_hint_command(progress)),
                )),
        );
    label
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap(design::tokens(cx).space.hair)
                .child(before.to_string())
                .child(if motion.help_hint.is_zero() {
                    h_flex()
                        .items_center()
                        .child(design::slash_pill(
                            gpui_kit::SharedString::from("welcome-slash-pill-1"),
                            cx,
                        ))
                        .child("help")
                        .into_any_element()
                } else {
                    typed.into_any_element()
                })
                .child(after.to_string()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};
    use sovra_core::{Command, NavTab};
    use sovra_runtime::Client;

    #[gpui_kit::test]
    fn help_hint_is_available_without_an_ai_and_returns_for_each_new_chat(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.state.models_loaded = true;
                shell.state.has_active_llm = false;
                shell.refresh(cx);
            });
            window.render_frame(cx);
            assert!(window.find("chat-welcome-help-hint").visible());
            assert!(window.find("welcome-slash-pill-1").visible());
            assert!(shell.read(cx).composer_input.read(cx).value().is_empty());
            assert_eq!(window.find("composer-send-btn").disabled(), Some(true));

            shell.update(cx, |shell, cx| {
                shell.fill_composer_draft("hello", window, cx);
            });
            window.render_frame(cx);
            assert!(window.try_find("chat-welcome-help-hint").is_none());
            assert!(window.find("composer-slash-pill-1").visible());
            assert!(window.find("composer-hint").label().unwrap().contains('/'));
        })
        .unwrap();
        // Input change subscriptions run after the update, just as between real keystrokes.
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.fill_composer_draft("", window, cx));
            window.render_frame(cx);
            assert!(window.try_find("chat-welcome-help-hint").is_none());
        })
        .unwrap();
        cx.run_until_parked();

        for _ in 0..2 {
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.start_new_chat(window, cx));
                window.render_frame(cx);
                assert!(window.find("chat-welcome-help-hint").visible());
                shell.update(cx, |shell, cx| {
                    shell.fill_composer_draft("/help", window, cx);
                });
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
                window.render_frame(cx);
                assert_eq!(shell.read(cx).state.active_tab, NavTab::Skills);
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Chat, cx));
                window.render_frame(cx);
                assert!(window.try_find("chat-welcome-help-hint").is_none());
            })
            .unwrap();
            cx.run_until_parked();
        }
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::RunPrompt { .. }))
        );
    }
}
