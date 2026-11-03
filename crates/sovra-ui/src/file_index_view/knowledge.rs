//! Read saved file connections and their cited text without running a model.

use super::*;
use sovra_core::file_index::{
    EvidenceTier,
    graph::{GraphEndpoint, GraphPage, GraphQuery, GraphRelation},
};

pub(super) struct KnowledgeView {
    pub(super) active: bool,
    search: Entity<InputState>,
    _search: gpui_kit::Subscription,
    selected: Option<String>,
    trail: Vec<GraphQuery>,
}

impl KnowledgeView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Find a person, project or statement…")
        });
        let subscription =
            cx.subscribe_in(&search, window, |shell, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && !shell.file_index.state.knowledge_browser.loading
                {
                    shell.search_file_knowledge(cx);
                }
            });
        Self {
            active: false,
            search,
            _search: subscription,
            selected: None,
            trail: vec![],
        }
    }
}

impl Shell {
    fn browse_file_knowledge(&mut self, query: GraphQuery, remember: bool, cx: &mut Context<Self>) {
        if remember {
            let trail = &mut self.file_index.knowledge.trail;
            if trail.len() == 32 {
                trail.remove(0);
            }
            trail.push(self.file_index.state.knowledge_browser.query.clone());
        }
        self.file_index.knowledge.active = true;
        self.file_index.knowledge.selected = None;
        let command = self.file_index.state.knowledge_browser.browse(query);
        self.index_command(command, cx);
        self.knowledge_scroll.set_offset(gpui_kit::Point::default());
    }

    fn search_file_knowledge(&mut self, cx: &mut Context<Self>) {
        let text = self
            .file_index
            .knowledge
            .search
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let query = GraphQuery {
            text: (!text.is_empty()).then_some(text),
            before: None,
            entity_id: None,
            ..self.file_index.state.knowledge_browser.query.clone()
        };
        self.browse_file_knowledge(query, true, cx);
    }
}

