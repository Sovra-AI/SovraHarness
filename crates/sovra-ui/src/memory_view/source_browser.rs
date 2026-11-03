//! Original records remain readable independently of fact extraction or recall.

use super::*;
use sovra_core::{MemorySourceMetadata, MemorySourceQuery, MemorySourceRecord, MemorySourceScope};

impl MemoryView {
    pub(super) fn load_sources(&mut self, query: MemorySourceQuery, cx: &mut Context<Self>) {
        if query.fact.is_none() {
            self.supporting_detail = None;
        }
        self.subject_query = None;
        self.source_query = Some(query.clone());
        self.source_only = false;
        self.state.detail = None;
        self.state.sources = sovra_core::MemorySourcePage::default();
        self.editing = None;
        self.stopping = None;
        self.source_stop = None;
        self.send(MemoryAction::ListSources(query), cx);
    }

    pub(super) fn search_sources(&mut self, cx: &mut Context<Self>) {
        self.load_sources(
            MemorySourceQuery {
                fact: self
                    .source_query
                    .as_ref()
                    .and_then(|query| query.fact.clone()),
                search: self.source_search.read(cx).value().to_string(),
                include_stopped: self
                    .source_query
                    .as_ref()
                    .is_some_and(|query| query.include_stopped),
                ..Default::default()
            },
            cx,
        );
    }

