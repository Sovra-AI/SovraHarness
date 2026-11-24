//! Read saved file connections and their cited text without running a model.

use super::*;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, button::ButtonVariants as _};
use sovra_core::file_index::{
    EvidenceTier,
    graph::{GraphEndpoint, GraphPage, GraphQuery, GraphRelation},
};

pub(super) struct KnowledgeView {
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
            search,
            _search: subscription,
            selected: None,
            trail: vec![],
        }
    }
}

impl FileIndexView {
    pub(crate) fn can_refresh_relationships(&self) -> bool {
        self.knowledge.selected.is_none()
            && self.state.knowledge_browser.evidence_id.is_none()
            && self.state.knowledge_browser.query.before.is_none()
    }
}
impl Shell {
    pub(crate) fn browse_file_knowledge(
        &mut self,
        query: GraphQuery,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        if remember {
            let trail = &mut self.file_index.knowledge.trail;
            if trail.len() == 32 {
                trail.remove(0);
            }
            trail.push(self.file_index.state.knowledge_browser.query.clone());
        }
        self.file_index.knowledge.selected = None;
        let command = self.file_index.state.knowledge_browser.browse(query);
        self.index_command(command, cx);
        self.scroll_to_knowledge(crate::knowledge_view::KnowledgeSection::Relationships);
    }

    pub(crate) fn filter_by_relationship_type(
        &mut self,
        relation: sovra_core::file_index::graph::RelationKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_index
            .knowledge
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        let previous = &self.file_index.state.knowledge_browser.query;
        let query = GraphQuery {
            relation: Some(relation),
            include_stale: previous.include_stale,
            include_superseded: previous.include_superseded,
            ..Default::default()
        };
        self.knowledge_vocabulary_open = false;
        self.browse_file_knowledge(query, true, cx);
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
            relation: None,
            direction: sovra_core::file_index::graph::GraphDirection::Either,
            hops: 1,
            ..self.file_index.state.knowledge_browser.query.clone()
        };
        self.browse_file_knowledge(query, true, cx);
    }
}

impl Shell {
    pub(crate) fn load_saved_relationships(&mut self, cx: &mut Context<Self>) {
        let state = &self.file_index.state.knowledge_browser;
        if !state.loading && self.file_index.can_refresh_relationships() {
            self.browse_file_knowledge(state.query.clone(), false, cx);
        }
    }
}

