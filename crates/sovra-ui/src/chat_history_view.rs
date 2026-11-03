//! Conversation history section rendered inside the unified left sidebar:
//! - Search bar (`Search chats…`) filtering by title, subtitle, and message body.
//! - Segmented `Chats | Scheduled` filter pills (`ChatFilterPills` radio group).
//! - Scrollable conversation list partitioned into `Pinned` and recency buckets.
//! - Per-row topic icon, title / inline rename input, subtitle preview, relative timestamp,
//!   and quick actions (`Pin`, `AI Title`, `Rename`, `Delete`).

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{
    ChatHistoryFilter, ChatThread, ConversationTopicIcon, NavTab, Timestamp, WorkspaceState,
    format_relative_time, group_threads, infer_topic_icon,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

/// Renders the unified conversation history section inside the single left sidebar.
pub(crate) fn render_chat_history_section(
    state: &WorkspaceState,
    history_search_input: &Entity<InputState>,
    rename_thread_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let reference_now = crate::shell::wall_time();
    let grouped = group_threads(
        &state.threads,
        state.history_filter,
        &state.history_search,
        reference_now,
    );

    v_flex()
        .id("chat-history-rail")
        .test_support()
        .w_full()
        .flex_1()
        .min_h(t.layout.list_row_min_height)
        .gap(t.space.compact)
        .child(render_rail_search(history_search_input, cx))
        .child(render_filter_pills(state.history_filter, cx))
        .child(render_thread_list(
            state,
            &grouped,
            reference_now,
            rename_thread_input,
            cx,
        ))
}

fn render_rail_search(
    history_search_input: &Entity<InputState>,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("chat-history-search-box")
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(t.space.compact)
        .px(t.space.small)
        .py(t.space.hair)
        .rounded(t.radius.input)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            Icon::new(IconName::Search)
                .xsmall()
                .text_color(t.text_secondary),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(history_search_input)
                    .id("chat-history-search")
                    .aria_label("Search chats")
                    .small()
                    .appearance(false)
                    .bordered(false),
            ),
        )
}

fn render_filter_pills(
    active_filter: ChatHistoryFilter,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (fg_active, fg_inactive) = (cx.theme().foreground, t.text_secondary);
    h_flex()
        .id("chat-filter-pills")
        .test_support()
        .role(Role::RadioGroup)
        .aria_label("Filter conversations")
        .w_full()
        .flex_shrink_0()
        .p(t.space.hair)
        .gap(t.space.hair)
        .rounded(t.radius.input)
        .bg(t.subtle)
        .children(ChatHistoryFilter::ALL.into_iter().map(|filter| {
            let is_active = filter == active_filter;
            let (id, tip) = match filter {
                ChatHistoryFilter::Chats => ("chat-filter-chats", "Your conversations with Sovra"),
                ChatHistoryFilter::Scheduled => (
                    "chat-filter-scheduled",
                    "Conversations started from Schedules",
                ),
            };
            design::button(id, filter.label(), cx)
                .tooltip(tip)
                .ghost()
                .selected(is_active)
                .xsmall()
                .min_h(t.layout.compact_control_height)
                .flex_1()
                .px(t.space.small)
                .rounded(t.radius.small)
                .text_color(if is_active { fg_active } else { fg_inactive })
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_history_filter(filter, cx)))
        }))
}

fn render_thread_list(
    state: &WorkspaceState,
    grouped: &sovra_core::GroupedHistory<'_>,
    reference_now: Timestamp,
    rename_thread_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let searching = !state.history_search.trim().is_empty();
    let t = design::tokens(cx);
    let mut list = v_flex()
        .id("chat-history-list")
        .test_support()
        .flex_1()
        .min_h_0()
        .w_full()
        .gap(t.space.hair)
        .overflow_y_scroll();

    let visible_non_draft: usize = grouped
        .pinned
        .iter()
        .chain(grouped.groups.iter().flat_map(|(_, items)| items.iter()))
        .filter(|th| !th.is_untouched_draft())
        .count();

    if grouped.is_empty()
        || (state.history_filter == ChatHistoryFilter::Chats && visible_non_draft == 0 && searching)
    {
        return list.child(render_empty_rail(state.history_filter, searching, cx));
    }

    let mut row_idx = 0usize;
    let sections = std::iter::once(("Pinned", grouped.pinned.as_slice())).chain(
        grouped
            .groups
            .iter()
            .map(|(bucket, items)| (bucket.label(), items.as_slice())),
    );
    for (label, items) in sections {
        if items.iter().all(|th| th.is_untouched_draft()) {
            continue;
        }
        list = list.child(render_section_label(label, cx));
        for thread in items.iter().filter(|th| !th.is_untouched_draft()) {
            let is_active = thread.id == state.session_id;
            let is_renaming = state.renaming_thread_id.as_deref() == Some(&thread.id);
            list = list.child(render_thread_row(
                thread,
                row_idx,
                is_active,
                is_renaming,
                reference_now,
                rename_thread_input,
                cx,
            ));
            row_idx += 1;
        }
    }

    if visible_non_draft == 0 && state.history_filter == ChatHistoryFilter::Chats && !searching {
        list = list.child(render_empty_chats_hint(cx));
    }
    list
}