    pub(super) fn source_library(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let busy = self.state.loading;
        let supporting = self
            .source_query
            .as_ref()
            .is_some_and(|query| query.fact.is_some());
        let mut panel = v_flex().id("memory-source-library").test_support().w_full().min_w_0().gap(t.space.medium)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child(if supporting { "Supporting records" } else { "Original records" }))
            .child(div().sovra_text(TextRole::Body, cx).child(if supporting { "These records were linked to the selected detail when it was saved or repeated. Read their original wording to check the interpretation. Stopped records are hidden unless you include them." } else { "Read the messages, notes and other records kept with memory, newest first. A record can be saved even when no useful details were extracted. These are original statements, not necessarily current facts." }))
            .children(self.supporting_detail.as_ref().map(|detail| div().id("memory-supported-detail").test_support().aria_label(detail.label.clone()).sovra_text(TextRole::Reading, cx).child(detail.label.clone())))
            .children(self.supporting_detail.as_ref().map(|_| design::button("memory-sources-back-detail", "Back to detail", cx).self_start().max_w_full().disabled(busy)
                .on_click(cx.listener(|view, _, _, cx| {
                    if let Some(detail) = view.supporting_detail.take() {
                        view.source_query = None;
                        view.source_only = detail.source_only;
                        view.source_navigation = detail.navigation;
                        view.trail = detail.trail;
                        view.send(MemoryAction::Inspect(detail.id), cx);
                    }
                }))))
            .child(design::button("memory-sources-close", "Back to saved knowledge", cx).self_start().max_w_full().disabled(busy)
                .on_click(cx.listener(|view, _, _, cx| view.load(cx))))
            .child(div().id("memory-sources-input").test_support().w_full().min_w_0()
                .child(Input::new(&self.source_search).aria_label("Search original records")))
            .child(h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small)
                .child(design::button("memory-sources-search", "Search records", cx).max_w_full().disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| view.search_sources(cx))))
                .child(design::button("memory-sources-stopped", "Include stopped records", cx).max_w_full().disabled(busy)
                    .selected(self.source_query.as_ref().is_some_and(|query| query.include_stopped))
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(query) = view.source_query.as_mut() { query.include_stopped = !query.include_stopped; }
                        view.search_sources(cx);
                    }))));
        if let Some(notice) = &self.notice {
            panel = panel.child(
                div()
                    .id("memory-source-notice")
                    .test_support()
                    .aria_label(notice.clone())
                    .child(notice.clone()),
            );
        }
        if busy {
            panel = panel.child(
                div()
                    .id("memory-sources-loading")
                    .test_support()
                    .child("Loading original records…"),
            );
        } else if let Some(error) = &self.state.error {
            panel = panel.child(
                div()
                    .id("memory-sources-error")
                    .test_support()
                    .child(error.clone()),
            );
        } else if self.state.sources.sources.is_empty() {
            let message = if supporting {
                "No supporting records match this page. Try different words or include stopped records. Older imported details may have no original record attached."
            } else if self
                .source_query
                .as_ref()
                .is_some_and(|query| !query.search.trim().is_empty() || query.before.is_some())
            {
                "No records match this page. Try different words, include stopped records, or return to the newest records."
            } else {
                "No original records are available here yet. Messages saved while remembering is on and details you add can appear here."
            };
            panel = panel.child(
                div()
                    .id("memory-sources-empty")
                    .test_support()
                    .aria_label(message)
                    .child(message),
            );
        }
        for (index, source) in self.state.sources.sources.iter().enumerate() {
            let id = source.id.clone();
            let excerpt = if source.shortened {
                format!("{}…", source.excerpt)
            } else {
                source.excerpt.clone()
            };
            panel = panel.child(
                v_flex()
                    .id(("memory-source-card", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .gap(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                        "{} · {} · {}",
                        exploration::date(source.occurred_at),
                        kind_label(source.kind),
                        source.metadata.author_label()
                    )))
                    .child(
                        div()
                            .id(("memory-source-excerpt", index))
                            .test_support()
                            .aria_label(excerpt.clone())
                            .min_w_0()
                            .sovra_text(TextRole::Reading, cx)
                            .child(excerpt),
                    )
                    .child(
                        div()
                            .id(("memory-source-state", index))
                            .test_support()
                            .aria_label(source.metadata.status_label())
                            .sovra_text(TextRole::Caption, cx)
                            .child(source.metadata.status_label()),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(detail_count(&source.metadata)),
                    )
                    .child(
                        design::button(("memory-source-open", index), "Read original record", cx)
                            .self_start()
                            .max_w_full()
                            .disabled(busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let query = view.source_query.clone();
                                let supporting = view.supporting_detail.clone();
                                view.open_source(MemoryAction::InspectSource(id.clone()), cx);
                                view.source_query = query;
                                view.supporting_detail = supporting;
                                view.source_navigation = SourceNavigation::Sources;
                            })),
                    ),
            );
        }
        let mut paging = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
        if let Some(cursor) = &self.state.sources.next {
            let cursor = cursor.clone();
            paging = paging.child(
                design::button("memory-sources-next", "Older records", cx)
                    .max_w_full()
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(mut query) = view.source_query.clone() {
                            query.before = Some(cursor.clone());
                            view.load_sources(query, cx);
                        }
                    })),
            );
        }
        if self
            .source_query
            .as_ref()
            .is_some_and(|query| query.before.is_some() || !query.search.is_empty())
        {
            paging = paging.child(
                design::button("memory-sources-newest", "All newest records", cx)
                    .max_w_full()
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.source_search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.search_sources(cx);
                    })),
            );
        }
        panel.child(paging)
    }

    pub(super) fn source_reader(
        &self,
        record: &MemorySourceRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let source = &record.episode;
        let id = source.id.clone();
        let metadata = &record.metadata;
        let busy = self.state.loading;
        let mut panel = v_flex()
            .id("memory-original-source")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Original record"),
            )
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "{} · {} UTC",
                kind_label(source.kind),
                exploration::date(source.occurred_at)
            )))
            .child(
                div()
                    .id("memory-source-origin")
                    .test_support()
                    .aria_label(metadata.author_label())
                    .child(metadata.author_label()),
            )
            .child(
                div()
                    .id("memory-source-status")
                    .test_support()
                    .aria_label(metadata.status_label())
                    .child(metadata.status_label()),
            )
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                    "{}{}",
                    match metadata.scope {
                        MemorySourceScope::Private => "Private source",
                        MemorySourceScope::Unknown => "Sharing permission was not recorded",
                    },
                    metadata
                        .version
                        .map(|version| format!(" · Saved version {version}"))
                        .unwrap_or_default()
                )))
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child(detail_count(metadata)),
            );
        let mut actions = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
        match self.source_navigation {
            SourceNavigation::Conversation => {}
            SourceNavigation::Memories => {
                actions = actions.child(
                    design::button("memory-close-source", "Back to saved knowledge", cx)
                        .self_start()
                        .max_w_full()
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.load(cx))),
                );
            }
            SourceNavigation::Sources => {
                actions = actions.child(
                    design::button("memory-close-source", "Back to original records", cx)
                        .self_start()
                        .max_w_full()
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.load_sources(view.source_query.clone().unwrap_or_default(), cx);
                        })),
                );
            }
        }
        if metadata.detail_count.is_some_and(|count| count > 0) {
            let source_id = id.clone();
            actions = actions.child(
                design::button("memory-source-details", "View linked details", cx)
                    .self_start()
                    .max_w_full()
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.query = MemoryListQuery {
                            source: Some(source_id.clone()),
                            include_history: true,
                            ..Default::default()
                        };
                        view.load(cx);
                    })),
            );
        }
        let stop_id = id.clone();
        actions = actions.child(
            design::button("memory-stop-source", "Stop using this record", cx)
                .self_start()
                .max_w_full()
                .disabled(busy || metadata.revoked_at.is_some())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.source_stop = Some(stop_id.clone());
                    cx.notify();
                })),
        );
        panel = panel.child(actions);
        if self.source_stop.as_ref() == Some(&id) {
            panel = panel.child(div().child("Stop using this original record and its linked details? Pending checks will be cancelled. The record stays available for review."))
                .child(h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small)
                    .child(design::button("memory-confirm-stop-source", "Confirm stop using", cx).max_w_full().disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| view.send(MemoryAction::StopSource(id.clone()), cx))))
                    .child(design::button("memory-cancel-stop-source", "Cancel", cx).max_w_full().disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| { view.source_stop = None; cx.notify(); }))));
        }
        panel
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Original wording · may contain earlier claims, questions or plans"),
            )
            .child(
                div()
                    .id("memory-source")
                    .test_support()
                    .aria_label(source.content.clone())
                    .w_full()
                    .min_w_0()
                    .sovra_text(TextRole::Reading, cx)
                    .child(source.content.clone()),
            )
    }
}

fn detail_count(metadata: &MemorySourceMetadata) -> String {
    match metadata.detail_count {
        None => "Linked detail count is unavailable".into(),
        Some(0) => "No saved details linked · the original evidence is still readable".into(),
        Some(1) => "1 linked saved detail, including retained history".into(),
        Some(count) => format!("{count} linked saved details, including retained history"),
    }
}

fn kind_label(kind: sovra_core::EpisodeKind) -> &'static str {
    match kind {
        sovra_core::EpisodeKind::UserMessage => "Chat message",
        sovra_core::EpisodeKind::AgentMessage => "Helper reply",
        sovra_core::EpisodeKind::Note => "Saved note",
        sovra_core::EpisodeKind::ToolCall => "Helper action",
        sovra_core::EpisodeKind::ToolResult => "Action result",
    }
}