pub(super) fn entry(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    design::button("knowledge-browse-saved", "Browse saved relationships", cx)
        .max_w_full()
        .on_click(cx.listener(|shell, _, window, cx| {
            shell.file_index.knowledge.trail.clear();
            shell
                .file_index
                .knowledge
                .search
                .update(cx, |input, cx| input.set_value("", window, cx));
            shell.browse_file_knowledge(GraphQuery::default(), false, cx);
        }))
}

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state.knowledge_browser;
    let mut panel = v_flex().id("knowledge-saved-browser").test_support().w_full().min_w_0().gap(t.space.medium)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Saved file relationships"))
        .child(div().sovra_text(TextRole::Body, cx).child("Connections reviewed and saved from your files, with their sources and uncertainty. Open a relationship to check the original wording or follow a person or project."))
        .child(design::button("knowledge-back-files", "Back to files", cx).self_start().max_w_full()
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.file_index.knowledge.active = false;
                shell.file_index.state.knowledge_browser.close_evidence();
                shell.knowledge_scroll.set_offset(gpui_kit::Point::default()); cx.notify();
            })));
    if state.evidence_id.is_some() {
        return panel.child(evidence(shell, cx)).into_any_element();
    }
    if let Some(relation) = state.page.as_ref().and_then(|page| {
        page.relations
            .iter()
            .find(|r| Some(&r.id) == shell.file_index.knowledge.selected.as_ref())
    }) {
        return panel.child(detail(shell, relation, cx)).into_any_element();
    }
    panel = panel
        .child(
            div()
                .id("knowledge-saved-search")
                .test_support()
                .w_full()
                .child(
                    Input::new(&shell.file_index.knowledge.search)
                        .aria_label("Search saved file relationships"),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("knowledge-saved-submit", "Search relationships", cx)
                        .max_w_full()
                        .disabled(state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.search_file_knowledge(cx))),
                )
                .child(
                    design::button("knowledge-saved-reset", "All saved relationships", cx)
                        .max_w_full()
                        .disabled(state.loading)
                        .on_click(cx.listener(|shell, _, window, cx| {
                            shell
                                .file_index
                                .knowledge
                                .search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            shell.browse_file_knowledge(GraphQuery::default(), true, cx);
                        })),
                ),
        )
        .child(
            Checkbox::new("knowledge-saved-stale")
                .cursor_pointer()
                .label("Include outdated sources")
                .checked(state.query.include_stale)
                .disabled(state.loading)
                .on_click(cx.listener(|shell, checked, _, cx| {
                    let query = GraphQuery {
                        include_stale: *checked,
                        before: None,
                        ..shell.file_index.state.knowledge_browser.query.clone()
                    };
                    shell.browse_file_knowledge(query, true, cx);
                })),
        )
        .child(
            Checkbox::new("knowledge-saved-history")
                .cursor_pointer()
                .label("Include replaced relationships")
                .checked(state.query.include_superseded)
                .disabled(state.loading)
                .on_click(cx.listener(|shell, checked, _, cx| {
                    let query = GraphQuery {
                        include_superseded: *checked,
                        before: None,
                        ..shell.file_index.state.knowledge_browser.query.clone()
                    };
                    shell.browse_file_knowledge(query, true, cx);
                })),
        );
    if !shell.file_index.knowledge.trail.is_empty() {
        panel = panel.child(
            design::button("knowledge-saved-back", "Back to previous results", cx)
                .self_start()
                .max_w_full()
                .disabled(state.loading)
                .on_click(cx.listener(|shell, _, window, cx| {
                    if let Some(query) = shell.file_index.knowledge.trail.pop() {
                        shell.file_index.knowledge.search.update(cx, |input, cx| {
                            input.set_value(query.text.clone().unwrap_or_default(), window, cx);
                        });
                        shell.browse_file_knowledge(query, false, cx);
                    }
                })),
        );
    }
    if state.loading {
        panel = panel.child(
            div()
                .id("knowledge-saved-loading")
                .test_support()
                .aria_label("Loading saved relationships")
                .child("Loading saved relationships…"),
        );
    }
    if let Some(error) = &state.error {
        panel = panel
            .child(
                div()
                    .id("knowledge-saved-error")
                    .test_support()
                    .aria_label(error.clone())
                    .child(error.clone()),
            )
            .child(
                design::button("knowledge-saved-retry", "Reload relationships", cx)
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        let query = GraphQuery {
                            before: None,
                            ..shell.file_index.state.knowledge_browser.query.clone()
                        };
                        shell.browse_file_knowledge(query, false, cx);
                    })),
            );
    }
    if let Some(page) = &state.page {
        if let Some(id) = &state.query.entity_id {
            let name = page
                .entities
                .iter()
                .find(|e| e.id == *id)
                .map_or("selected subject", |e| e.label.as_str());
            panel = panel.child(
                div()
                    .id("knowledge-saved-filter")
                    .test_support()
                    .child(format!("Connections for {name}")),
            );
        }
        if page.relations.is_empty() {
            let message = if page.next.is_some() {
                "No matching relationships in this batch. Continue to more relationships or narrow the search."
            } else {
                "No saved relationships match this view. Try another search, or review a helper's proposed file connections on the Files page."
            };
            panel = panel.child(
                div()
                    .id("knowledge-saved-empty")
                    .test_support()
                    .aria_label(message)
                    .child(message),
            );
            panel = panel.child(
                design::button("knowledge-find-helper", "Find a helper for this", cx)
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(|shell, _, window, cx| {
                        shell.show_skill_help("map-files", window, cx);
                    })),
            );
        }
        for (index, relation) in page.relations.iter().enumerate() {
            let id = relation.id.clone();
            panel = panel.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        div()
                            .sovra_text(TextRole::Reading, cx)
                            .child(relation.statement.clone()),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(status(relation)),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(format!("Saved {}", date(relation.basis.recorded_at))),
                    )
                    .child(
                        design::button(("knowledge-saved-open", index), "Read relationship", cx)
                            .self_start()
                            .max_w_full()
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.file_index.knowledge.selected = Some(id.clone());
                                shell
                                    .knowledge_scroll
                                    .set_offset(gpui_kit::Point::default());
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(cursor) = &page.next {
            let cursor = cursor.clone();
            panel = panel.child(
                design::button("knowledge-saved-next", "More relationships", cx)
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        let query = GraphQuery {
                            before: Some(cursor.clone()),
                            ..shell.file_index.state.knowledge_browser.query.clone()
                        };
                        shell.browse_file_knowledge(query, true, cx);
                    })),
            );
        } else if page.truncated {
            panel = panel.child(
                div().child("This view is incomplete. Narrow the search or choose a subject."),
            );
        }
    }
    panel.into_any_element()
}

