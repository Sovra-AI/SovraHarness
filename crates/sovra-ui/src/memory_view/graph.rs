//! Picture of saved links, drawn from the pure layout, plus a local repeat check.
//!
//! The picture uses only links the domain already stored or confirmed. Words stay
//! in a card beside or under the drawing so a diagram is never the only way to read.

use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, h_flex, v_flex};
use gpui_kit::{
    Bounds, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder,
    Pixels, Rems, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    canvas, div, fill, point, size,
};
use sovra_core::{
    Fact, MemoryDuplicateReport, MemoryFactChat, MemoryGraph, MemoryGraphEdgeKind,
    MemoryGraphNodeId, MemoryGraphRecord, MemoryPoint, check_memory_duplicates,
    confirm_memory_duplicate, memory_graph,
};

use super::MemoryView;
use crate::design::{self, SovraStyled as _, Surface, TextRole};

struct GraphInk {
    canvas: Hsla,
    version: Hsla,
    message: Hsla,
    subject: Hsla,
    chat: Hsla,
    detail: Hsla,
    selected: Hsla,
}

impl MemoryView {
    pub(super) fn memory_picture(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut column = v_flex()
            .id("memory-graph")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("How these details connect"),
            )
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Lines join a shared subject, the same original message, an earlier version, or the chat a detail came from. The word you, on its own, is not a link."),
            );
        let facts = &self.state.page.facts;
        if facts.is_empty() {
            return column.child(
                div()
                    .id("memory-graph-empty")
                    .test_support()
                    .aria_label("Nothing is saved yet, so there is no picture. A detail you save, or one learned from chat, will appear here.")
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child("Nothing is saved yet, so there is no picture. A detail you save, or one learned from chat, will appear here."),
            );
        }
        if facts.len() == 1 {
            column = column.child(
                div()
                    .id("memory-graph-single")
                    .test_support()
                    .aria_label("One saved detail. Links appear when another detail shares a subject, a message, a version, or a chat.")
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child("One saved detail. Links appear when another detail shares a subject, a message, a version, or a chat."),
            );
        }
        let versions = self
            .duplicates
            .as_ref()
            .map_or(&[][..], |report| report.exact.as_slice());
        let records = graph_records(facts, &self.state.page.chats);
        let graph = memory_graph(&records, versions);
        let selected = self
            .graph_selection
            .clone()
            .filter(|id| graph.nodes.iter().any(|node| &node.id == id));
        column = column.child(self.graph_body(&graph, selected.as_ref(), cx));
        if graph.omitted {
            column = column.child(
                div()
                    .id("memory-graph-more")
                    .test_support()
                    .aria_label("More links exist than this picture shows.")
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child("More links exist than this picture shows."),
            );
        }
        column = column.child(self.link_lines(&graph, cx));
        column.child(self.duplicate_check(cx))
    }

    fn graph_body(
        &self,
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let t = design::tokens(cx);
        let canvas = self.graph_canvas(graph, selected, cx);
        let reading = self.reading_card(selected, cx);
        if self.width_class.fits_detail_pane() {
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(canvas)
                .child(reading)
                .into_any_element()
        } else {
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .child(canvas)
                .child(reading)
                .into_any_element()
        }
    }

    fn graph_canvas(
        &self,
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let accent = t.accent;
        let ink = GraphInk {
            canvas: t.subtle,
            version: accent.indigo,
            message: accent.violet,
            subject: accent.green,
            chat: accent.slate,
            detail: accent.violet,
            selected: accent.violet_light,
        };
        let hairline = t.layout.hairline;
        let diameter = t.layout.icon_medium;
        let height = t.layout.index_preview_height;
        let painted = graph.clone();
        let selected = selected.cloned();
        let drawing = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                paint_graph(
                    bounds,
                    &painted,
                    selected.as_ref(),
                    &ink,
                    diameter,
                    hairline,
                    window,
                );
            },
        )
        .w_full()
        .h(height);
        let mut nodes = v_flex().w_full().min_w_0().gap(t.space.small);
        for (index, node) in graph.nodes.iter().enumerate() {
            let id = node.id.clone();
            let label = node.label.clone();
            let chosen = selected_matches(selected_id(self), &node.id);
            nodes = nodes.child(
                design::button(("memory-graph-node", index), label, cx)
                    .w_full()
                    .max_w_full()
                    .selected(chosen)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.graph_selection = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .w_full()
            .min_w_0()
            .flex_1()
            .gap(t.space.small)
            .child(
                div()
                    .id("memory-graph-canvas")
                    .test_support()
                    .aria_label("Picture of saved links")
                    .w_full()
                    .min_w_0()
                    .h(height)
                    .overflow_hidden()
                    .rounded(t.radius.card)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(drawing),
            )
            .child(nodes)
    }

    fn reading_card(
        &self,
        selected: Option<&MemoryGraphNodeId>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let words = reading_words(selected, &self.state.page.facts);
        v_flex()
            .w_full()
            .min_w_0()
            .flex_1()
            .max_w(t.layout.reading_width)
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(div().sovra_text(TextRole::Caption, cx).child("The words"))
            .child(
                div()
                    .id("memory-graph-reading")
                    .test_support()
                    .aria_label(words.clone())
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Reading, cx)
                    .child(words),
            )
    }

    fn link_lines(&self, graph: &MemoryGraph, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut lines = v_flex().w_full().min_w_0().gap(t.space.hair);
        for (index, edge) in graph.edges.iter().enumerate() {
            let label = edge.kind.label();
            lines = lines.child(
                div()
                    .id(("memory-graph-link", index))
                    .test_support()
                    .aria_label(label.clone())
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child(label),
            );
        }
        lines
    }

    fn duplicate_check(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Look through the details on this page for the same words, and for wording that is close. Nothing is deleted."),
            )
            .child(
                design::button("memory-duplicate-check", "Check for repeats", cx)
                    .self_start()
                    .max_w_full()
                    .disabled(self.state.loading)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.duplicates = Some(check_memory_duplicates(&view.state.page.facts));
                        cx.notify();
                    })),
            );
        if let Some(report) = &self.duplicates {
            panel = panel
                .child(exact_block(report, cx))
                .child(near_block(self, report, cx));
        }
        panel
    }
}

