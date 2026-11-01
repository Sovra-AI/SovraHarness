//! Explicit review of derived file knowledge; model tools cannot approve these changes.

use crate::{
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::file_index::{IndexCommand, IndexState, graph::KnowledgeProposal};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn date(value: Option<u64>) -> String {
    value
        .and_then(|v| i64::try_from(v).ok())
        .and_then(|v| OffsetDateTime::from_unix_timestamp(v).ok())
        .and_then(|v| v.format(&Rfc3339).ok())
        .unwrap_or_else(|| "Unknown".into())
}

pub(crate) fn render(state: &IndexState, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().id("knowledge-review").test_support().w_full().min_w_0().gap(t.space.small)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Review file knowledge"))
        .child(div().sovra_text(TextRole::Caption, cx).child("Helpers can propose relationships from indexed sources. Review each relationship and its evidence before saving. Saved inferences remain unverified; your source files stay intact."))
        .child(design::button("reload-knowledge", "Reload proposals", cx)
            .disabled(!state.knowledge_reviewing.is_empty())
            .on_click(cx.listener(|shell, _, _, cx| shell.index_command(IndexCommand::ListKnowledge, cx))))
        .children(state.knowledge_pending.iter().map(|proposal| card(proposal, state.knowledge_reviewing.contains(&proposal.id), cx)))
}

fn card(
    proposal: &KnowledgeProposal,
    reviewing: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let id = proposal.id;
    let mut card = v_flex()
        .id(SharedString::from(format!("knowledge-proposal-{id}")))
        .test_support()
        .min_w_0()
        .w_full()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(proposal.draft.title.clone()),
        )
        .child(div().sovra_text(TextRole::Caption, cx).child(format!(
            "Proposed by {} · {} · Model inference, unverified",
            proposal.origin.agent,
            date(Some(proposal.created_at))
        )));
    for entity in &proposal.draft.entities {
        card = card.child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .child(format!(
                    "New {}: {} · Sources {}",
                    entity.kind.label(),
                    entity.label,
                    indices(&entity.evidence)
                )),
        );
    }
    for (index, relation) in proposal.draft.relations.iter().enumerate() {
        let confidence = relation.confidence.map_or_else(
            || "Not supplied".into(),
            |v| format!("{:.0}% (model estimate, uncalibrated)", v.value() * 100.),
        );
        let valid = relation.valid_time.as_ref();
        let mut row = v_flex()
            .id(SharedString::from(format!(
                "knowledge-relation-{id}-{index}"
            )))
            .test_support()
            .min_w_0()
            .w_full()
            .gap(t.space.compact)
            .child(
                div()
                    .min_w_0()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!(
                        "{} → {} → {}",
                        proposal.endpoint_label(&relation.subject),
                        relation.relation.label(),
                        proposal.endpoint_label(&relation.object)
                    )),
            )
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child(relation.statement.clone()),
            )
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "Sources {} · Confidence: {confidence}",
                indices(&relation.evidence)
            )))
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "Real-world dates: {} through {} (UTC)",
                date(valid.and_then(|v| v.from)),
                date(valid.and_then(|v| v.until))
            )));
        if let Some(old) = relation
            .replaces
            .as_ref()
            .and_then(|id| proposal.replaced_relations.iter().find(|r| r.id == *id))
        {
            row = row.child(div().sovra_text(TextRole::Body, cx).child(format!(
                "Supersedes [{}]: {} → {} → {}. {} The earlier record stays in history.",
                old.id,
                proposal.saved_endpoint_label(&old.subject),
                old.relation.label(),
                proposal.saved_endpoint_label(&old.object),
                old.statement
            )));
        }
        card = card.child(row);
    }
    for (index, source) in proposal.sources.iter().enumerate() {
        if let Some(note) = &source.note {
            card = card.child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child(format!("Source {} extraction note: {note}", index + 1)),
            );
        }
        // A bounded passage is shown in full so the approval does not hide evidence.
        card = card.child(
            v_flex()
                .min_w_0()
                .w_full()
                .gap(t.space.compact)
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!(
                            "Source {} · {} · lines {}–{} · version {}",
                            index + 1,
                            source.path,
                            source.start_line,
                            source.end_line,
                            source.reference.version_id
                        )),
                )
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Body, cx)
                        .child(source.text.clone()),
                ),
        );
    }
    card.child(
        h_flex()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::button(
                    SharedString::from(format!("save-knowledge-{id}")),
                    "Save inferences",
                    cx,
                )
                .disabled(reviewing)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    if shell.file_index.state.begin_review(id) {
                        shell.index_command(
                            IndexCommand::ReviewKnowledge {
                                proposal_id: id,
                                approve: true,
                            },
                            cx,
                        );
                    }
                })),
            )
            .child(
                design::button(
                    SharedString::from(format!("reject-knowledge-{id}")),
                    "Reject",
                    cx,
                )
                .disabled(reviewing)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    if shell.file_index.state.begin_review(id) {
                        shell.index_command(
                            IndexCommand::ReviewKnowledge {
                                proposal_id: id,
                                approve: false,
                            },
                            cx,
                        );
                    }
                })),
            ),
    )
}

