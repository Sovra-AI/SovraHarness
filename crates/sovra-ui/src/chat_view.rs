//! Selectable Markdown conversation with native virtualized, tail-following scrolling.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::TextareaState,
    text::{TextView, TextViewState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, FollowMode, InteractiveElement as _,
    IntoElement, ListAlignment, ListState, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, div, list,
};
use sovra_core::{AgUiWidget, ChatMessage, MessageRole, ResponseModel, WorkspaceState};

use crate::agui_widgets::render_agui_widget;
use crate::composer::render_composer;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

struct MarkdownMessage {
    source: String,
    widgets: Vec<AgUiWidget>,
    model: Option<ResponseModel>,
    state: Entity<TextViewState>,
    _changed: Subscription,
}

pub(crate) struct ChatPresentation {
    pub(crate) list: ListState,
    markdown: HashMap<String, MarkdownMessage>,
    session_id: String,
}

impl ChatPresentation {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, design::tokens(cx).space.large);
        list.set_follow_mode(FollowMode::Tail);
        list.set_scroll_handler(cx.listener(|_, _, _, cx| cx.notify()));
        Self {
            list,
            markdown: HashMap::new(),
            session_id: String::new(),
        }
    }

    pub(crate) fn sync(&mut self, state: &WorkspaceState, cx: &mut Context<Shell>) {
        if self.session_id != state.session_id {
            self.session_id.clone_from(&state.session_id);
            self.markdown.clear();
            self.list.reset(0);
            self.follow_latest();
        }
        let count = state.messages.len() + usize::from(state.is_generating);
        let previous = self.list.item_count();
        if count != previous {
            let start = previous.min(count);
            self.list.splice(start..previous, count - start);
            self.list.remeasure_items(start.saturating_sub(1)..count);
        }
        for (index, message) in state.messages.iter().enumerate() {
            if message.role != MessageRole::Assistant {
                continue;
            }
            if let Some(markdown) = self.markdown.get_mut(&message.id) {
                let text_changed = markdown.source != message.text;
                if text_changed {
                    markdown.source.clone_from(&message.text);
                    markdown
                        .state
                        .update(cx, |view, cx| view.set_text(&message.text, cx));
                }
                if text_changed
                    || markdown.widgets != message.widgets
                    || markdown.model != message.model
                {
                    markdown.model.clone_from(&message.model);
                    markdown.widgets.clone_from(&message.widgets);
                    self.list.remeasure_items(index..index + 1);
                }
            } else {
                let view = cx.new(|cx| TextViewState::markdown(&message.text, cx));
                // Parsing and font/layout changes can finish after the network chunk.
                // Let ListState preserve the reading position or follow the newly measured tail.
                let changed = cx.observe(&view, move |shell, _, cx| {
                    shell.chat.list.remeasure_items(index..index + 1);
                    cx.notify();
                });
                self.markdown.insert(
                    message.id.clone(),
                    MarkdownMessage {
                        source: message.text.clone(),
                        widgets: message.widgets.clone(),
                        model: message.model.clone(),
                        state: view,
                        _changed: changed,
                    },
                );
                self.list.remeasure_items(index..index + 1);
            }
        }
    }

    pub(crate) fn follow_latest(&self) {
        self.list.set_follow_mode(FollowMode::Tail);
    }
}

pub(crate) fn render_chat_column(
    state: &WorkspaceState,
    composer_input: &Entity<TextareaState>,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("sovra-chat-main")
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .px(t.space.medium)
        .pb(t.space.small)
        .overflow_hidden()
        .child(
            div()
                .id("chat-scroll-area")
                .role(Role::Log)
                .aria_label("Conversation")
                .test_support()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(
                    list(
                        chat.list.clone(),
                        cx.processor(|shell, index, _, cx| {
                            render_row(&shell.state, &shell.chat, index, cx).into_any_element()
                        }),
                    )
                    .size_full(),
                )
                .children((!chat.list.is_following_tail()).then(|| {
                    h_flex()
                        .absolute()
                        .bottom(t.space.small)
                        .w_full()
                        .justify_center()
                        .child(
                            design::button("chat-jump-latest", "Jump to latest", cx).on_click(
                                cx.listener(|shell, _, _, cx| {
                                    shell.chat.follow_latest();
                                    cx.notify();
                                }),
                            ),
                        )
                })),
        )
        .child(
            h_flex().w_full().justify_center().child(
                div()
                    .w_full()
                    .max_w(t.layout.reading_width)
                    .child(render_composer(state, composer_input, cx)),
            ),
        )
}

