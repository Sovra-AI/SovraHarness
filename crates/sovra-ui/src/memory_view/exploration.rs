//! Reading-first cards and explicit connection trails for real saved knowledge.

use super::*;
use sovra_core::{Fact, MemoryDetail, Timestamp};

use crate::design::FitsPane as _;

pub(super) fn date(stamp: Timestamp) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(stamp.millis()) * 1_000_000)
        .map_or_else(
            |_| "Date unavailable".into(),
            |value| format!("{} {} {}", value.day(), value.month(), value.year()),
        )
}

pub(super) fn source_excerpt(content: &str) -> (String, bool) {
    match content.char_indices().nth(600) {
        Some((end, _)) => (format!("{}…", &content[..end]), true),
        None => (content.to_owned(), false),
    }
}

fn standing(fact: &Fact) -> &'static str {
    if fact.stopped_at.is_some() {
        "Stopped"
    } else if !fact.is_current() {
        "Earlier version"
    } else if fact.extraction.is_some() {
        "Interpreted from a source"
    } else {
        "Saved detail"
    }
}

impl MemoryView {
    pub(super) fn subject_buttons(
        &self,
        fact: &Fact,
        prefix: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .children(
                fact.entities
                    .iter()
                    .take(16)
                    .enumerate()
                    .map(|(index, subject)| {
                        let name = subject.clone();
                        let label = if subject.eq_ignore_ascii_case("user") {
                            "You".to_owned()
                        } else {
                            subject.clone()
                        };
                        design::button(
                            (gpui_kit::SharedString::from(prefix.to_owned()), index),
                            label,
                            cx,
                        )
                        .disabled(self.state.loading)
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.query.subject = Some(name.clone());
                                view.search
                                    .update(cx, |search, cx| search.set_value("", window, cx));
                                view.load(cx);
                            },
                        ))
                    }),
            )
    }

    pub(super) fn saved_card(
        &self,
        fact: &Fact,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let id = fact.id.clone();
        let node = sovra_core::MemoryGraphNodeId::Detail(fact.id.clone());
        let on_map = self.graph_selection.as_ref() == Some(&node);
        let card = v_flex()
            .id(("memory-card", index))
            .test_support()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .when(on_map, |card| card.border_color(t.graph.detail))
            // Pointing at a card lights the same detail up on the map above.
            .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                view.preview_on_map(&node, *hovered, cx);
            }));
        let card = if self.width_class == design::WidthClass::Compact {
            card.w_full()
        } else {
            card.prefer_width(t.layout.card_min_width)
        };
        card.child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    div()
                        .id(("memory-card-standing", index))
                        .test_support()
                        .sovra_text(TextRole::Caption, cx)
                        .child(standing(fact)),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!("· Saved {}", date(fact.created_at))),
                ),
        )
        .child(
            div()
                .id(("memory-card-text", index))
                .test_support()
                .aria_label(fact.content.clone())
                .min_w_0()
                .flex_1()
                .sovra_text(TextRole::Reading, cx)
                .child(fact.content.clone()),
        )
        .child(self.subject_buttons(fact, &format!("memory-card-subject-{index}"), cx))
        .child(
            design::button(("memory-open", index), "Explore detail", cx)
                .self_start()
                .max_w_full()
                .disabled(self.state.loading)
                .on_click(cx.listener(move |view, _, _, cx| view.inspect(id.clone(), cx))),
        )
    }

    pub(super) fn connected_details(
        &self,
        detail: &MemoryDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex().id("memory-connections").test_support()
            .w_full().min_w_0().gap(t.space.small)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("Connected knowledge"))
            .child(div().sovra_text(TextRole::Caption, cx)
                .child("Follow a source, an update, or a shared subject. Mentioning the same person or project does not prove a relationship."));
        if detail.neighborhood.connections.is_empty() {
            panel = panel.child(div().id("memory-connections-empty").test_support()
                .child("No connected details found yet. As you share more, links to the same people, projects and sources can appear here."));
        }
        for (index, link) in detail.neighborhood.connections.iter().enumerate() {
            let id = link.fact.id.clone();
            panel = panel.child(
                v_flex()
                    .id(("memory-connection", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .gap(t.space.small)
                    .sovra_surface(Surface::Matte, cx)
                    .child(
                        div()
                            .id(("memory-connection-reason", index))
                            .test_support()
                            .aria_label(link.reason.label())
                            .sovra_text(TextRole::Caption, cx)
                            .child(link.reason.label()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .sovra_text(TextRole::Reading, cx)
                            .child(link.fact.content.clone()),
                    )
                    .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                        "{} · {}",
                        standing(&link.fact),
                        date(link.fact.created_at)
                    )))
                    .child(
                        design::button(
                            ("memory-connection-open", index),
                            "Open connected detail",
                            cx,
                        )
                        .self_start()
                        .max_w_full()
                        .disabled(self.state.loading)
                        .on_click(cx.listener(move |view, _, _, cx| view.inspect(id.clone(), cx))),
                    ),
            );
        }
        if detail.neighborhood.has_more {
            panel = panel.child(div().id("memory-connections-more").test_support()
                .child("Showing a selection of connections. Select a subject above to browse more saved details."));
        }
        panel
    }

    pub(super) fn inspect(&mut self, id: FactId, cx: &mut Context<Self>) {
        if let Some(detail) = &self.state.detail
            && detail.fact.id != id
        {
            if self.trail.len() == 32 {
                self.trail.remove(0);
            }
            self.trail.push(detail.fact.id.clone());
        }
        self.expanded_source = None;
        self.editing = None;
        self.stopping = None;
        self.send(MemoryAction::Inspect(id), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_source_previews_preserve_unicode_and_short_sources_stay_exact() {
        assert_eq!(
            source_excerpt("An exact quote."),
            ("An exact quote.".into(), false)
        );
        let content = "茶".repeat(601);
        let (excerpt, shortened) = source_excerpt(&content);
        assert!(shortened);
        assert_eq!(excerpt, format!("{}…", "茶".repeat(600)));
        assert!(!source_excerpt(&"茶".repeat(600)).1);
    }

    #[test]
    fn dates_are_readable_and_do_not_expose_epoch_numbers() {
        assert_eq!(date(Timestamp(0)), "1 January 1970");
        assert_eq!(date(Timestamp(-86_400_000)), "31 December 1969");
        assert_eq!(date(Timestamp(i64::MAX)), "Date unavailable");
    }
}