fn indices(values: &[u8]) -> String {
    values
        .iter()
        .map(|v| (usize::from(*v) + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
    use sovra_core::{
        Command, Event, NavTab,
        file_index::{IndexEvent, IndexProgress, IndexStatus, IndexedRoot},
        models::InterfaceScale,
    };
    use sovra_runtime::Client;

    fn reveal_review_button(
        window: &mut gpui_kit::Window,
        id: &'static str,
        height: f32,
        cx: &mut gpui_kit::App,
    ) {
        for _ in 0..100 {
            window.render_frame(cx);
            let item = window.find(id);
            let bounds = item.bounds();
            if item.visible() && bounds.bottom() <= px(height - 60.) && bounds.top() >= px(60.) {
                return;
            }
            let delta = if bounds.top() < px(60.) { 80. } else { -80. };
            window.scroll(
                "sovra-knowledge-view",
                ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("{id} must remain reachable by scrolling");
    }

    fn proposal() -> KnowledgeProposal {
        serde_json::from_value(serde_json::json!({
            "id":1,"state":"pending","created_at":100,"origin":{"session_id":"s","invocation_id":"i","agent":"File helper"},
            "draft":{"title":"Map the Orchid brief","evidence":[{"root_id":"root","file_id":"file","version_id":"v1","ordinal":0}],
                "entities":[{"local_id":"doc","kind":"document","label":"Orchid brief","evidence":[0]}],
                "relations":[{"subject":{"kind":"new","id":"doc"},"relation":"grounded_in","object":{"kind":"source","id":0},"statement":"This source represents the brief.","evidence":[0],"confidence":0.7}]},
            "sources":[{"reference":{"root_id":"root","file_id":"file","version_id":"v1","ordinal":0},"path":format!("notes/{}/brief.md", "long-folder/".repeat(12)),"text":"The Orchid brief proposes a garden.","start_line":1,"end_line":1,"mime_type":"text/markdown","note":"Extracted text only.","observed_at":100,"modified_at":null}],
            "existing_entities":[],"replaced_relations":[],"required_roots":["root"]
        })).unwrap()
    }

    #[gpui_kit::test]
    fn review_requires_click_and_remains_reachable_in_both_themes_and_sizes(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                shell.update(cx, |shell, cx| {
                    shell
                        .file_index
                        .state
                        .apply(&IndexEvent::Roots(vec![IndexedRoot {
                            id: "root".into(),
                            path: "/chosen".into(),
                            status: IndexStatus::Ready,
                            progress: IndexProgress::default(),
                            issues: vec![],
                            completed_at: Some(100),
                            auto_refresh: false,
                            scan_options: sovra_core::file_index::ScanOptions::default(),
                        }]));
                    shell
                        .file_index
                        .state
                        .apply(&IndexEvent::KnowledgePending(vec![proposal()]));
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Files,
                        cx,
                    );
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    for id in [
                        "knowledge-proposal-1",
                        "knowledge-relation-1-0",
                        "save-knowledge-1",
                        "reject-knowledge-1",
                    ] {
                        let bounds = window.find(id).bounds();
                        assert!(
                            bounds.left() >= px(0.) && bounds.right() <= px(width),
                            "{id} at {scale}%: {bounds:?}"
                        );
                        assert!(bounds.size.height > px(0.));
                    }
                }
                assert!(
                    !std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                        c,
                        Command::FileIndex(IndexCommand::ReviewKnowledge { .. })
                    ))
                );
                let approve = scale != 200;
                let button = if approve {
                    "save-knowledge-1"
                } else {
                    "reject-knowledge-1"
                };
                reveal_review_button(window, button, height, cx);
                window.click(button, cx);
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|c| c
                    == Command::FileIndex(IndexCommand::ReviewKnowledge {
                        proposal_id: 1,
                        approve
                    })));
                assert!(
                    shell
                        .read(cx)
                        .file_index
                        .state
                        .knowledge_reviewing
                        .contains(&1)
                );
                window.render_frame(cx);
                reveal_review_button(window, "reject-knowledge-1", height, cx);
                window.click("reject-knowledge-1", cx);
                assert!(
                    !std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                        c,
                        Command::FileIndex(IndexCommand::ReviewKnowledge { .. })
                    ))
                );
            })
            .unwrap();
        }
        // Receipt of an event is never itself permission to save.
        events
            .send_blocking(Event::FileIndex(IndexEvent::KnowledgeReviewed {
                proposal_id: 1,
                accepted: true,
            }))
            .unwrap();
        cx.run_until_parked();
    }

    #[test]
    fn review_dates_are_calendar_dates_and_unknown_is_explicit() {
        assert_eq!(date(Some(0)), "1970-01-01T00:00:00Z");
        assert_eq!(date(None), "Unknown");
        assert_eq!(date(Some(u64::MAX)), "Unknown");
    }
}
