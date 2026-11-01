//! The standard chat: bare assistant replies beside the Sovra
//! avatar, neutral user bubbles, a "Thinking…" pill with an elapsed timer and a
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
    shimmer::ShimmerText,
    text::{TextView, TextViewState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, FollowMode, InteractiveElement as _,
    IntoElement, ListAlignment, ListState, ParentElement as _, Rems, Render, Role, ScrollHandle,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription,
    TestSupportExt as _, WeakEntity, Window, div, list, px, relative,
};
use sovra_core::{AgUiWidget, ChatMessage, MessageRole, ResponseModel, WorkspaceState};

use crate::agui_widgets::render_agui_widget;
use crate::composer::render_composer;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::markdown_segments::Segmenter;
use crate::shell::{Shell, logo};

/// One rendered slice of a reply (see `markdown_segments`).
struct Segment {
    /// Where this segment begins in `ChatMessage::text`.
    start: usize,
    /// End of the bytes already handed to `state`. Replies only grow while
    /// streaming, so new bytes are appended without copying or comparing old ones.
    fed: usize,
    state: Entity<TextViewState>,
    /// Continues a paragraph split between sentences: drawn without a gap.
    soft: bool,
    _changed: Subscription,
}

/// Streamed Markdown split into segments (see `markdown_segments`). Never empty
/// once fed; earlier segments are frozen and only the last one grows.
struct SegmentedText {
    segmenter: Segmenter,
    segments: Vec<Segment>,
}

struct MarkdownMessage {
    /// The message this state was built for; a different message at the same
    /// position rebuilds it.
    id: String,
    answer: SegmentedText,
    widgets: Vec<AgUiWidget>,
    model: Option<ResponseModel>,
    thinking: Option<ThinkingView>,
}

struct ThinkingView {
    text: SegmentedText,
    /// Keeps the capped reasoning panel pinned to its newest line while it streams.
    scroll: ScrollHandle,
}

/// One virtualized list row: a user message, one segment of a reply, or the
/// waiting row shown before the first output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Message { message: usize, segment: usize },
    Waiting,
    Outbound { index: usize },
}

pub(crate) struct ChatPresentation {
    pub(crate) list: ListState,
    /// Keyed by position: a conversation only grows, and saved history can hold
    /// duplicate message ids from older builds.
    markdown: HashMap<usize, MarkdownMessage>,
    rows: Vec<Row>,
    /// Reasoning disclosures the reader opened (or closed while streaming).
    pub(crate) thinking_toggled: HashSet<usize>,
    session_id: String,
    /// When the current turn started, for the "Thinking… 12s" timer.
    pub(crate) turn_started: Option<Instant>,
    was_generating: bool,
    pub(crate) composer_focused: bool,
}

fn markdown_view(
    text: &str,
    message: usize,
    segment: usize,
    cx: &mut Context<Shell>,
) -> (Entity<TextViewState>, Subscription) {
    let view = cx.new(|cx| TextViewState::markdown(text, cx));
    // Parsing runs on a background executor and can finish after the network chunk.
    // Let ListState preserve the reading position or follow the newly measured tail.
    let changed = cx.observe(&view, move |shell, _, cx| {
        if let Some(row) = shell.chat.row_index(message, segment) {
            shell.chat.list.remeasure_items(row..row + 1);
        }
        shell.refresh_transcript(cx);
    });
    (view, changed)
}

fn new_segment(start: usize, message: usize, segment: usize, cx: &mut Context<Shell>) -> Segment {
    let (state, changed) = markdown_view("", message, segment, cx);
    Segment {
        start,
        fed: start,
        state,
        soft: false,
        _changed: changed,
    }
}

impl SegmentedText {
    fn new() -> Self {
        Self {
            segmenter: Segmenter::default(),
            segments: Vec::new(),
        }
    }

