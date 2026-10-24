//! The standard chat, styled after `SovraSRX`: bare assistant replies beside the Sovra
//! avatar, gradient user bubbles, a "Thinking…" pill with an elapsed timer and a
//! collapsible view of the model's reasoning. Rendering stays cheap while streaming:
//! rows are virtualized, Markdown is fed only appended bytes (parsed off the UI
//! thread), collapsed reasoning is never built, and the only continuous animation is
//! the shimmer shown before the first answer token.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::TextareaState,
    shimmer::ShimmerText,
    text::{TextView, TextViewState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, FollowMode, InteractiveElement as _,
    IntoElement, ListAlignment, ListState, ParentElement as _, Pixels, Role, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, div, list, px,
    relative,
};
use sovra_core::{AgUiWidget, ChatMessage, MessageRole, ResponseModel, WorkspaceState};

use crate::agui_widgets::render_agui_widget;
use crate::composer::render_composer;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

struct MarkdownMessage {
    /// Bytes of `ChatMessage::text` already handed to `state`. Messages only grow
    /// while streaming, so the new suffix is appended without copying or comparing
    /// the text received so far.
    answer_len: usize,
    widgets: Vec<AgUiWidget>,
    model: Option<ResponseModel>,
    state: Entity<TextViewState>,
    thinking: Option<ThinkingView>,
    _changed: Subscription,
}

struct ThinkingView {
    len: usize,
    state: Entity<TextViewState>,
    /// Keeps the capped reasoning panel pinned to its newest line while it streams.
    scroll: ScrollHandle,
    _changed: Subscription,
}

pub(crate) struct ChatPresentation {
    pub(crate) list: ListState,
    markdown: HashMap<String, MarkdownMessage>,
    /// Reasoning disclosures the reader opened (or closed while streaming).
    pub(crate) thinking_toggled: HashSet<String>,
    session_id: String,
    /// When the current turn started, for the "Thinking… 12s" timer.
    pub(crate) turn_started: Option<Instant>,
    was_generating: bool,
    pub(crate) composer_focused: bool,
    /// Window width, for `SovraSRX`'s responsive reading column.
    pub(crate) viewport_width: Pixels,
}

/// Feeds a markdown view only the bytes it has not seen. A shorter text means the
/// message was replaced (not streamed), so it is re-parsed in full.
fn feed(
    view: &Entity<TextViewState>,
    seen: &mut usize,
    text: &str,
    cx: &mut Context<Shell>,
) -> bool {
    if text.len() == *seen {
        return false;
    }
    if text.len() > *seen && text.is_char_boundary(*seen) {
        let delta = &text[*seen..];
        view.update(cx, |view, cx| view.push_str(delta, cx));
    } else {
        view.update(cx, |view, cx| view.set_text(text, cx));
    }
    *seen = text.len();
    true
}

fn markdown_view(
    text: &str,
    index: usize,
    cx: &mut Context<Shell>,
) -> (Entity<TextViewState>, Subscription) {
    let view = cx.new(|cx| TextViewState::markdown(text, cx));
    // Parsing runs on a background executor and can finish after the network chunk.
    // Let ListState preserve the reading position or follow the newly measured tail.
    let changed = cx.observe(&view, move |shell, _, cx| {
        shell.chat.list.remeasure_items(index..index + 1);
        cx.notify();
    });
    (view, changed)
}