/// The saved relationships: a search box, one row of type filters, and a short
/// list of readable cards. Opening a card shows where it came from.
pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state.knowledge_browser;
    let mut panel = v_flex()
        .id("knowledge-saved-browser")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(crate::knowledge_view::parts::heading(
            "Relationships",
            "How people, places and things in your files connect. Open one to see the file and line it came from.",
            cx,
        ));
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
            h_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .child(
                    div()
                        .id("knowledge-saved-search")
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .child(
                            Input::new(&shell.file_index.knowledge.search)
                                .prefix(
                                    gpui_kit::component::Icon::new(
                                        gpui_kit::assets::IconName::Search,
                                    )
                                    .text_color(t.text_secondary),
                                )
                                .aria_label("Search saved file relationships"),
                        ),
                )
                .child(
                    design::button("knowledge-saved-submit", "Search", cx)
                        .flex_shrink_0()
                        .disabled(state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.search_file_knowledge(cx))),
                ),
        )
        .child(type_filters(shell, cx));
    if let Some(relation) = &state.query.relation {
        let label = format!("Showing only “{}”", relation.display_name());
        panel = panel.child(
            div()
                .id("knowledge-current-predicate")
                .test_support()
                .aria_label(label.clone())
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(label),
        );
    }
    if !shell.file_index.knowledge.trail.is_empty() {
        panel = panel.child(
            design::button("knowledge-saved-back", "Back", cx)
                .ghost()
                .small()
                .icon(gpui_kit::assets::IconName::ChevronLeft)
                .self_start()
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
                .sovra_text(TextRole::Caption, cx)
                .child("Loading relationships…"),
        );
    }
    if let Some(error) = &state.error {
        panel = panel.child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .id("knowledge-saved-error")
                        .test_support()
                        .aria_label(error.clone())
                        .flex_1()
                        .min_w_0()
                        .sovra_text(TextRole::Body, cx)
                        .child(error.clone()),
                )
                .child(
                    design::button("knowledge-saved-retry", "Try again", cx)
                        .small()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            let query = GraphQuery {
                                before: None,
                                ..shell.file_index.state.knowledge_browser.query.clone()
                            };
                            shell.browse_file_knowledge(query, false, cx);
                        })),
                ),
        );
    }
    if let Some(page) = &state.page {
        if let Some(id) = &state.query.entity_id {
            let name = page
                .entities
                .iter()
                .find(|e| e.id == *id)
                .map_or("this subject", |e| e.label.as_str());
            panel = panel.child(
                div()
                    .id("knowledge-saved-filter")
                    .test_support()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(format!("Showing what connects to {name}")),
            );
        }
        if page.relations.is_empty() {
            let message = if page.next.is_some() {
                "Nothing matches in this batch. Show more, or try other words."
            } else {
                "No relationships match. Sovra adds them as it reads your folders."
            };
            panel = panel.child(
                h_flex()
                    .id("knowledge-saved-empty")
                    .test_support()
                    .aria_label(message)
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Info)
                            .text_color(t.text_secondary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .sovra_text(TextRole::Body, cx)
                            .child(message),
                    ),
            );
        }
        let mut list = v_flex().w_full().min_w_0().gap(t.space.small);
        for (index, relation) in page.relations.iter().enumerate() {
            list = list.child(card(index, page, relation, cx));
        }
        panel = panel.child(list);
        if let Some(cursor) = &page.next {
            let cursor = cursor.clone();
            panel = panel.child(
                design::button("knowledge-saved-next", "Show more", cx)
                    .self_start()
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
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Not everything fits here. Search for a word or open a subject."),
            );
        }
    }
    panel
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.medium)
                .child(
                    Checkbox::new("knowledge-saved-stale")
                        .cursor_pointer()
                        .label("Show changed files")
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
                        .label("Show replaced ones")
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
                ),
        )
        .into_any_element()
}

/// "All" plus the relationship types actually found, with how many of each.
fn type_filters(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let query = &shell.file_index.state.knowledge_browser.query;
    let everything = query.relation.is_none() && query.text.is_none() && query.entity_id.is_none();
    let loading = shell.file_index.state.knowledge_browser.loading;
    let mut row = h_flex()
        .id("knowledge-saved-types")
        .test_support()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.compact)
        .child(
            design::button("knowledge-saved-reset", "All", cx)
                .small()
                .selected(everything)
                .disabled(loading)
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell
                        .file_index
                        .knowledge
                        .search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    shell.browse_file_knowledge(GraphQuery::default(), true, cx);
                })),
        );
    let found = shell
        .file_index
        .state
        .discoveries
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|r| r.predicates.clone())
        .unwrap_or_default();
    for (index, (kind, count)) in found.into_iter().enumerate() {
        let selected = query.relation.as_ref() == Some(&kind);
        let label = format!(
            "{} · {}",
            kind.display_name(),
            sovra_core::file_index::ontology::group_digits(count)
        );
        row = row.child(
            design::button(("knowledge-type", index), label, cx)
                .small()
                .max_w_full()
                .selected(selected)
                .disabled(loading)
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.filter_by_relationship_type(kind.clone(), window, cx);
                })),
        );
    }
    row
}