    /// Appends new bytes, opening a new segment at each safe split. `row_segment`
    /// maps a segment to the list row its parse results must re-measure.
    /// Returns whether anything changed.
    fn feed(
        &mut self,
        text: &str,
        message: usize,
        row_segment: fn(usize) -> usize,
        cx: &mut Context<Shell>,
    ) -> bool {
        let fed = self.segments.last().map_or(0, |segment| segment.fed);
        if !self.segments.is_empty() && text.len() == fed {
            return false;
        }
        if text.len() < fed || !text.is_char_boundary(fed) {
            // Replaced rather than streamed: start over.
            self.segmenter = Segmenter::default();
            self.segments.clear();
        }
        let was_whole = self.segmenter.keep_whole();
        self.segmenter.advance(text);
        if self.segmenter.keep_whole() && !was_whole && self.segments.len() > 1 {
            // A link definition appeared: links may point across segments, so
            // render this text as one block from here on.
            self.segments.clear();
        }
        if self.segments.is_empty() {
            self.segments
                .push(new_segment(0, message, row_segment(0), cx));
        }
        while self.segments.len() <= self.segmenter.boundaries().len() {
            let boundary = self.segmenter.boundaries()[self.segments.len() - 1];
            if boundary < self.segments.last().map_or(0, |segment| segment.fed) {
                // Defensive: never slice backwards. Render the text as one block.
                tracing::warn!(boundary, "segment split behind rendered text");
                self.segmenter = Segmenter::whole();
                self.segments.clear();
                self.segments
                    .push(new_segment(0, message, row_segment(0), cx));
                break;
            }
            let last = self.segments.last_mut().expect("segments are never empty");
            push(&last.state, &text[last.fed..boundary], cx);
            last.fed = boundary;
            let mut next = new_segment(boundary, message, row_segment(self.segments.len()), cx);
            next.soft = self.segmenter.is_soft(self.segments.len() - 1);
            self.segments.push(next);
        }
        let last = self.segments.last_mut().expect("segments are never empty");
        push(&last.state, &text[last.fed..], cx);
        last.fed = text.len();
        true
    }
}

/// Answer segments each have their own row.
fn own_row(segment: usize) -> usize {
    segment
}

/// Reasoning lives in the reply's first row.
fn first_row(_: usize) -> usize {
    0
}

impl ThinkingView {
    fn new() -> Self {
        Self {
            text: SegmentedText::new(),
            scroll: ScrollHandle::new(),
        }
    }
}

impl MarkdownMessage {
    fn new(message: &ChatMessage, index: usize, cx: &mut Context<Shell>) -> Self {
        let mut markdown = Self {
            id: message.id.clone(),
            answer: SegmentedText::new(),
            widgets: message.widgets.clone(),
            model: message.model.clone(),
            thinking: None,
        };
        markdown.feed(message, index, cx);
        markdown
    }

    /// Feeds new answer and reasoning bytes; returns whether either changed.
    fn feed(&mut self, message: &ChatMessage, index: usize, cx: &mut Context<Shell>) -> bool {
        let mut changed = self.answer.feed(&message.text, index, own_row, cx);
        if !message.thinking.is_empty() {
            changed |= self
                .thinking
                .get_or_insert_with(ThinkingView::new)
                .text
                .feed(&message.thinking, index, first_row, cx);
        }
        changed
    }
}

fn push(view: &Entity<TextViewState>, delta: &str, cx: &mut Context<Shell>) {
    if !delta.is_empty() {
        view.update(cx, |view, cx| view.push_str(delta, cx));
    }
}