impl ChatPresentation {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        // Rows carry SovraSRX's 8px vertical padding; the list adds only a small gap.
        let list = ListState::new(0, ListAlignment::Top, design::tokens(cx).space.compact);
        list.set_follow_mode(FollowMode::Tail);
        list.set_scroll_handler(cx.listener(|_, _, _, cx| cx.notify()));
        Self {
            list,
            markdown: HashMap::new(),
            thinking_toggled: HashSet::new(),
            session_id: String::new(),
            turn_started: None,
            was_generating: false,
            composer_focused: false,
            viewport_width: px(0.),
        }
    }

    pub(crate) fn sync(&mut self, state: &WorkspaceState, cx: &mut Context<Shell>) {
        if self.session_id != state.session_id {
            self.session_id.clone_from(&state.session_id);
            self.markdown.clear();
            self.thinking_toggled.clear();
            self.list.reset(0);
            self.follow_latest();
        }
        if state.is_generating != self.was_generating {
            self.was_generating = state.is_generating;
            self.turn_started = state.is_generating.then(Instant::now);
            // The streaming row gains/loses its live line and action row.
            let last = self.list.item_count();
            self.list.remeasure_items(last.saturating_sub(1)..last);
        }
        let count = state.messages.len() + usize::from(awaiting_first_output(state));
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
                let mut changed =
                    feed(&markdown.state, &mut markdown.answer_len, &message.text, cx);
                match &mut markdown.thinking {
                    Some(thinking) => {
                        changed |= feed(&thinking.state, &mut thinking.len, &message.thinking, cx);
                    }
                    None if !message.thinking.is_empty() => {
                        let (state, changed_sub) = markdown_view(&message.thinking, index, cx);
                        markdown.thinking = Some(ThinkingView {
                            len: message.thinking.len(),
                            state,
                            scroll: ScrollHandle::new(),
                            _changed: changed_sub,
                        });
                        changed = true;
                    }
                    None => {}
                }
                if changed || markdown.widgets != message.widgets || markdown.model != message.model
                {
                    markdown.model.clone_from(&message.model);
                    markdown.widgets.clone_from(&message.widgets);
                    self.list.remeasure_items(index..index + 1);
                }
            } else {
                let (view, changed) = markdown_view(&message.text, index, cx);
                let thinking = (!message.thinking.is_empty()).then(|| {
                    let (state, changed) = markdown_view(&message.thinking, index, cx);
                    ThinkingView {
                        len: message.thinking.len(),
                        state,
                        scroll: ScrollHandle::new(),
                        _changed: changed,
                    }
                });
                self.markdown.insert(
                    message.id.clone(),
                    MarkdownMessage {
                        answer_len: message.text.len(),
                        widgets: message.widgets.clone(),
                        model: message.model.clone(),
                        state: view,
                        thinking,
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

/// The turn is running but no assistant message exists yet: show a waiting row.
fn awaiting_first_output(state: &WorkspaceState) -> bool {
    state.is_generating
        && state
            .messages
            .last()
            .is_none_or(|message| message.role != MessageRole::Assistant)
}

/// `SovraSRX`'s elapsed format: `42s`, `3m`, `3m 12s`.
pub(crate) fn elapsed_label(ms: u64) -> String {
    let seconds = ms / 1000;
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, s) => format!("{m}m {s}s"),
    }
}

fn column_width(chat: &ChatPresentation, cx: &Context<Shell>) -> Option<Pixels> {
    design::tokens(cx)
        .chat
        .column_widths
        .iter()
        .rev()
        .find(|(breakpoint, _)| chat.viewport_width >= *breakpoint)
        .map(|(_, width)| *width)
}

fn column(chat: &ChatPresentation, cx: &Context<Shell>) -> gpui_kit::Div {
    let column = div().w_full().min_w_0();
    match column_width(chat, cx) {
        Some(width) => column.max_w(width),
        None => column,
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
                            design::button("chat-jump-latest", "Jump to latest", cx)
                                .icon(IconName::ChevronDown)
                                .small()
                                .sovra_surface(Surface::Glass, cx)
                                .rounded(t.radius.pill)
                                .on_click(cx.listener(|shell, _, _, cx| {
                                    shell.chat.follow_latest();
                                    cx.notify();
                                })),
                        )
                })),
        )
        .child(
            h_flex()
                .w_full()
                .justify_center()
                .child(column(chat, cx).child(render_composer(state, composer_input, chat, cx))),
        )
}

fn render_row(
    state: &WorkspaceState,
    chat: &ChatPresentation,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let row = if let Some(message) = state.messages.get(index) {
        match message.role {
            MessageRole::User => render_user_message(state, message, index, cx).into_any_element(),
            MessageRole::Assistant => {
                let streaming = state.is_generating && index + 1 == state.messages.len();
                render_assistant_message(message, index, streaming, chat, cx).into_any_element()
            }
        }
    } else {
        h_flex()
            .w_full()
            .items_start()
            .gap(design::tokens(cx).chat.row_gap)
            .child(assistant_avatar(true, cx))
            .child(waiting_pill("Sovra is thinking…", chat, cx))
            .into_any_element()
    };
    h_flex()
        .w_full()
        .justify_center()
        .child(column(chat, cx).py(px(8.)).child(row))
}