fn detail(
    shell: &Shell,
    relation: &GraphRelation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let page = shell
        .file_index
        .state
        .knowledge_browser
        .page
        .as_ref()
        .expect("detail belongs to loaded page");
    let mut panel = v_flex()
        .id("knowledge-relationship")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(
            design::button("knowledge-relationship-back", "Back to relationships", cx)
                .self_start()
                .max_w_full()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.file_index.knowledge.selected = None;
                    shell
                        .knowledge_scroll
                        .set_offset(gpui_kit::Point::default());
                    cx.notify();
                })),
        )
        .child(
            div()
                .sovra_text(TextRole::Reading, cx)
                .child(relation.statement.clone()),
        )
        .child(
            div()
                .id("knowledge-relationship-status")
                .test_support()
                .aria_label(status(relation))
                .sovra_text(TextRole::Body, cx)
                .child(status(relation)),
        )
        .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "Saved {} · {}",
                date(relation.basis.recorded_at),
                relation
                    .origin
                    .as_ref()
                    .map_or("Origin not recorded", |o| o.agent.as_str())
            )));
    let validity = relation.basis.valid_time.as_ref();
    if let Some(confidence) = relation.basis.confidence {
        panel = panel.child(div().sovra_text(TextRole::Caption, cx).child(format!(
            "Confidence estimate: {:.0}% · not a verified accuracy score",
            confidence.value() * 100.
        )));
    }
    panel = panel.child(
        div()
            .sovra_text(TextRole::Caption, cx)
            .child(match validity {
                Some(v) if v.from.is_some() || v.until.is_some() => format!(
                    "Reported real-world dates: {} to {}",
                    v.from.map_or("unknown".into(), date),
                    v.until.map_or("unspecified".into(), date)
                ),
                _ => "Real-world dates were not recorded.".into(),
            }),
    );
    if let Some(at) = relation.expired_at {
        panel = panel.child(div().child(format!("Replaced {}", date(at))));
    }
    panel = panel.child(
        div()
            .sovra_text(TextRole::SectionTitle, cx)
            .child("Connected subjects"),
    );
    for (index, endpoint) in [&relation.subject, &relation.object]
        .into_iter()
        .enumerate()
    {
        panel = panel.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child(endpoint_label(page, endpoint)),
        );
        if index == 0 {
            panel = panel.child(div().sovra_text(TextRole::Caption, cx).child(
                if relation.relation == sovra_core::file_index::graph::RelationKind::GroundedIn {
                    "Supported by this source"
                } else {
                    relation.relation.label()
                },
            ));
        }
        if let GraphEndpoint::Entity(id) = endpoint {
            let id = id.clone();
            panel = panel.child(
                design::button(
                    ("knowledge-follow-subject", index),
                    "Explore this subject",
                    cx,
                )
                .self_start()
                .max_w_full()
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell
                        .file_index
                        .knowledge
                        .search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    let query = GraphQuery {
                        entity_id: Some(id.clone()),
                        text: None,
                        before: None,
                        ..shell.file_index.state.knowledge_browser.query.clone()
                    };
                    shell.browse_file_knowledge(query, true, cx);
                })),
            );
        }
    }
    let mut ids = std::collections::BTreeSet::from_iter(relation.evidence.iter().copied());
    for endpoint in [&relation.subject, &relation.object] {
        match endpoint {
            GraphEndpoint::Source(id) => {
                ids.insert(*id);
            }
            GraphEndpoint::Entity(id) => {
                if let Some(entity) = page.entities.iter().find(|e| e.id == *id) {
                    ids.extend(entity.evidence.iter().copied());
                }
            }
        }
    }
    panel = panel.child(
        div()
            .sovra_text(TextRole::SectionTitle, cx)
            .child("Supporting records"),
    );
    for (index, id) in ids.into_iter().enumerate() {
        if let Some(source) = page.sources.iter().find(|s| s.id == id) {
            panel = panel
                .child(div().sovra_text(TextRole::Body, cx).child(format!(
                    "{} · lines {}–{} · {}",
                    source.path,
                    source.start_line,
                    source.end_line,
                    if source.current {
                        "Current saved version"
                    } else {
                        "Outdated source"
                    }
                )))
                .child(
                    design::button(
                        ("knowledge-read-evidence", index),
                        format!("Read source {}", index + 1),
                        cx,
                    )
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        if let Some(command) =
                            shell.file_index.state.knowledge_browser.read_evidence(id)
                        {
                            shell.index_command(command, cx);
                            shell
                                .knowledge_scroll
                                .set_offset(gpui_kit::Point::default());
                        }
                    })),
                );
        }
    }
    panel
}