impl ChatPresentation {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        // Rows carry their own spacing so segments of one reply sit flush.
        let list = ListState::new(0, ListAlignment::Top, px(0.));
        list.set_follow_mode(FollowMode::Tail);
        list.set_scroll_handler(cx.listener(|shell, _, _, cx| shell.refresh(cx)));
        Self {
            list,
            markdown: HashMap::new(),
            rows: Vec::new(),
            thinking_toggled: HashSet::new(),
            session_id: String::new(),
            turn_started: None,
            was_generating: false,
            composer_focused: false,
        }
    }

    fn row_index(&self, message: usize, segment: usize) -> Option<usize> {
        self.rows
            .iter()
            .rposition(|row| *row == Row::Message { message, segment })
    }

    fn remeasure(&self, message: usize, segment: usize) {
        if let Some(row) = self.row_index(message, segment) {
            self.list.remeasure_items(row..row + 1);
        }
    }

    pub(crate) fn sync(&mut self, state: &WorkspaceState, cx: &mut Context<Shell>) {
        if self.session_id != state.session_id {
            self.session_id.clone_from(&state.session_id);
            self.markdown.clear();
            self.rows.clear();
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
        let mut changed = Vec::new();
        for (index, message) in state.messages.iter().enumerate() {
            if message.role != MessageRole::Assistant {
                continue;
            }
            if let Some(markdown) = self
                .markdown
                .get_mut(&index)
                .filter(|markdown| markdown.id == message.id)
            {
                let dirty = markdown.feed(message, index, cx);
                if dirty || markdown.widgets != message.widgets || markdown.model != message.model {
                    markdown.model.clone_from(&message.model);
                    markdown.widgets.clone_from(&message.widgets);
                    changed.push((index, markdown.answer.segments.len()));
                }
            } else {
                let markdown = MarkdownMessage::new(message, index, cx);
                changed.push((index, markdown.answer.segments.len()));
                self.markdown.insert(index, markdown);
            }
        }
        let mut rows = Vec::with_capacity(self.rows.len() + 1);
        for (index, message) in state.messages.iter().enumerate() {
            let segments = match message.role {
                MessageRole::User => 1,
                MessageRole::Assistant => self
                    .markdown
                    .get(&index)
                    .map_or(1, |markdown| markdown.answer.segments.len()),
            };
            rows.extend((0..segments).map(|segment| Row::Message {
                message: index,
                segment,
            }));
        }
        rows.extend(
            state
                .connections
                .outbound_messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.session_id == state.session_id)
                .map(|(index, _)| Row::Outbound { index }),
        );
        if awaiting_first_output(state) {
            rows.push(Row::Waiting);
        }
        if rows != self.rows {
            let kept = self
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(old, new)| old == new)
                .count();
            self.list.splice(kept..self.rows.len(), rows.len() - kept);
            // The previous last row may lose its live line or actions.
            self.list
                .remeasure_items(kept.saturating_sub(1)..rows.len());
            self.rows = rows;
        }
        // Approval and receipt changes can change a row's height without changing its id.
        for (index, row) in self.rows.iter().enumerate() {
            if matches!(row, Row::Outbound { .. }) {
                self.list.remeasure_items(index..index + 1);
            }
        }
        // Only the first row (thinking, widgets above) and the growing last segment
        // of a changed reply need measuring again — not every segment.
        for (message, segments) in changed {
            self.remeasure(message, 0);
            self.remeasure(message, segments.saturating_sub(1));
        }
    }

    pub(crate) fn follow_latest(&self) {
        self.list.set_follow_mode(FollowMode::Tail);
    }

    /// All rendered answer text of a reply, across its segments.
    #[cfg(test)]
    fn rendered_answer(&self, message: usize, cx: &gpui_kit::App) -> String {
        self.markdown[&message]
            .answer
            .segments
            .iter()
            .map(|segment| segment.state.read(cx).rendered_text().as_str().to_owned())
            .collect()
    }

    #[cfg(test)]
    fn segment_count(&self, message: usize) -> usize {
        self.markdown[&message].answer.segments.len()
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

/// Fills narrow panes and caps prose at a readable measure on wide ones; the
/// cap is in rems, so it tracks the interface scale without breakpoints.
fn column(cx: &Context<Shell>) -> gpui_kit::Div {
    div()
        .w_full()
        .min_w_0()
        .max_w(design::tokens(cx).chat.column_width)
}

/// The conversation list, rendered by its own entity so `Shell` can embed it with
/// [`Entity::cached`]: GPUI then reuses its last layout and paint unless
/// `Shell::refresh_transcript` notifies it, instead of re-laying-out every visible
/// message whenever the composer cursor blinks or the sidebar changes.
pub(crate) struct Transcript {
    shell: WeakEntity<Shell>,
    #[cfg(test)]
    renders: usize,
}

impl Transcript {
    pub(crate) fn new(shell: WeakEntity<Shell>) -> Self {
        Self {
            shell,
            #[cfg(test)]
            renders: 0,
        }
    }
}

impl Render for Transcript {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        shell.update(cx, |shell, cx| {
            let Shell { chat, state, .. } = shell;
            chat.sync(state, cx);
            render_transcript(&shell.chat, cx).into_any_element()
        })
    }
}

fn render_transcript(chat: &ChatPresentation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    div()
        .id("chat-scroll-area")
        .role(Role::Log)
        .aria_label("Conversation")
        .test_support()
        .relative()
        .size_full()
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
                            shell.refresh(cx);
                        })),
                )
        }))
}