/// One relationship as a sentence: subject, the relationship, object.
fn card(
    index: usize,
    page: &GraphPage,
    relation: &GraphRelation,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (subject, subject_kind) = endpoint_name(page, &relation.subject);
    let (object, object_kind) = endpoint_name(page, &relation.object);
    // A card names the file; its full folder path is on the opened relationship.
    let object = match relation.object {
        GraphEndpoint::Source(_) => object
            .rsplit(['/', '\\'])
            .next()
            .map_or(object.clone(), str::to_owned),
        GraphEndpoint::Entity(_) => object,
    };
    let verb = if relation.relation == sovra_core::file_index::graph::RelationKind::GroundedIn {
        "comes from".to_owned()
    } else {
        relation.relation.display_name()
    };
    // Kinds read from loose wording are guesses ("Task → Task"); only a table or
    // you name them reliably, so only then are they worth a line on the card.
    let declared = matches!(
        relation.basis.tier,
        EvidenceTier::DeterministicExtraction | EvidenceTier::UserAssertion
    );
    let kinds = match (
        subject_kind.filter(|_| declared),
        object_kind.filter(|_| declared),
    ) {
        (Some(s), Some(o)) => format!("{s} → {o} · "),
        (Some(s), None) => format!("{s} · "),
        _ => String::new(),
    };
    let caption = format!(
        "{kinds}{} · saved {}",
        status(relation),
        date(relation.basis.recorded_at)
    );
    let id = relation.id.clone();
    design::button(("knowledge-saved-open", index), "", cx)
        .outline()
        .w_full()
        .h_auto()
        .py(t.space.small)
        .accessibility_label(format!("{subject} {verb} {object}. {caption}"))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(t.space.medium)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .items_start()
                        .gap(t.space.hair)
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .flex_wrap()
                                .gap(t.space.compact)
                                .child(
                                    div()
                                        .min_w_0()
                                        .line_clamp(2)
                                        .sovra_text(TextRole::Body, cx)
                                        .child(subject),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .sovra_text(TextRole::Body, cx)
                                        .text_color(cx.theme().link)
                                        .child(verb),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .line_clamp(2)
                                        .sovra_text(TextRole::Body, cx)
                                        .child(object),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .max_w_full()
                                .line_clamp(2)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(caption),
                        ),
                )
                .child(
                    gpui_kit::component::Icon::new(gpui_kit::assets::IconName::ChevronRight)
                        .text_color(t.text_secondary),
                ),
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.file_index.knowledge.selected = Some(id.clone());
            shell.scroll_to_knowledge(crate::knowledge_view::KnowledgeSection::Relationships);
            cx.notify();
        }))
}