fn render_user_message(
    state: &WorkspaceState,
    msg: &ChatMessage,
    index: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let chat = &t.chat;
    let copy_text = msg.text.clone();
    h_flex()
        .id(("user-msg", index))
        .w_full()
        .justify_end()
        .items_start()
        .gap(chat.row_gap)
        .child(
            design::button(("copy-user-message", index), "", cx)
                .ghost()
                .xsmall()
                .child(
                    Icon::new(IconName::Copy)
                        .xsmall()
                        .text_color(t.text_secondary),
                )
                .px(t.space.compact)
                .accessibility_label("Copy message")
                .opacity(chat.action_idle_opacity)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                }),
        )
        .child(
            div()
                .max_w(relative(0.74))
                .min_w_0()
                .px(px(16.))
                .py(t.space.small)
                .bg(chat.user_bubble)
                .rounded_tl(chat.user_bubble_radius)
                .rounded_tr(chat.user_bubble_radius)
                .rounded_bl(chat.user_bubble_radius)
                .rounded_br(chat.user_bubble_tail_radius)
                .shadow(vec![chat.user_bubble_shadow.clone()])
                .sovra_text(TextRole::Body, cx)
                .text_color(chat.user_bubble_foreground)
                .child(msg.text.clone()),
        )
        .child(
            h_flex()
                .size(chat.avatar)
                .mt(px(4.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.nav_active_background)
                .sovra_text(TextRole::Caption, cx)
                .child(state.user.initials.clone()),
        )
}

/// The Sovra logo; a ring marks the reply that is still being written. The ring is
/// static on purpose: a pulsing one would redraw the window every frame.
fn assistant_avatar(active: bool, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let chat = &t.chat;
    h_flex()
        .relative()
        .size(chat.avatar)
        .mt(px(4.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.subtle)
        .child(logo("assistant-avatar", chat.avatar - px(8.)))
        .children(active.then(|| {
            div()
                .absolute()
                .top(px(-3.))
                .left(px(-3.))
                .size(chat.avatar + px(6.))
                .rounded_full()
                .border_1()
                .border_color(chat.avatar_ring)
        }))
}

fn turn_elapsed(chat: &ChatPresentation) -> String {
    let ms = chat.turn_started.map_or(0, |started| {
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
    });
    elapsed_label(ms)
}

/// `SovraSRX`'s pre-answer pill: shimmering label plus a once-a-second timer.
fn waiting_pill(
    label: &'static str,
    chat: &ChatPresentation,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("chat-response-status")
        .test_support()
        .role(Role::Status)
        .aria_label(label)
        .mt(px(4.))
        .gap(t.space.compact)
        .items_center()
        .px(px(10.))
        .py(px(4.))
        .rounded(t.radius.pill)
        .bg(t.chat.thinking_pill)
        .sovra_text(TextRole::Caption, cx)
        .italic()
        .child(ShimmerText::new(label))
        .child(div().text_color(t.text_secondary).child(turn_elapsed(chat)))
}

/// While the answer streams: a static dot, what Sovra is doing and the elapsed time.
fn live_line(chat: &ChatPresentation, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("chat-response-status")
        .test_support()
        .role(Role::Status)
        .aria_label("Sovra is writing")
        .gap(t.space.compact)
        .items_center()
        .sovra_text(TextRole::Caption, cx)
        .text_color(t.text_secondary)
        .child(div().size(px(8.)).rounded_full().bg(t.accent.violet))
        .child("Writing…")
        .child(format!("· {}", turn_elapsed(chat)))
}

fn render_thinking(
    msg: &ChatMessage,
    thinking: &ThinkingView,
    index: usize,
    live: bool,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    // Open while the model is reasoning, folded once the answer starts — unless the
    // reader chose otherwise.
    let open = live != chat.thinking_toggled.contains(&msg.id);
    let label = if live {
        format!("Thinking… {}", turn_elapsed(chat))
    } else {
        msg.thinking_ms.map_or_else(
            || "Thought for a moment".to_owned(),
            |ms| format!("Thought for {}", elapsed_label(ms.max(1000))),
        )
    };
    let id = msg.id.clone();
    let mut block = v_flex()
        .id(("thinking", index))
        .w_full()
        .gap(t.space.compact)
        .child(
            h_flex().child(
                design::button(("thinking-toggle", index), "", cx)
                    .ghost()
                    .xsmall()
                    .px(t.space.compact)
                    .accessibility_label(if open {
                        "Hide reasoning"
                    } else {
                        "Show reasoning"
                    })
                    .child(
                        h_flex()
                            .gap(t.space.compact)
                            .items_center()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            )
                            .child(if live {
                                ShimmerText::new(label).into_any_element()
                            } else {
                                div().child(label).into_any_element()
                            }),
                    )
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        if !shell.chat.thinking_toggled.remove(&id) {
                            shell.chat.thinking_toggled.insert(id.clone());
                        }
                        shell.chat.list.remeasure_items(index..index + 1);
                        cx.notify();
                    })),
            ),
        );
    if open {
        if live {
            thinking.scroll.scroll_to_bottom();
        }
        block = block.child(
            div()
                .id(("thinking-body", index))
                .test_support()
                .max_h(t.chat.thinking_max_height)
                .overflow_y_scroll()
                .track_scroll(&thinking.scroll)
                .pl(t.space.small)
                .border_l_2()
                .border_color(t.chat.thinking_rule)
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(
                    TextView::new(&thinking.state)
                        .selectable(true)
                        .scrollable(false)
                        .style(design::markdown_style(cx)),
                ),
        );
    }
    block
}