fn render_row(
    state: &WorkspaceState,
    chat: &ChatPresentation,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let row = if let Some(message) = state.messages.get(index) {
        match message.role {
            MessageRole::User => render_user_message(message, index, cx).into_any_element(),
            MessageRole::Assistant => {
                render_assistant_message(message, index, chat, cx).into_any_element()
            }
        }
    } else {
        div()
            .id("chat-response-status")
            .test_support()
            .sovra_text(TextRole::Caption, cx)
            .text_color(cx.theme().muted_foreground)
            .child("Responding…")
            .into_any_element()
    };
    h_flex().w_full().justify_center().child(
        div()
            .w_full()
            .max_w(t.layout.reading_width)
            .py(t.space.medium)
            .child(row),
    )
}

fn render_user_message(
    msg: &ChatMessage,
    index: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id(("user-msg", index))
        .w_full()
        .justify_end()
        .child(
            div()
                .max_w_full()
                .px(t.space.medium)
                .py(t.space.small)
                .sovra_surface(Surface::UserBubble, cx)
                .sovra_text(TextRole::Body, cx)
                .child(msg.text.clone()),
        )
}

fn render_assistant_message(
    msg: &ChatMessage,
    index: usize,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let markdown = chat
        .markdown
        .get(&msg.id)
        .expect("assistant Markdown is synchronized before rendering");
    let copy_text = msg.text.clone();
    v_flex()
        .id(("assistant-msg", index))
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .children((!msg.text.is_empty()).then(|| {
            div()
                .w_full()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .child(
                    TextView::new(&markdown.state)
                        .selectable(true)
                        .scrollable(false)
                        .stream_fade(true)
                        .style(design::markdown_style())
                        .code_block_actions(|code, _, cx| {
                            let text = code.code().to_string();
                            design::button("copy-code", "Copy code", cx)
                                .ghost()
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                })
                        }),
                )
        }))
        .children(
            msg.widgets
                .iter()
                .enumerate()
                .map(|(widget_index, widget)| {
                    render_agui_widget(widget, index * 10 + widget_index, cx)
                }),
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .children(
                    msg.model
                        .as_ref()
                        .map(|model| render_model_pill(model, index, cx)),
                )
                .children((!msg.text.is_empty()).then(|| {
                    design::button(("copy-message", index), "Copy", cx)
                        .ghost()
                        .accessibility_label("Copy response")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        })
                })),
        )
}