/// One relationship, its facts, and the files behind it.
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
    let (subject, _) = endpoint_name(page, &relation.subject);
    let (object, _) = endpoint_name(page, &relation.object);
    let verb = if relation.relation == sovra_core::file_index::graph::RelationKind::GroundedIn {
        "comes from".to_owned()
    } else {
        relation.relation.display_name()
    };
    let fact = |label: &str, value: String, cx: &mut Context<Shell>| {
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                div()
                    .w(t.layout.filter_width)
                    .flex_shrink_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(label.to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .sovra_text(TextRole::Body, cx)
                    .child(value),
            )
    };
    let mut facts = v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .id("knowledge-relationship-status")
                .test_support()
                .aria_label(status(relation))
                .child(fact("How Sovra knows", status(relation), cx)),
        )
        .child(fact(
            "Saved",
            format!(
                "{} · {}",
                date(relation.basis.recorded_at),
                relation
                    .origin
                    .as_ref()
                    .map_or("origin not recorded", |o| o.agent.as_str())
            ),
            cx,
        ));
    if let Some(confidence) = relation.basis.confidence {
        facts = facts.child(fact(
            "Confidence",
            format!(
                "{:.0}% · an estimate, not a checked score",
                confidence.value() * 100.
            ),
            cx,
        ));
    }
    let validity = relation.basis.valid_time.as_ref();
    facts = facts.child(fact(
        "True during",
        match validity {
            Some(v) if v.from.is_some() || v.until.is_some() => format!(
                "{} to {}",
                v.from.map_or("unknown".into(), date),
                v.until.map_or("now".into(), date)
            ),
            _ => "Not stated in the file".into(),
        },
        cx,
    ));
    if let Some(at) = relation.expired_at {
        facts = facts.child(fact("Replaced", date(at), cx));
    }
    let mut panel = v_flex()
        .id("knowledge-relationship")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(
            design::button("knowledge-relationship-back", "All relationships", cx)
                .ghost()
                .small()
                .icon(gpui_kit::assets::IconName::ChevronLeft)
                .self_start()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.file_index.knowledge.selected = None;
                    shell.scroll_to_knowledge(
                        crate::knowledge_view::KnowledgeSection::Relationships,
                    );
                    cx.notify();
                })),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.small)
                .child(div().sovra_text(TextRole::PageTitle, cx).child(subject))
                .child(
                    div()
                        .sovra_text(TextRole::PageTitle, cx)
                        .text_color(cx.theme().link)
                        .child(verb),
                )
                .child(div().sovra_text(TextRole::PageTitle, cx).child(object)),
        )
        .child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!("In the file: “{}”", relation.statement)),
        )
        .child(facts);
    let mut explore = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
    for (index, endpoint) in [&relation.subject, &relation.object]
        .into_iter()
        .enumerate()
    {
        if let GraphEndpoint::Entity(id) = endpoint {
            let id = id.clone();
            let (name, _) = endpoint_name(page, endpoint);
            explore = explore.child(
                design::button(
                    ("knowledge-follow-subject", index),
                    format!("Everything about {name}"),
                    cx,
                )
                .small()
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
    if let GraphEndpoint::Entity(id) = &relation.subject {
        let id = id.clone();
        let predicate = relation.relation.clone();
        explore = explore.child(
            design::button(
                "knowledge-follow-predicate",
                format!("Follow “{}” further", predicate.display_name()),
                cx,
            )
            .small()
            .max_w_full()
            .on_click(cx.listener(move |shell, _, _, cx| {
                let query = GraphQuery {
                    entity_id: Some(id.clone()),
                    direction: sovra_core::file_index::graph::GraphDirection::Outgoing,
                    relation: Some(predicate.clone()),
                    hops: 2,
                    text: None,
                    before: None,
                    ..shell.file_index.state.knowledge_browser.query.clone()
                };
                shell.browse_file_knowledge(query, true, cx);
            })),
        );
    }
    panel = panel.child(explore);
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
    let mut sources = v_flex().w_full().min_w_0().gap(t.space.small).child(
        div()
            .sovra_text(TextRole::SectionTitle, cx)
            .child("Where it came from"),
    );
    for (index, id) in ids.into_iter().enumerate() {
        if let Some(source) = page.sources.iter().find(|s| s.id == id) {
            sources = sources.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap(t.space.small)
                    .p(t.space.small)
                    .rounded(t.radius.card)
                    .border_1()
                    .border_color(t.material_border)
                    .child(
                        gpui_kit::component::Icon::new(gpui_kit::assets::IconName::FileText)
                            .text_color(t.text_secondary),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .sovra_text(TextRole::Body, cx)
                                    .child(source.path.clone()),
                            )
                            .child(
                                div()
                                    .sovra_text(TextRole::Caption, cx)
                                    .text_color(t.text_secondary)
                                    .child(format!(
                                        "Lines {}–{} · {}",
                                        source.start_line,
                                        source.end_line,
                                        if source.current {
                                            "up to date"
                                        } else {
                                            "the file has changed since"
                                        }
                                    )),
                            ),
                    )
                    .child(
                        design::button(("knowledge-read-evidence", index), "Read", cx)
                            .small()
                            .flex_shrink_0()
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                if let Some(command) =
                                    shell.file_index.state.knowledge_browser.read_evidence(id)
                                {
                                    shell.index_command(command, cx);
                                    shell.scroll_to_knowledge(
                                        crate::knowledge_view::KnowledgeSection::Relationships,
                                    );
                                }
                            })),
                    ),
            );
        }
    }
    panel.child(sources)
}

fn evidence(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state.knowledge_browser;
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.medium).child(
        design::button("knowledge-evidence-back", "Back to relationship", cx)
            .ghost()
            .small()
            .icon(gpui_kit::assets::IconName::ChevronLeft)
            .self_start()
            .max_w_full()
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.file_index.state.knowledge_browser.close_evidence();
                shell.scroll_to_knowledge(crate::knowledge_view::KnowledgeSection::Relationships);
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

/// A subject's name and kind, or a source file's path.
fn endpoint_name(page: &GraphPage, endpoint: &GraphEndpoint) -> (String, Option<String>) {
    match endpoint {
        GraphEndpoint::Entity(id) => page
            .entities
            .iter()
            .find(|e| e.id == *id)
            .map_or(("Unavailable subject".into(), None), |e| {
                (e.label.clone(), Some(e.kind.display_name()))
            }),
        GraphEndpoint::Source(id) => page
            .sources
            .iter()
            .find(|s| s.id == *id)
            .map_or(("Unavailable source".into(), None), |s| {
                (s.path.clone(), None)
            }),
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
            format!("{} {} {}", d.day(), d.month(), d.year())
        })
}