fn render_assistant_message(
    msg: &ChatMessage,
    index: usize,
    streaming: bool,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let markdown = chat
        .markdown
        .get(&msg.id)
        .expect("assistant Markdown is synchronized before rendering");
    let copy_text = msg.text.clone();
    let answering = !msg.text.is_empty();
    let mut content = v_flex().flex_1().min_w_0().gap(t.space.small);
    if let Some(thinking) = &markdown.thinking {
        content = content.child(render_thinking(
            msg,
            thinking,
            index,
            streaming && !answering,
            chat,
            cx,
        ));
    } else if streaming && !answering && msg.widgets.is_empty() {
        content = content.child(waiting_pill("Sovra is thinking…", chat, cx));
    }
    if answering {
        content = content.child(
            div()
                .w_full()
                .min_w_0()
                .sovra_text(TextRole::Reading, cx)
                .child(
                    TextView::new(&markdown.state)
                        .selectable(true)
                        .scrollable(false)
                        .style(design::markdown_style(cx))
                        .code_block_actions(|code, _, cx| {
                            let text = code.code().to_string();
                            design::button("copy-code", "Copy", cx)
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                })
                        }),
                ),
        );
    }
    content = content.children(
        msg.widgets
            .iter()
            .enumerate()
            .map(|(widget_index, widget)| {
                render_agui_widget(widget, index * 10 + widget_index, cx)
            }),
    );
    if streaming && answering {
        content = content.child(live_line(chat, cx));
    }
    if !streaming && (answering || msg.model.is_some()) {
        content = content.child(
            h_flex()
                .id(("message-actions", index))
                .w_full()
                .flex_wrap()
                .items_center()
                .gap(t.space.compact)
                .opacity(t.chat.action_idle_opacity)
                .hover(|style| style.opacity(1.))
                .children(answering.then(|| {
                    design::button(("copy-message", index), "", cx)
                        .ghost()
                        .xsmall()
                        .child(
                            Icon::new(IconName::Copy)
                                .xsmall()
                                .text_color(t.text_secondary),
                        )
                        .px(t.space.compact)
                        .accessibility_label("Copy response")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        })
                }))
                .children(
                    msg.model
                        .as_ref()
                        .map(|model| render_model_pill(model, index, cx)),
                ),
        );
    }
    h_flex()
        .id(("assistant-msg", index))
        .w_full()
        .items_start()
        .gap(t.chat.row_gap)
        .child(assistant_avatar(streaming, cx))
        .child(content)
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

    #[test]
    fn elapsed_label_matches_sovrasrx_format() {
        assert_eq!(elapsed_label(0), "0s");
        assert_eq!(elapsed_label(42_900), "42s");
        assert_eq!(elapsed_label(180_000), "3m");
        assert_eq!(elapsed_label(192_000), "3m 12s");
    }

    #[gpui_kit::test]
    fn reasoning_streams_open_then_folds_when_the_answer_starts(cx: &mut TestAppContext) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Why?", window, cx));
            window.render_frame(cx);
            assert!(
                window.find("chat-response-status").visible(),
                "a waiting pill shows before any output"
            );
        })
        .unwrap();
        events
            .send_blocking(Event::AgentThinkingChunk {
                session_id: session.clone(),
                text: "Weighing **two** options.".into(),
                elapsed_ms: 2_400,
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(("thinking-body", 1usize)).visible());
        })
        .unwrap();
        for event in [
            Event::AgentChunk {
                session_id: session.clone(),
                text: "Option one.".into(),
            },
            Event::AgentCompleted {
                session_id: session,
            },
        ] {
            events.send_blocking(event).unwrap();
        }
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(("thinking-body", 1usize)).is_none(),
                "reasoning folds once the answer arrives"
            );
            assert_eq!(
                window.find(("thinking-toggle", 1usize)).label(),
                Some("Show reasoning")
            );
            window.click(("thinking-toggle", 1usize), cx);
            window.render_frame(cx);
            assert!(window.find(("thinking-body", 1usize)).visible());
            let message = &shell.read(cx).state.messages[1];
            assert_eq!(message.thinking, "Weighing **two** options.");
            assert_eq!(message.thinking_ms, Some(2_400));
            assert!(window.try_find("chat-response-status").is_none());
        })
        .unwrap();
    }

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