pub(crate) fn render_chat_column(
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let Shell {
        state, transcript, ..
    } = shell;
    let t = design::tokens(cx);
    let welcome = !state
        .connections
        .outbound_messages
        .iter()
        .any(|m| m.session_id == state.session_id)
        && state.messages.is_empty()
        && shell.composer_input.read(cx).value().is_empty()
        && !state.is_generating
        && state
            .file_drafts
            .get(&state.session_id)
            .is_none_or(|draft| draft.files.is_empty());
    let mut area = StyleRefinement::default().w_full().min_h_0();
    // The empty list yields its height to the welcome block.
    area.flex_grow = Some(if welcome { 0. } else { 1. });
    area.flex_basis = Some(px(0.).into());
    v_flex()
        .id("sovra-chat-main")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .px(t.space.large)
        .pb(t.space.medium)
        .overflow_hidden()
        .children(welcome.then(|| render_welcome(state, cx)))
        .child(transcript.clone().cached(area))
        .children(crate::skill_menu::render(shell, cx).map(|menu| {
            // Keep a real hit target. A flexible chat column would otherwise
            // shrink this menu to zero height and swallow its clicks.
            h_flex()
                .w_full()
                .flex_shrink_0()
                .max_h(t.layout.command_menu_height)
                .justify_center()
                .child(column(cx).child(menu))
        }))
        .child(
            h_flex()
                .w_full()
                .justify_center()
                .child(column(cx).child(render_composer(shell, cx))),
        )
}

/// Starters for people who don't know what to ask an AI yet.
const STARTERS: [(&str, &str); 4] = [
    (
        "Understand something",
        "Explain this to me simply, like I'm new to it: ",
    ),
    (
        "Draft an email",
        "Help me write a short, friendly email about ",
    ),
    (
        "Plan my week",
        "Help me plan my week. Here's what I need to get done: ",
    ),
    (
        "Summarize a document",
        "Summarize this in a few bullet points:\n\n",
    ),
];

/// `SovraSRX`'s welcome: greeting, one-line subtitle and starter chips that fill the
/// composer (they never send on their own).
fn render_welcome(state: &WorkspaceState, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let first_name = state
        .user
        .full_name
        .split_whitespace()
        .next()
        .unwrap_or("there")
        .to_owned();
    // Scrolls inside its pane in short windows, so the composer always fits.
    h_flex()
        .id("chat-welcome")
        .w_full()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .test_support()
        .justify_center()
        .child(
            v_flex()
                .w_full()
                .my_auto()
                .max_w(t.chat.welcome_width)
                .gap(t.space.medium)
                .child(logo("chat-welcome-logo", t.layout.logo_hero))
                .child(
                    div()
                        .sovra_text(TextRole::Display, cx)
                        .child(format!("What’s on your mind, {first_name}?")),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child("A question, an idea, a little help getting started."),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.compact)
                        .pt(t.space.small)
                        .children(STARTERS.iter().enumerate().map(|(index, (label, draft))| {
                            design::button(("chat-starter", index), *label, cx)
                                .ghost()
                                .small()
                                .min_h(t.chat.chip_height)
                                .px(t.space.small)
                                .rounded(t.radius.input)
                                .border_1()
                                .border_color(t.chat.chip_border)
                                .on_click(cx.listener(move |shell, _, window, cx| {
                                    shell.fill_composer_draft(draft, window, cx);
                                }))
                        })),
                ),
        )
}

