//! Conversation history section rendered inside the unified left sidebar:
//! - Search bar (`Search chats...`) filtering by title, subtitle, and message body.
//! - Segmented `Chats | Scheduled` filter pills (`ChatFilterPills` radio group).
//! - Scrollable conversation list partitioned into `Pinned` and recency buckets
//!   (`Today`, `Yesterday`, `Previous 7 Days`, `Previous 30 Days`, `Older`).
//! - Per-row topic icon, AI-generated or user title / inline rename input, subtitle preview,
//!   relative timestamp, and quick actions (`Pin`, `AI Title`, `Rename`, `Delete`).
//! - Dedicated empty states for `Chats` and `Scheduled`.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
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
    let reference_now = state
        .threads
        .iter()
        .filter(|th| th.is_active())
        .map(|th| th.updated_at)
        .max()
        .unwrap_or(Timestamp(0));
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
                .small()
                .text_color(t.text_secondary),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(history_search_input)
                    .id("chat-history-search")
                    .aria_label("Search chats")
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
    let theme = cx.theme();
    let fg_active = theme.foreground;
    let fg_inactive = t.text_secondary;

    h_flex()
        .id("chat-filter-pills")
        .test_support()
        .role(Role::RadioGroup)
        .aria_label("Filter conversations")
        .w_full()
        .flex_shrink_0()
        .p(t.space.hair)
        .gap(t.space.compact)
        .rounded(t.radius.pill)
        .sovra_surface(Surface::Subtle, cx)
        .children(ChatHistoryFilter::ALL.into_iter().map(|filter| {
            let is_active = filter == active_filter;
            let id = match filter {
                ChatHistoryFilter::Chats => "chat-filter-chats",
                ChatHistoryFilter::Scheduled => "chat-filter-scheduled",
            };
            design::button(id, filter.label(), cx)
                .ghost()
                .selected(is_active)
                .small()
                .min_h(t.layout.compact_control_height)
                .flex_1()
                .px(t.space.small)
                .rounded(t.radius.pill)
                .text_color(if is_active { fg_active } else { fg_inactive })
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.set_history_filter(filter, cx);
                }))
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
        .gap(t.space.compact)
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
    if state.history_filter == ChatHistoryFilter::Scheduled && grouped.is_empty() {
        return list.child(render_empty_rail(
            ChatHistoryFilter::Scheduled,
            searching,
            cx,
        ));
    }

    let mut row_idx = 0usize;
    if grouped
        .pinned
        .iter()
        .any(|thread| !thread.is_untouched_draft())
    {
        list = list.child(render_section_label("Pinned", cx));
        for thread in grouped
            .pinned
            .iter()
            .filter(|thread| !thread.is_untouched_draft())
        {
            list = list.child(render_thread_row(
                thread,
                row_idx,
                thread.id == state.session_id,
                state.renaming_thread_id.as_deref() == Some(&thread.id),
                reference_now,
                rename_thread_input,
                cx,
            ));
            row_idx += 1;
        }
    }

    for (bucket, items) in &grouped.groups {
        if items.iter().all(|thread| thread.is_untouched_draft()) {
            continue;
        }
        list = list.child(render_section_label(bucket.label(), cx));
        for thread in items.iter().filter(|thread| !thread.is_untouched_draft()) {
            list = list.child(render_thread_row(
                thread,
                row_idx,
                thread.id == state.session_id,
                state.renaming_thread_id.as_deref() == Some(&thread.id),
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
        .px(t.space.compact)
        .pt(t.space.small)
        .pb(t.space.compact)
        .sovra_text(TextRole::Caption, cx)
        .text_color(t.text_secondary)
        .child(label.to_owned())
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
    let theme = cx.theme();
    let icon = match infer_topic_icon(&thread.title, thread.kind) {
        ConversationTopicIcon::Chat => IconName::MessageCircle,
        ConversationTopicIcon::Code => IconName::Code,
        ConversationTopicIcon::Market => IconName::TrendingUp,
        ConversationTopicIcon::Housing => IconName::House,
        ConversationTopicIcon::Schedule => IconName::Calendar,
        ConversationTopicIcon::Search => IconName::Search,
        ConversationTopicIcon::Document => IconName::FileText,
    };
    let icon_color = if is_active {
        cx.theme().link
    } else {
        t.text_secondary
    };
    let time_label = format_relative_time(thread.updated_at, reference_now);
    let thread_id_select = thread.id.clone();
    let thread_id_pin = thread.id.clone();
    let thread_id_ai_title = thread.id.clone();
    let thread_id_rename = thread.id.clone();
    let thread_id_delete = thread.id.clone();
    let thread_id_commit = thread.id.clone();
    let is_pinned = thread.pinned;
    let has_messages = !thread.messages.is_empty();

    let surface = if is_active {
        Surface::NavActive
    } else {
        Surface::Chrome
    };

    let body = if is_renaming {
        h_flex()
            .w_full()
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
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.commit_rename_thread(&thread_id_commit, cx);
                    })),
            )
            .into_any_element()
    } else {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(t.space.hair)
            .child(
                h_flex()
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
                        div()
                            .flex_shrink_0()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(theme.muted_foreground)
                            .child(time_label),
                    ),
            )
            .children((!thread.last_message_preview.is_empty()).then(|| {
                div()
                    .w_full()
                    .truncate()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(thread.last_message_preview.clone())
            }))
            .into_any_element()
    };

    v_flex()
        .id(("chat-thread-card", idx))
        .w_full()
        .p(t.space.compact)
        .gap(t.space.compact)
        .sovra_surface(surface, cx)
        .child(if is_renaming {
            body
        } else {
            design::button(("chat-thread-row", idx), "", cx)
                .accessibility_label(format!("Open conversation {}", thread.title))
                .ghost()
                .h_auto()
                .min_h(t.layout.control_height)
                .p(t.space.compact)
                .w_full()
                .items_start()
                .gap(t.space.compact)
                .child(
                    div()
                        .pt(t.space.hair)
                        .flex_shrink_0()
                        .child(Icon::new(icon).small().text_color(icon_color)),
                )
                .child(body)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.select_thread(&thread_id_select, cx);
                }))
                .into_any_element()
        })
        .children(is_active.then(|| {
            h_flex()
                .w_full()
                .justify_end()
                .items_center()
                .gap(t.space.compact)
                .child(
                    design::button(("chat-thread-pin", idx), "", cx)
                        .icon(IconName::Pin)
                        .tooltip(if is_pinned { "Unpin" } else { "Pin" })
                        .accessibility_label(if is_pinned {
                            "Unpin conversation"
                        } else {
                            "Pin conversation"
                        })
                        .ghost()
                        .selected(is_pinned)
                        .xsmall()
                        .min_h(t.layout.compact_control_height)
                        .px(t.space.compact)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.toggle_thread_pin(&thread_id_pin, cx);
                        })),
                )
                .children(has_messages.then(|| {
                    design::button(("chat-thread-ai-title", idx), "", cx)
                        .icon(IconName::Sparkles)
                        .tooltip("Generate a title")
                        .accessibility_label("Generate conversation title with AI")
                        .ghost()
                        .xsmall()
                        .min_h(t.layout.compact_control_height)
                        .px(t.space.compact)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.regenerate_thread_title(&thread_id_ai_title, cx);
                        }))
                }))
                .child(
                    design::button(("chat-thread-rename", idx), "", cx)
                        .icon(IconName::Pencil)
                        .tooltip("Rename")
                        .accessibility_label("Rename conversation")
                        .ghost()
                        .xsmall()
                        .min_h(t.layout.compact_control_height)
                        .px(t.space.compact)
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.begin_rename_thread(&thread_id_rename, window, cx);
                        })),
                )
                .child(
                    design::button(("chat-thread-delete", idx), "", cx)
                        .icon(IconName::Trash)
                        .tooltip("Delete")
                        .accessibility_label("Delete conversation")
                        .ghost()
                        .xsmall()
                        .min_h(t.layout.compact_control_height)
                        .px(t.space.compact)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.delete_thread(&thread_id_delete, cx);
                        })),
                )
        }))
}

fn render_empty_chats_hint(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("chat-history-empty-hint")
        .test_support()
        .w_full()
        .p(t.space.small)
        .gap(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .child("Start a conversation and it will stay within reach here."),
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
                    .child("Try a different search."),
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
                    .child("Runs created by the scheduler will appear here."),
            )
            .child(
                design::button("chat-scheduled-open-schedules-btn", "Open Schedules", cx)
                    .small()
                    .w_full()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.select_tab(NavTab::Schedules, cx);
                    })),
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
                    .on_click(cx.listener(|shell, _, window, cx| {
                        shell.start_new_chat(window, cx);
                    })),
            )
            .into_any_element(),
    }
}