fn render_model_pill(
    model: &ResponseModel,
    index: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let name = if model.name.is_empty() {
        &model.model_id
    } else {
        &model.name
    };
    let label = if model.provider.is_empty() {
        name.clone()
    } else {
        format!("{name} · {}", model.provider)
    };
    let details = format!(
        "Response model: {name}\nModel ID: {}{}",
        model.model_id,
        if model.provider.is_empty() {
            String::new()
        } else {
            format!("\nConnection: {}", model.provider)
        }
    );
    h_flex()
        .id(("response-model", index))
        .aria_label(details.clone())
        .test_support()
        .max_w_full()
        .min_w_0()
        .gap(t.space.compact)
        .px(t.space.small)
        .py(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .rounded(t.radius.pill)
        .sovra_text(TextRole::Caption, cx)
        .tooltip(move |window, cx| Tooltip::new(details.clone()).build(window, cx))
        .child(
            Icon::new(IconName::Cpu)
                .xsmall()
                .flex_shrink_0()
                .text_color(cx.theme().link),
        )
        .child(div().min_w_0().truncate().child(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ScrollDelta, TestAppContext, point, px, size};
    use sovra_core::Event;
    use sovra_runtime::Client;

    #[gpui_kit::test]
    fn response_model_pill_fits_both_themes_and_preserves_copy(cx: &mut TestAppContext) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Hi", window, cx));
        })
        .unwrap();
        let model = ResponseModel {
            model_id: "organization/model-version-2026".into(),
            name: "A model with a deliberately long catalog display name".into(),
            provider: "My OpenRouter connection".into(),
        };
        for event in [
            Event::AgentResponseStarted {
                session_id: session.clone(),
                model: model.clone(),
            },
            Event::AgentChunk {
                session_id: session.clone(),
                text: "**Hello** from this model.".into(),
            },
            Event::AgentCompleted {
                session_id: session,
            },
        ] {
            events.send_blocking(event).unwrap();
        }
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                Theme::change(mode, Some(window), cx);
                window.render_frame(cx);
                let pill = window.find(("response-model", 1usize));
                assert!(pill.visible());
                assert!(pill.label().unwrap().contains(&model.model_id));
                assert!(pill.label().unwrap().contains(&model.provider));
                let bounds = pill.bounds();
                let chat = window.find("chat-scroll-area").bounds();
                assert!(bounds.left() >= chat.left());
                assert!(bounds.right() <= chat.right());
                window.click(("copy-message", 1usize), cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    "**Hello** from this model."
                );
            }
            assert_eq!(
                shell.read(cx).state.messages[1].model.as_ref(),
                Some(&model)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn streamed_markdown_follows_tail_without_stealing_the_reading_position(
        cx: &mut TestAppContext,
    ) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.send_prompt_text("Explain Markdown", window, cx);
            });
            window.render_frame(cx);
            assert!(window.find("chat-history-rail-collapsed").visible());
            window.click("chat-history-expand-btn", cx);
            window.render_frame(cx);
            assert!(window.find("chat-history-rail").visible());
            window.click("chat-history-collapse-btn", cx);
            window.render_frame(cx);
            assert!(window.try_find("chat-history-rail").is_none());
        })
        .unwrap();
        let markdown = "# Heading\n\n**Bold** and [link](https://example.com).\n\n- One\n- Two\n\n```rust\nlet answer = 42;\n```\n\n| Name | Value |\n| --- | --- |\n| Answer | 42 |\n\n";
        events
            .send_blocking(Event::AgentChunk {
                session_id: session.clone(),
                text: markdown.repeat(12),
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = shell.read(cx);
            assert!(view.chat.list.is_following_tail());
            let last = view.state.messages.last().unwrap();
            let rendered = view.chat.markdown[&last.id].state.read(cx).rendered_text();
            assert!(rendered.as_str().contains("Heading"));
            assert!(rendered.as_str().contains("Bold"));
            assert!(rendered.as_str().contains("let answer = 42;"));
            assert!(!rendered.as_str().contains("**Bold**"));
            assert!(!rendered.as_str().contains("```"));
            assert!(!rendered.as_str().contains("https://example.com"));
            assert!(window.find("chat-composer-input").bounds().size.width >= px(160.));
            assert!(window.find("composer-send-btn").bounds().right() <= px(640.));
            assert!(window.find("chat-response-status").visible());
            window.scroll(
                "chat-scroll-area",
                ScrollDelta::Pixels(point(px(0.), px(400.))),
                cx,
            );
            window.render_frame(cx);
            assert!(!shell.read(cx).chat.list.is_following_tail());
            assert!(window.find("chat-jump-latest").visible());
        })
        .unwrap();
        let position = shell.read_with(cx, |shell, _| shell.chat.list.logical_scroll_top());
        events
            .send_blocking(Event::AgentChunk {
                session_id: session.clone(),
                text: "\n\nLatest text".repeat(20),
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let after = shell.read(cx).chat.list.logical_scroll_top();
            assert_eq!(after.item_ix, position.item_ix);
            assert_eq!(after.offset_in_item, position.offset_in_item);
            window.click("chat-jump-latest", cx);
            window.render_frame(cx);
            assert!(shell.read(cx).chat.list.is_following_tail());
            assert!(window.find("chat-response-status").visible());
            Theme::change(ThemeMode::Light, Some(window), cx);
            window.render_frame(cx);
            assert!(window.find("sovra-composer-container").visible());
        })
        .unwrap();
        events
            .send_blocking(Event::AgentCompleted {
                session_id: session,
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("chat-response-status").is_none());
            assert!(shell.read(cx).chat.list.is_following_tail());
            window.click(("copy-message", 1usize), cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                shell.read(cx).state.messages[1].text
            );
        })
        .unwrap();
    }
}