fn render_section_label(label: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    div()
        .id(gpui_kit::SharedString::from(format!(
            "chat-history-section-{label}"
        )))
        .role(Role::Heading)
        .aria_label(label.to_owned())
        .test_support()
        .px(t.space.small)
        .pt(t.space.snug)
        .pb(t.space.hair)
        .sovra_text(TextRole::Caption, cx)
        .text_color(cx.theme().muted_foreground)
        .child(label.to_owned())
}

fn topic_icon_name(thread: &ChatThread) -> IconName {
    match infer_topic_icon(&thread.title, thread.kind) {
        ConversationTopicIcon::Chat => IconName::MessageCircle,
        ConversationTopicIcon::Code => IconName::Code,
        ConversationTopicIcon::Market => IconName::TrendingUp,
        ConversationTopicIcon::Housing => IconName::House,
        ConversationTopicIcon::Schedule => IconName::Calendar,
        ConversationTopicIcon::Search => IconName::Search,
        ConversationTopicIcon::Document => IconName::FileText,
    }
}

fn render_rename_row(
    thread_id: String,
    rename_thread_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .p(t.space.compact)
        .items_center()
        .gap(t.space.compact)
        .child(
            div().flex_1().min_w_0().child(
                Input::new(rename_thread_input)
                    .id("chat-rename-input")
                    .aria_label("Rename conversation")
                    .small(),
            ),
        )
        .child(
            design::button("chat-rename-save", "Save", cx)
                .small()
                .min_h(t.layout.compact_control_height)
                .px(t.space.compact)
                .on_click(
                    cx.listener(move |shell, _, _, cx| shell.commit_rename_thread(&thread_id, cx)),
                ),
        )
}