fn render_row(
    state: &WorkspaceState,
    chat: &ChatPresentation,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    // Messages are 1.375rem apart; segments of one reply continue at paragraph spacing.
    let t = design::tokens(cx);
    let (message_gap, paragraph_gap) = (t.chat.message_padding, t.space.medium);
    let (row, top, bottom) = match chat.rows.get(index).copied() {
        Some(Row::Message { message, segment }) => match state.messages.get(message) {
            Some(msg) if msg.role == MessageRole::User => (
                render_user_message(state, msg, message, cx).into_any_element(),
                message_gap,
                message_gap,
            ),
            Some(msg) => {
                let streaming = state.is_generating && message + 1 == state.messages.len();
                let answer = chat.markdown.get(&message).map(|markdown| &markdown.answer);
                let segments = answer.map_or(1, |answer| answer.segments.len());
                let soft = answer
                    .and_then(|answer| answer.segments.get(segment))
                    .is_some_and(|part| part.soft);
                (
                    render_assistant_segment(state, msg, message, segment, streaming, chat, cx)
                        .into_any_element(),
                    // A paragraph continued between sentences gets no gap.
                    match (segment, soft) {
                        (0, _) => message_gap,
                        (_, true) => Rems::ZERO,
                        (_, false) => paragraph_gap,
                    },
                    if segment + 1 == segments {
                        message_gap
                    } else {
                        Rems::ZERO
                    },
                )
            }
            None => (div().into_any_element(), Rems::ZERO, Rems::ZERO),
        },
        Some(Row::Outbound { index }) => (
            crate::message_review::render(&state.connections.outbound_messages[index], index, cx)
                .into_any_element(),
            message_gap,
            message_gap,
        ),
        Some(Row::Waiting) | None => (
            h_flex()
                .w_full()
                .items_start()
                .gap(design::tokens(cx).chat.row_gap)
                .child(assistant_avatar(true, cx))
                .child(waiting_pill("Sovra is thinking…", chat, cx))
                .into_any_element(),
            message_gap,
            message_gap,
        ),
    };
    h_flex()
        .w_full()
        .justify_center()
        .child(column(cx).pt(top).pb(bottom).child(row))
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
                .px(t.space.medium)
                .py(t.space.small)
                .bg(chat.user_bubble)
                .rounded_tl(chat.user_bubble_radius)
                .rounded_tr(chat.user_bubble_radius)
                .rounded_bl(chat.user_bubble_radius)
                .rounded_br(chat.user_bubble_tail_radius)
                .shadow(vec![chat.user_bubble_shadow.clone()])
                .sovra_text(TextRole::Body, cx)
                .text_color(chat.user_bubble_foreground)
                .child(
                    v_flex()
                        .gap(t.space.small)
                        .children(msg.attachments.iter().map(|file| {
                            v_flex()
                                .gap(t.space.compact)
                                .child(crate::widgets::attachments::file_card(file, cx))
                                .children(file.note.as_ref().map(|note| {
                                    div()
                                        .sovra_text(TextRole::Caption, cx)
                                        .text_color(t.text_secondary)
                                        .child(note.clone())
                                }))
                        })),
                )
                .child(msg.text.clone()),
        )
        .child(
            h_flex()
                .size(chat.avatar)
                .mt(t.space.compact)
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
/// A team member's face: their emoji in a soft circle, sized like the Sovra avatar.
pub(crate) fn member_avatar<V>(
    icon: &str,
    size: Rems,
    cx: &Context<V>,
) -> impl IntoElement + use<V> {
    let t = design::tokens(cx);
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.subtle)
        .border_1()
        .border_color(t.material_border)
        .text_size(size * 0.55)
        .child(icon.to_owned())
}