fn selected_id(view: &MemoryView) -> Option<&MemoryGraphNodeId> {
    view.graph_selection.as_ref()
}

fn selected_matches(selected: Option<&MemoryGraphNodeId>, id: &MemoryGraphNodeId) -> bool {
    selected.is_some_and(|selected| selected == id)
}

fn graph_records(facts: &[Fact], chats: &[MemoryFactChat]) -> Vec<MemoryGraphRecord> {
    facts
        .iter()
        .map(|fact| MemoryGraphRecord {
            fact: fact.clone(),
            chat: chats.iter().find_map(|chat| {
                (chat.fact == fact.id && !chat.chat.trim().is_empty()).then(|| chat.chat.clone())
            }),
        })
        .collect()
}

fn reading_words(selected: Option<&MemoryGraphNodeId>, facts: &[Fact]) -> String {
    match selected {
        Some(MemoryGraphNodeId::Detail(id)) => {
            facts.iter().find(|fact| &fact.id == id).map_or_else(
                || "Select a detail in the picture to read it here.".into(),
                |fact| fact.content.clone(),
            )
        }
        Some(MemoryGraphNodeId::Chat(name)) => {
            format!("These details were saved from the chat {name}.")
        }
        None => "Select a detail in the picture to read it here.".into(),
    }
}

fn exact_block(
    report: &MemoryDuplicateReport,
    cx: &mut Context<MemoryView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut words = if report.exact.is_empty() {
        "No exact repeats in the details shown.".to_owned()
    } else {
        "Same wording, saved more than once. Both stay. The older one is an earlier version."
            .to_owned()
    };
    for pair in &report.exact {
        words.push(' ');
        words.push_str(&pair.older_words);
        words.push(' ');
        words.push_str(&pair.newer_words);
    }
    v_flex()
        .id("memory-duplicate-exact")
        .test_support()
        .aria_label(words.clone())
        .w_full()
        .min_w_0()
        .gap(t.space.hair)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .child(words),
        )
}