fn render_thread_row(
    thread: &ChatThread,
    idx: usize,
    is_active: bool,
    is_renaming: bool,
    reference_now: Timestamp,
    rename_thread_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (link, muted) = (cx.theme().link, cx.theme().muted_foreground);
    let icon_color = if is_active { link } else { t.text_secondary };
    let time_label = format_relative_time(thread.updated_at, reference_now);
    let surface = if is_active {
        Surface::NavActive
    } else {
        Surface::Chrome
    };
    let card = v_flex()
        .id(("chat-thread-card", idx))
        .w_full()
        .sovra_surface(surface, cx);

    if is_renaming {
        return card
            .child(render_rename_row(
                thread.id.clone(),
                rename_thread_input,
                cx,
            ))
            .children(is_active.then(|| render_thread_actions(thread, idx, cx)));
    }

    let thread_id = thread.id.clone();
    let title_row = h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap(t.space.compact)
        .child(
            div()
                .id(("chat-thread-title", idx))
                .aria_label(thread.title.clone())
                .test_support()
                .flex_1()
                .min_w_0()
                .truncate()
                .sovra_text(TextRole::Body, cx)
                .child(thread.title.clone()),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap(t.space.hair)
                .children(
                    thread
                        .pinned
                        .then(|| Icon::new(IconName::Pin).xsmall().text_color(link)),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(muted)
                        .child(time_label),
                ),
        );
    let preview = (!thread.last_message_preview.is_empty()).then(|| {
        div()
            .w_full()
            .truncate()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .child(thread.last_message_preview.clone())
    });

    card.child(
        design::button(("chat-thread-row", idx), "", cx)
            .accessibility_label(format!("Open conversation {}", thread.title))
            .ghost()
            .h_auto()
            .min_h(t.layout.control_height)
            .px(t.space.small)
            .py(t.space.compact)
            .w_full()
            .items_start()
            .gap(t.space.small)
            .child(
                div().pt(t.space.hair).flex_shrink_0().child(
                    Icon::new(topic_icon_name(thread))
                        .small()
                        .text_color(icon_color),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(t.space.hair)
                    .child(title_row)
                    .children(preview),
            )
            .on_click(cx.listener(move |shell, _, _, cx| shell.select_thread(&thread_id, cx))),
    )
    .children(is_active.then(|| render_thread_actions(thread, idx, cx)))
}

fn action_btn(
    id: impl Into<ElementId>,
    icon: IconName,
    tip: &'static str,
    label: &'static str,
    cx: &Context<Shell>,
) -> Button {
    let t = design::tokens(cx);
    design::button(id, "", cx)
        .icon(icon)
        .tooltip(tip)
        .accessibility_label(label)
        .ghost()
        .xsmall()
        .min_h(t.layout.compact_control_height)
        .px(t.space.compact)
}

fn render_thread_actions(
    thread: &ChatThread,
    idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (id_pin, id_ai, id_ren, id_del) = (
        thread.id.clone(),
        thread.id.clone(),
        thread.id.clone(),
        thread.id.clone(),
    );
    let (pin_tip, pin_lbl) = if thread.pinned {
        ("Unpin from top", "Unpin conversation")
    } else {
        ("Pin to top", "Pin conversation")
    };

    h_flex()
        .w_full()
        .px(t.space.compact)
        .pb(t.space.hair)
        .justify_end()
        .items_center()
        .gap(t.space.hair)
        .child(
            action_btn(
                ("chat-thread-pin", idx),
                IconName::Pin,
                pin_tip,
                pin_lbl,
                cx,
            )
            .selected(thread.pinned)
            .on_click(cx.listener(move |shell, _, _, cx| shell.toggle_thread_pin(&id_pin, cx))),
        )
        .children((!thread.messages.is_empty()).then(|| {
            action_btn(
                ("chat-thread-ai-title", idx),
                IconName::Sparkles,
                "Generate a smart title",
                "Generate conversation title with AI",
                cx,
            )
            .on_click(cx.listener(move |shell, _, _, cx| shell.regenerate_thread_title(&id_ai, cx)))
        }))
        .child(
            action_btn(
                ("chat-thread-rename", idx),
                IconName::Pencil,
                "Rename conversation",
                "Rename conversation",
                cx,
            )
            .on_click(cx.listener(move |shell, _, window, cx| {
                shell.begin_rename_thread(&id_ren, window, cx);
            })),
        )
        .child(
            action_btn(
                ("chat-thread-delete", idx),
                IconName::Trash,
                "Delete conversation",
                "Delete conversation",
                cx,
            )
            .on_click(cx.listener(move |shell, _, _, cx| shell.delete_thread(&id_del, cx))),
        )
}

fn render_empty_chats_hint(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("chat-history-empty-hint")
        .test_support()
        .w_full()
        .p(t.space.small)
        .gap(t.space.small)
        .items_start()
        .rounded(t.radius.input)
        .bg(t.subtle)
        .child(
            div().pt(t.space.hair).flex_shrink_0().child(
                Icon::new(IconName::MessageCircle)
                    .xsmall()
                    .text_color(t.text_secondary),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(
                    "Start a conversation and it will stay saved here so you can return anytime.",
                ),
        )
}

fn render_empty_rail(
    filter: ChatHistoryFilter,
    searching: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    if searching {
        return v_flex()
            .id("chat-history-no-matches")
            .test_support()
            .w_full()
            .p(t.space.medium)
            .gap(t.space.compact)
            .items_center()
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("No chats match"),
            )
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child("Try a different search word."),
            )
            .into_any_element();
    }

    match filter {
        ChatHistoryFilter::Scheduled => v_flex()
            .id("chat-history-scheduled-placeholder")
            .test_support()
            .w_full()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.space.compact)
                    .child(
                        Icon::new(IconName::Calendar)
                            .small()
                            .text_color(cx.theme().link),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child("No scheduled runs yet"),
                    ),
            )
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child("Runs started from Schedules will appear here."),
            )
            .child(
                design::button("chat-scheduled-open-schedules-btn", "Open Schedules", cx)
                    .small()
                    .w_full()
                    .on_click(
                        cx.listener(|shell, _, _, cx| shell.select_tab(NavTab::Schedules, cx)),
                    ),
            )
            .into_any_element(),
        ChatHistoryFilter::Chats => v_flex()
            .id("chat-history-empty-chats")
            .test_support()
            .w_full()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("No chats yet"),
            )
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child("Start a conversation and it will stay within reach here."),
            )
            .child(
                design::primary_button("chat-empty-start-btn", "Start your first chat", cx)
                    .small()
                    .w_full()
                    .on_click(cx.listener(|shell, _, window, cx| shell.start_new_chat(window, cx))),
            )
            .into_any_element(),
    }
}