fn assistant_avatar(active: bool, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let chat = &t.chat;
    h_flex()
        .relative()
        .size(chat.avatar)
        .mt(t.space.compact)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.subtle)
        .child(logo("assistant-avatar", chat.avatar - t.space.small))
        .children(active.then(|| {
            div()
                .absolute()
                .top(-chat.avatar_ring_offset)
                .left(-chat.avatar_ring_offset)
                .size(chat.avatar + chat.avatar_ring_offset + chat.avatar_ring_offset)
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
        .mt(t.space.compact)
        .gap(t.space.compact)
        .items_center()
        .px(t.space.small)
        .py(t.space.compact)
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
        .child(div().size(t.space.small).rounded_full().bg(t.accent.violet))
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
    let open = live != chat.thinking_toggled.contains(&index);
    let label = if live {
        format!("Thinking… {}", turn_elapsed(chat))
    } else {
        msg.thinking_ms.map_or_else(
            || "Thought for a moment".to_owned(),
            |ms| format!("Thought for {}", elapsed_label(ms.max(1000))),
        )
    };
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
                        if !shell.chat.thinking_toggled.remove(&index) {
                            shell.chat.thinking_toggled.insert(index);
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
                // While reasoning streams, the pinned panel shows only its end, so
                // only the last segments are built; opened later, all of them are.
                .children(
                    thinking
                        .text
                        .segments
                        .iter()
                        .skip(if live {
                            thinking.text.segments.len().saturating_sub(2)
                        } else {
                            0
                        })
                        // No explicit style: the view inherits this muted text colour.
                        .map(|part| {
                            div()
                                .pt(if part.start > 0 && !part.soft {
                                    t.space.small + t.space.compact
                                } else {
                                    Rems::ZERO
                                })
                                .child(
                                    TextView::new(&part.state)
                                        .selectable(true)
                                        .scrollable(false),
                                )
                        }),
                ),
        );
    }
    block
}

fn render_assistant_segment(
    state: &WorkspaceState,
    msg: &ChatMessage,
    index: usize,
    segment: usize,
    streaming: bool,
    chat: &ChatPresentation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let markdown = chat
        .markdown
        .get(&index)
        .expect("assistant Markdown is synchronized before rendering");
    let first = segment == 0;
    let last = segment + 1 == markdown.answer.segments.len();
    let answering = !msg.text.is_empty();
    let mut content = v_flex().flex_1().min_w_0().gap(t.space.small);
    if first && let Some(speaker) = &msg.speaker {
        content = content.child(
            div()
                .id(("team-speaker", index))
                .test_support()
                .mt(t.space.compact)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!("{} · {}", speaker.name, speaker.role)),
        );
    }
    if first {
        if let Some(thinking) = &markdown.thinking {
            content = content.child(render_thinking(
                msg,
                thinking,
                index,
                streaming && !answering,
                chat,
                cx,
            ));
        } else if streaming && !answering && msg.widgets.is_empty() && msg.speaker.is_none() {
            content = content.child(waiting_pill("Sovra is thinking…", chat, cx));
        }
    }
    if let Some(part) = markdown
        .answer
        .segments
        .get(segment)
        .filter(|part| part.fed > part.start)
    {
        let source = sovra_core::artifacts::ArtifactSource {
            session_id: state.session_id.clone(),
            message_id: msg.id.clone(),
            author: msg
                .speaker
                .as_ref()
                .map_or_else(|| "Sovra".to_owned(), |speaker| speaker.name.clone()),
        };
        let view = TextView::new(&part.state)
            .selectable(true)
            .scrollable(false)
            .style(design::markdown_style(cx))
            .code_block_actions(move |code, _, cx| {
                let text = code.code().to_string();
                let language = code.lang().map(|lang| lang.to_string()).unwrap_or_default();
                let save = sovra_core::artifacts::draft_from_piece(
                    &language,
                    &text,
                    source.clone(),
                    "Saved snippet",
                );
                h_flex()
                    .gap(design::tokens(cx).space.hair)
                    .child(
                        design::button("copy-code", "Copy", cx)
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            }),
                    )
                    .child(
                        design::button("save-code", "Save", cx)
                            .ghost()
                            .xsmall()
                            .icon(IconName::Bookmark)
                            .accessibility_label("Save this to Artifacts")
                            .on_click(move |_, _, cx| {
                                let _ = cx.global::<crate::RuntimeClient>().0.send(
                                    sovra_core::Command::Artifacts(
                                        sovra_core::artifacts::ArtifactCommand::Save(save.clone()),
                                    ),
                                );
                            }),
                    )
            });
        let body = div()
            .w_full()
            .min_w_0()
            .sovra_text(TextRole::Reading, cx)
            .child(view);
        content = content.child(if first {
            body.id(("assistant-answer", index))
                .test_support()
                .into_any_element()
        } else {
            body.id(("assistant-segment", index * 100_000 + segment))
                .test_support()
                .into_any_element()
        });
    }
    if last {
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
            let citations = sovra_core::memory_citations(&msg.text);
            if !citations.is_empty() {
                content = content.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .flex_wrap()
                        .gap(t.space.compact)
                        .children(citations.into_iter().enumerate().map(|(number, id)| {
                            design::button(
                                (
                                    gpui_kit::SharedString::from(format!(
                                        "memory-citation-{index}"
                                    )),
                                    number,
                                ),
                                format!("Saved source {}", number + 1),
                                cx,
                            )
                            .ghost()
                            .xsmall()
                            .accessibility_label(format!(
                                "Open saved source {}: memory:{}",
                                number + 1,
                                id.0
                            ))
                            .tooltip(format!("memory:{}", id.0))
                            .on_click(cx.listener(
                                move |shell, _, _, cx| shell.open_memory_source(id.clone(), cx),
                            ))
                        })),
                );
            }
            let copy_text = msg.text.clone();
            let saved = state
                .artifacts
                .artifacts
                .iter()
                .any(|artifact| artifact.source.message_id == msg.id);
            let save_text = msg.text.clone();
            let source = sovra_core::artifacts::ArtifactSource {
                session_id: state.session_id.clone(),
                message_id: msg.id.clone(),
                author: msg
                    .speaker
                    .as_ref()
                    .map_or_else(|| "Sovra".to_owned(), |speaker| speaker.name.clone()),
            };
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
                    .children(answering.then(|| {
                        design::button(
                            ("save-message", index),
                            if saved { "Saved" } else { "Save" },
                            cx,
                        )
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(if saved {
                            IconName::Check
                        } else {
                            IconName::Bookmark
                        }))
                        .px(t.space.compact)
                        .accessibility_label(if saved {
                            "Saved to Artifacts"
                        } else {
                            "Save this answer to Artifacts"
                        })
                        .tooltip("Keep this in Artifacts")
                        .on_click(move |_, _, cx| {
                            let draft = sovra_core::artifacts::draft_from_reply(
                                &save_text,
                                source.clone(),
                                "Saved answer",
                            );
                            let _ = cx.global::<crate::RuntimeClient>().0.send(
                                sovra_core::Command::Artifacts(
                                    sovra_core::artifacts::ArtifactCommand::Save(draft),
                                ),
                            );
                        })
                    }))
                    .children(
                        msg.model
                            .as_ref()
                            .map(|model| render_model_pill(model, index, cx)),
                    ),
            );
        }
    }
    let gutter = if first && let Some(speaker) = &msg.speaker {
        div()
            .mt(t.space.compact)
            .child(member_avatar(&speaker.icon, t.chat.avatar, cx))
            .into_any_element()
    } else if first {
        assistant_avatar(streaming, cx).into_any_element()
    } else {
        // Continuation rows keep the text aligned under the first row's text.
        div().w(t.chat.avatar).flex_shrink_0().into_any_element()
    };
    h_flex()
        .id(("assistant-msg", index * 100_000 + segment))
        .w_full()
        .items_start()
        .gap(t.chat.row_gap)
        .child(gutter)
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

    #[gpui_kit::test]
    fn empty_chat_welcomes_and_starters_only_fill_the_composer(cx: &mut TestAppContext) {
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
            window.render_frame(cx);
            assert!(window.find("chat-welcome").visible());
            window.click(("chat-starter", 1usize), cx);
            window.render_frame(cx);
            assert!(
                shell.read(cx).state.messages.is_empty(),
                "starters never send"
            );
            assert_ne!(window.find("composer-send-btn").disabled(), Some(true));
            window.click("composer-send-btn", cx);
            window.render_frame(cx);
            assert!(window.try_find("chat-welcome").is_none());
        })
        .unwrap();
        assert!(std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, sovra_core::Command::RunPrompt { prompt, .. } if prompt.starts_with("Help me write"))));
    }

    #[gpui_kit::test]
    fn sending_in_a_restored_conversation_keeps_earlier_answers_visible(cx: &mut TestAppContext) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let mut thread = sovra_core::ChatThread::new("restored", sovra_core::Timestamp(1));
        thread
            .messages
            .push(ChatMessage::user("msg-user-1", "Hi", "Earlier"));
        let mut reply = ChatMessage::assistant(
            "msg-assistant-2",
            "An earlier **answer** that must stay visible.",
            "Earlier",
            Vec::new(),
        );
        reply.thinking = "Earlier reasoning.".into();
        reply.thinking_ms = Some(5_000);
        thread.messages.push(reply);
        // Older builds could save two replies under one id.
        thread
            .messages
            .push(ChatMessage::user("msg-user-3", "And?", "Earlier"));
        thread.messages.push(ChatMessage::assistant(
            "msg-assistant-2",
            "A different second answer.",
            "Earlier",
            Vec::new(),
        ));
        events
            .send_blocking(Event::ChatHistoryLoaded {
                threads: vec![thread],
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_thread("restored", cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let before = cx
            .update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window
                    .find(("assistant-answer", 1usize))
                    .bounds()
                    .size
                    .height
            })
            .unwrap();
        assert!(before > px(10.), "restored answer renders: {before:?}");
        shell.read_with(cx, |shell, cx| {
            let text = |index: usize| shell.chat.rendered_answer(index, cx);
            assert!(text(1).contains("earlier answer"));
            assert!(text(3).contains("different second answer"));
        });
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Again", window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        events
            .send_blocking(Event::AgentThinkingChunk {
                session_id: "restored".into(),
                text: "New reasoning".into(),
                elapsed_ms: 300,
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let after = window
                .find(("assistant-answer", 1usize))
                .bounds()
                .size
                .height;
            assert_eq!(after, before, "earlier answer collapsed after sending");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn typing_does_not_rerender_the_conversation_but_new_output_does(cx: &mut TestAppContext) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        let renders = |cx: &mut TestAppContext| {
            shell.read_with(cx, |shell, cx| shell.transcript.read(cx).renders)
        };
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Hi", window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        events
            .send_blocking(Event::AgentChunk {
                session_id: session.clone(),
                text: "Hello there.".into(),
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        // `render_frame` forces a full refresh, which bypasses view caches; real
        // frames (`draw`) only redraw what was notified.
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let settled = renders(cx);
        cx.update_window(window.into(), |_, window, cx| {
            for draft in ["t", "ty", "typing a draft"] {
                let input = shell.read(cx).composer_input.clone();
                input.update(cx, |input, cx| input.set_value(draft, window, cx));
                window.draw(cx).clear(cx);
            }
        })
        .unwrap();
        assert_eq!(
            renders(cx),
            settled,
            "composer typing reused the cached conversation"
        );
        events
            .send_blocking(Event::AgentChunk {
                session_id: session,
                text: " More.".into(),
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert!(
            renders(cx) > settled,
            "new output re-renders the conversation"
        );
    }

    #[gpui_kit::test]
    fn selection_spans_the_segments_of_a_long_reply(cx: &mut TestAppContext) {
        use gpui_kit::{Modifiers, MouseButton, base::TextSelection};

        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Hi", window, cx));
        });
        let filler = "words ".repeat(270);
        for event in [
            Event::AgentChunk {
                session_id: session.clone(),
                text: format!("Alphamarker {filler}\n\nOmegamarker closes the reply.\n"),
            },
            Event::AgentCompleted {
                session_id: session,
            },
        ] {
            events.send_blocking(event).unwrap();
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let draw = |cx: &mut gpui_kit::VisualTestContext| {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        };
        draw(cx);
        let (from, to) = cx.update(|window, cx| {
            window.render_frame(cx);
            assert_eq!(shell.read(cx).chat.segment_count(1), 2);
            let first = window.find(("assistant-answer", 1usize)).bounds();
            let second = window.find(("assistant-segment", 100_001usize)).bounds();
            (
                first.origin + point(px(1.), px(8.)),
                second.bottom_right() - point(px(4.), px(8.)),
            )
        });
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
        draw(cx);
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        draw(cx);
        let selected = cx.update(TextSelection::selected_text);
        assert!(selected.contains("Alphamarker"), "got: {selected:?}");
        assert!(selected.contains("Omegamarker"), "got: {selected:?}");
    }

    #[gpui_kit::test]
    fn code_heavy_replies_stream_in_mid_line_chunks_without_slicing_backwards(
        cx: &mut TestAppContext,
    ) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Code", window, cx));
        })
        .unwrap();
        let block = "```rust\nfn step(input: &str) -> usize {\n    input.len()\n}\n```\n\nExplanation words. ";
        let reply = block.repeat(40);
        let bytes = reply.as_bytes();
        for chunk in bytes.chunks(7) {
            events
                .send_blocking(Event::AgentChunk {
                    session_id: session.clone(),
                    text: String::from_utf8(chunk.to_vec()).unwrap(),
                })
                .unwrap();
            // A frame between chunks, as in the app, so partial lines get rendered.
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                let _ = window.draw(cx);
            })
            .unwrap();
        }
        shell.read_with(cx, |shell, cx| {
            assert!(shell.chat.segment_count(1) > 1);
            let rendered = shell.chat.rendered_answer(1, cx);
            assert_eq!(rendered.matches("input.len()").count(), 40);
        });
    }

    #[gpui_kit::test]
    fn long_reasoning_is_segmented_like_answers(cx: &mut TestAppContext) {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1100.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let session = shell.read_with(cx, |shell, _| shell.state.session_id.clone());
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.send_prompt_text("Why?", window, cx));
        })
        .unwrap();
        let reasoning = format!("{}\n\n", "considering options ".repeat(30)).repeat(12);
        events
            .send_blocking(Event::AgentThinkingChunk {
                session_id: session,
                text: reasoning.clone(),
                elapsed_ms: 900,
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(("thinking-body", 1usize)).visible());
            let chat = &shell.read(cx).chat;
            let thinking = chat.markdown[&1].thinking.as_ref().unwrap();
            assert!(thinking.text.segments.len() > 1);
            let fed: usize = thinking
                .text
                .segments
                .iter()
                .map(|part| part.fed - part.start)
                .sum();
            assert_eq!(
                fed,
                reasoning.len(),
                "every reasoning byte is rendered once"
            );
        })
        .unwrap();
    }

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
            let view = cx.new(|cx| Shell::new(0, window, cx));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
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
            let view = cx.new(|cx| Shell::new(0, window, cx));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
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
            let view = cx.new(|cx| Shell::new(0, window, cx));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = shell.read(cx);
            assert!(view.chat.list.is_following_tail());
            let last = view.state.messages.len() - 1;
            assert!(
                view.chat.segment_count(last) >= 2,
                "a long reply renders as several virtualized rows"
            );
            let rendered = view.chat.rendered_answer(last, cx);
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
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
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
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