fn near_block(
    view: &MemoryView,
    report: &MemoryDuplicateReport,
    cx: &mut Context<MemoryView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let _ = view;
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.small);
    if report.near.is_empty() {
        return panel.child(
            div()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .child("No close wording is waiting for you."),
        );
    }
    panel = panel.child(
        div()
            .min_w_0()
            .whitespace_normal()
            .sovra_text(TextRole::Caption, cx)
            .child("These are close, but not the same words. Nothing is joined until you confirm."),
    );
    for (index, pair) in report.near.iter().enumerate() {
        let newer = pair.newer.clone();
        let older = pair.older.clone();
        let label = format!(
            "These are close, but not the same words. Nothing is joined until you confirm. {} {}",
            pair.older_words, pair.newer_words
        );
        panel = panel.child(
            v_flex()
                .id(("memory-duplicate-near", index))
                .test_support()
                .aria_label(label)
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .p(t.space.small)
                .sovra_surface(Surface::Matte, cx)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(pair.older_words.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(pair.newer_words.clone()),
                )
                .child(
                    design::button(
                        ("memory-duplicate-confirm", index),
                        "Treat as the same detail",
                        cx,
                    )
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(report) = &mut view.duplicates {
                            confirm_memory_duplicate(report, &newer, &older);
                        }
                        cx.notify();
                    })),
                ),
        );
    }
    panel
}

fn paint_graph(
    bounds: Bounds<Pixels>,
    graph: &MemoryGraph,
    selected: Option<&MemoryGraphNodeId>,
    ink: &GraphInk,
    diameter: Rems,
    hairline: Pixels,
    window: &mut Window,
) {
    window.paint_quad(fill(bounds, ink.canvas));
    if graph.nodes.is_empty() || graph.points.len() != graph.nodes.len() {
        return;
    }
    let diameter = diameter.to_pixels(window.rem_size());
    let pad = diameter;
    let zero = bounds.origin.x * 0.;
    let span_x = (bounds.size.width - pad * 2.).max(zero);
    let span_y = (bounds.size.height - pad * 2.).max(zero);
    let center = |spot: MemoryPoint| {
        point(
            bounds.origin.x + pad + span_x * thousandth(spot.x),
            bounds.origin.y + pad + span_y * thousandth(spot.y),
        )
    };
    for edge in &graph.edges {
        let Some(from) = node_point(graph, &edge.from) else {
            continue;
        };
        let Some(to) = node_point(graph, &edge.to) else {
            continue;
        };
        let mut path = PathBuilder::stroke(hairline);
        path.move_to(center(from));
        path.line_to(center(to));
        if let Ok(path) = path.build() {
            window.paint_path(path, edge_ink(ink, &edge.kind));
        }
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        let spot = center(graph.points[index]);
        let chosen = selected.is_some_and(|id| id == &node.id);
        let color = if chosen {
            ink.selected
        } else if matches!(node.id, MemoryGraphNodeId::Chat(_)) {
            ink.chat
        } else {
            ink.detail
        };
        let side = if chosen { diameter * 1.4 } else { diameter };
        window.paint_quad(
            fill(
                Bounds::new(
                    point(spot.x - side * 0.5, spot.y - side * 0.5),
                    size(side, side),
                ),
                color,
            )
            .corner_radii(side * 0.5),
        );
    }
}

fn node_point(graph: &MemoryGraph, id: &MemoryGraphNodeId) -> Option<MemoryPoint> {
    graph
        .nodes
        .iter()
        .position(|node| &node.id == id)
        .map(|index| graph.points[index])
}

fn edge_ink(ink: &GraphInk, kind: &MemoryGraphEdgeKind) -> Hsla {
    match kind {
        MemoryGraphEdgeKind::EarlierVersion => ink.version,
        MemoryGraphEdgeKind::SameMessage => ink.message,
        MemoryGraphEdgeKind::SharedSubject(_) => ink.subject,
        MemoryGraphEdgeKind::SavedFromChat => ink.chat,
    }
}

fn thousandth(value: i32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "layout coordinates stay inside 0..=1000"
    )]
    let value = value as f32;
    value / 1000.
}