fn evidence(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state.knowledge_browser;
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.medium).child(
        design::button("knowledge-evidence-back", "Back to relationship", cx)
            .self_start()
            .max_w_full()
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.file_index.state.knowledge_browser.close_evidence();
                shell
                    .knowledge_scroll
                    .set_offset(gpui_kit::Point::default());
                cx.notify();
            })),
    );
    if state.reading {
        panel = panel.child(div().child("Reading saved source…"));
    }
    if let Some(error) = &state.evidence_error {
        panel = panel
            .child(
                div()
                    .id("knowledge-evidence-error")
                    .test_support()
                    .child(error.clone()),
            )
            .child(
                design::button("knowledge-evidence-retry", "Try reading again", cx)
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        if let Some(id) = shell.file_index.state.knowledge_browser.evidence_id
                            && let Some(command) =
                                shell.file_index.state.knowledge_browser.read_evidence(id)
                        {
                            shell.index_command(command, cx);
                        }
                    })),
            );
    }
    if let Some(evidence) = &state.evidence {
        panel =
            panel
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(evidence.source.path.clone()),
                )
                .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                    "Lines {}–{} · {}",
                    evidence.source.start_line,
                    evidence.source.end_line,
                    if evidence.source.current {
                        "Current saved version"
                    } else {
                        "Outdated source · the file has changed or is no longer present"
                    }
                )))
                .child(div().sovra_text(TextRole::Caption, cx).child(
                    "Original saved passage · this excerpt does not represent the entire file",
                ))
                .children(
                    evidence
                        .passage
                        .note
                        .as_ref()
                        .map(|note| div().child(note.clone())),
                )
                .child(
                    div()
                        .id("knowledge-evidence-text")
                        .test_support()
                        .aria_label(evidence.passage.text.clone())
                        .w_full()
                        .min_w_0()
                        .sovra_text(TextRole::Reading, cx)
                        .child(evidence.passage.text.clone()),
                );
    }
    panel
}

fn endpoint_label(page: &GraphPage, endpoint: &GraphEndpoint) -> String {
    match endpoint {
        GraphEndpoint::Entity(id) => page
            .entities
            .iter()
            .find(|e| e.id == *id)
            .map_or("Unavailable subject".into(), |e| {
                format!("{}: {}", e.kind.label(), e.label)
            }),
        GraphEndpoint::Source(id) => page
            .sources
            .iter()
            .find(|s| s.id == *id)
            .map_or("Unavailable source".into(), |s| s.path.clone()),
    }
}

fn status(relation: &GraphRelation) -> String {
    let tier = match relation.basis.tier {
        EvidenceTier::FilesystemObservation => "File observation",
        EvidenceTier::DeterministicExtraction => "Extracted from a file",
        EvidenceTier::HeuristicExtraction => "Estimated from a file · unverified",
        EvidenceTier::ModelInference => "A helper’s interpretation · not verified",
        EvidenceTier::UserAssertion => "A statement from you",
        EvidenceTier::LegacyImport => "Imported record · origin not confirmed",
    };
    format!(
        "{tier}{}{}",
        if relation.expired_at.is_some() {
            " · Replaced relationship"
        } else {
            ""
        },
        if relation.sources_current {
            ""
        } else {
            " · Outdated source"
        }
    )
}

fn date(value: u64) -> String {
    i64::try_from(value)
        .ok()
        .and_then(|v| time::OffsetDateTime::from_unix_timestamp(v).ok())
        .map_or("Unknown date".into(), |d| {
            format!("{} {} {} UTC", d.day(), d.month(), d.year())
        })
}
