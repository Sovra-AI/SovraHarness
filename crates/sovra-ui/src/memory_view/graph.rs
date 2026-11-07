//! The knowledge map: an interactive force-directed picture of saved links.
//!
//! Hover a detail to light up what it is linked to, click to read it and walk to its
//! neighbours, drag a detail to move it, drag the background to pan, and pinch or
//! Ctrl/⌘ + scroll to zoom. The legend doubles as a filter. Links are only the ones
//! the domain stored or the person confirmed. Words stay in the inspector and in an
//! optional list; clickable labels also keep the map reachable by keyboard.

mod viewport;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, button::ButtonVariants as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Bounds, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    PathBuilder, Pixels, Point, ScrollDelta, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, canvas, div, fill, point, size,
};
use sovra_core::{
    Fact, MemoryFactChat, MemoryGraph, MemoryGraphEdgeKind, MemoryGraphNodeId, MemoryGraphRecord,
    MemoryPoint, layout_memory_graph, memory_graph,
};

use super::MemoryView;
use crate::design::{self, GraphTokens, SovraStyled as _, Surface, TextRole};
use viewport::{Frame, Viewport};

/// How many of the best-linked details keep their words visible at normal zoom.
const HUB_LABELS: usize = 3;

/// The four kinds of link, in legend order. Each can be hidden from the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    Subject,
    Message,
    Version,
    Chat,
}

impl LinkKind {
    const ALL: [Self; 4] = [Self::Subject, Self::Message, Self::Version, Self::Chat];

    fn of(kind: &MemoryGraphEdgeKind) -> Self {
        match kind {
            MemoryGraphEdgeKind::SharedSubject(_) => Self::Subject,
            MemoryGraphEdgeKind::SameMessage => Self::Message,
            MemoryGraphEdgeKind::EarlierVersion => Self::Version,
            MemoryGraphEdgeKind::SavedFromChat => Self::Chat,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Subject => 0,
            Self::Message => 1,
            Self::Version => 2,
            Self::Chat => 3,
        }
    }

    fn legend(self) -> &'static str {
        match self {
            Self::Subject => "Same subject",
            Self::Message => "Same message",
            Self::Version => "Earlier version",
            Self::Chat => "From a chat",
        }
    }

    fn ink(self, graph: &GraphTokens) -> Hsla {
        match self {
            Self::Subject => graph.subject,
            Self::Message => graph.message,
            Self::Version => graph.version,
            Self::Chat => graph.saved_from,
        }
    }

    /// Links of different kinds between the same pair bow apart instead of overlapping.
    fn bend(self) -> f32 {
        match self {
            Self::Subject => 0.07,
            Self::Message => -0.07,
            Self::Version => 0.14,
            Self::Chat => 0.0,
        }
    }
}

enum Drag {
    Pan { last: Point<Pixels>, moved: bool },
    Node { id: MemoryGraphNodeId },
}

struct CachedLayout {
    nodes: Vec<MemoryGraphNodeId>,
    links: Vec<(MemoryGraphNodeId, MemoryGraphNodeId)>,
    aspect: u16,
    points: Vec<MemoryPoint>,
}

/// Map interaction state owned by the memory view. Layouts are cached because the
/// view re-renders on every pointer move while hovering or dragging.
#[derive(Default)]
pub(super) struct GraphState {
    frame: Option<Frame>,
    view: Viewport,
    hovered: Option<MemoryGraphNodeId>,
    hovered_link: Option<usize>,
    drag: Option<Drag>,
    /// Details the person dragged, in layout thousandths. They stay where they were put.
    pinned: HashMap<MemoryGraphNodeId, (f32, f32)>,
    hidden: [bool; 4],
    list_open: bool,
    focused: bool,
    layout: RefCell<Option<CachedLayout>>,
}

/// Everything the paint pass needs, computed once per render.
#[derive(Clone)]
struct Scene {
    spots: Vec<(f32, f32)>,
    nodes: Vec<SceneNode>,
    links: Vec<SceneLink>,
    view: Viewport,
    pad: Pixels,
    grid: Pixels,
    ink: GraphTokens,
}

#[derive(Clone)]
struct SceneNode {
    shown: bool,
    chat: bool,
    diameter: Pixels,
    ink: Hsla,
    alpha: f32,
    emphasis: Emphasis,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Emphasis {
    Plain,
    Hovered,
    Selected,
}

#[derive(Clone)]
struct SceneLink {
    from: usize,
    to: usize,
    ink: Hsla,
    width: Pixels,
    alpha: f32,
    bend: f32,
}

impl MemoryView {
    pub(super) fn memory_picture(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
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
                    .child("Your knowledge map"),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child("See how your saved details fit together. Choose a detail to read it and discover what connects to it."),
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
                    .p(t.space.large)
                    .rounded(t.radius.card)
                    .bg(t.graph.backdrop)
                    .border_1()
                    .border_color(cx.theme().border)
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
        let overview = memory_graph(&records, versions);
        let graph = if self.graph.focused {
            self.graph_selection
                .as_ref()
                .map_or_else(|| overview.clone(), |id| overview.around(id))
        } else {
            overview
        };
        let selected = self
            .graph_selection
            .clone()
            .filter(|id| graph.nodes.iter().any(|node| &node.id == id));
        column = column
            .child(self.graph_toolbar(&graph, cx))
            .child(self.graph_body(&graph, selected.as_ref(), window, cx));
        if self.graph.focused {
            column = column.child(div()
                .id("memory-graph-focus-status")
                .test_support()
                .aria_label("Showing one detail and its direct connections. Choose Show all to return to the whole map.")
                .sovra_text(TextRole::Caption, cx)
                .whitespace_normal()
                .child("Showing one detail and its direct connections. Choose Show all to return to the whole map."));
        }
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
        if self.graph.list_open {
            column = column.child(Self::graph_list(&graph, selected.as_ref(), cx));
        }
        column.child(self.duplicate_check(cx))
    }

    /// Legend chips that also filter, then view controls.
    fn graph_toolbar(
        &self,
        graph: &MemoryGraph,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut counts = [0_usize; 4];
        for edge in &graph.edges {
            counts[LinkKind::of(&edge.kind).index()] += 1;
        }
        let mut legend = h_flex()
            .id("memory-graph-legend")
            .test_support()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.compact);
        for kind in LinkKind::ALL {
            let index = kind.index();
            let shown = !self.graph.hidden[index];
            let label = format!(
                "{} · {} · {}",
                kind.legend(),
                counts[index],
                if shown { "shown" } else { "hidden" }
            );
            let swatch = div()
                .flex_shrink_0()
                .size(t.graph.legend_swatch)
                .bg(kind.ink(&t.graph))
                .when_chat(kind == LinkKind::Chat, t.radius.small, t.radius.pill);
            legend = legend.child(
                design::button(("memory-graph-filter", index), "", cx)
                    .accessibility_label(label)
                    .small()
                    .selected(shown)
                    .rounded(t.radius.pill)
                    .px(t.space.small)
                    .gap(t.space.snug)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.graph.hidden[index] = !view.graph.hidden[index];
                        view.graph.hovered_link = None;
                        cx.notify();
                    }))
                    .child(swatch)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .sovra_text(TextRole::Caption, cx)
                            .child(format!("{} {}", kind.legend(), counts[index])),
                    ),
            );
        }
        legend
    }

    /// Keep controls outside the drawing so they cannot cover a detail.
    fn graph_controls(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let zoom = |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
            design::button(id, label, cx).small().ghost()
        };
        // Controls wrap inside narrow panes and at enlarged text sizes.
        let list_open = self.graph.list_open;
        let pill = h_flex()
            .max_w_full()
            .flex_wrap()
            .justify_end()
            .gap(t.space.hair)
            .p(t.space.hair)
            .rounded(t.radius.button)
            .bg(t.elevated)
            .border_1()
            .border_color(t.material_border)
            .shadow(vec![t.button_shadow])
            .child(
                zoom("memory-graph-zoom-out", "−", cx)
                    .accessibility_label("Zoom out")
                    .tooltip("Zoom out")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.zoom_map(1. / design::tokens(cx).graph.zoom_step, cx);
                    })),
            )
            .child(
                zoom("memory-graph-zoom-in", "+", cx)
                    .accessibility_label("Zoom in")
                    .tooltip("Zoom in")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.zoom_map(design::tokens(cx).graph.zoom_step, cx);
                    })),
            )
            .child(
                zoom("memory-graph-fit", "Show all", cx)
                    .tooltip("Show the whole map again")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.graph.view = Viewport::default();
                        view.graph.pinned.clear();
                        view.graph.focused = false;
                        view.graph.hovered = None;
                        view.graph.hovered_link = None;
                        cx.notify();
                    })),
            )
            .child(
                zoom("memory-graph-list-toggle", "Read as a list", cx)
                    .tooltip("Read the map as a list")
                    .selected(list_open)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.graph.list_open = !view.graph.list_open;
                        cx.notify();
                    })),
            );
        h_flex()
            .id("memory-graph-controls")
            .test_support()
            .w_full()
            .min_w_0()
            .justify_end()
            .cursor_default()
            .child(pill)
    }

    /// A saved card under the pointer lights up its detail on the map.
    pub(super) fn preview_on_map(
        &mut self,
        node: &MemoryGraphNodeId,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.graph.hovered = Some(node.clone());
            cx.notify();
        } else if self.graph.hovered.as_ref() == Some(node) {
            self.graph.hovered = None;
            cx.notify();
        }
    }

    fn zoom_map(&mut self, factor: f32, cx: &mut Context<Self>) {
        let limits = zoom_limits(cx);
        if let Some(frame) = self.graph.frame {
            self.graph.view =
                frame.zoom_about(self.graph.view, frame.bounds.center(), factor, limits);
        } else {
            self.graph.view.zoom = (self.graph.view.zoom * factor).clamp(limits.0, limits.1);
        }
        cx.notify();
    }

    fn graph_body(
        &self,
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let t = design::tokens(cx);
        let map = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(self.graph_controls(cx))
            .child(self.graph_canvas(graph, selected, window, cx));
        let inspector = self.graph_inspector(graph, selected, cx);
        if self.width_class.fits_detail_pane() {
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(div().flex_1().min_w_0().child(map))
                .child(
                    div()
                        .flex_shrink_0()
                        .w(t.layout.inspector_width)
                        .max_w_full()
                        .child(inspector),
                )
                .into_any_element()
        } else {
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .child(map)
                .child(inspector)
                .into_any_element()
        }
    }

    /// Layout for this graph and pane shape, computed once and reused while the
    /// pointer moves. Dragged details override their computed spot.
    fn graph_spots(&self, graph: &MemoryGraph, aspect: u16) -> Vec<(f32, f32)> {
        let nodes: Vec<_> = graph.nodes.iter().map(|node| node.id.clone()).collect();
        let links: Vec<_> = graph
            .edges
            .iter()
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect();
        let mut cache = self.graph.layout.borrow_mut();
        let fresh = cache.as_ref().is_some_and(|cached| {
            cached.aspect == aspect && cached.nodes == nodes && cached.links == links
        });
        if !fresh {
            *cache = Some(CachedLayout {
                points: layout_memory_graph(&graph.nodes, &graph.edges, aspect),
                nodes,
                links,
                aspect,
            });
        }
        let points = cache
            .as_ref()
            .map_or(&graph.points, |cached| &cached.points);
        graph
            .nodes
            .iter()
            .zip(points)
            .map(|(node, spot)| {
                self.graph
                    .pinned
                    .get(&node.id)
                    .copied()
                    .unwrap_or((thousandth(spot.x), thousandth(spot.y)))
            })
            .collect()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one drawing: scene, hit targets, labels and link markers share the same geometry"
    )]
    fn graph_canvas(
        &self,
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let ink = t.graph;
        let rem = window.rem_size();
        // A pane too narrow for a side column also gets the shorter drawing,
        // so the words under it stay within a scroll of the fold.
        let height = if self.width_class.fits_detail_pane() {
            ink.height
        } else {
            ink.compact_height
        };
        let pad = ink.node_max.to_pixels(rem);
        let aspect = self
            .graph
            .frame
            .map_or(sovra_core::MEMORY_GRAPH_SQUARE, |frame| {
                frame.aspect_percent()
            });
        let spots = self.graph_spots(graph, aspect);
        let shown_kind =
            |kind: &MemoryGraphEdgeKind| !self.graph.hidden[LinkKind::of(kind).index()];
        let chats_shown = !self.graph.hidden[LinkKind::Chat.index()];
        let node_shown: Vec<bool> = graph
            .nodes
            .iter()
            .map(|node| chats_shown || !matches!(node.id, MemoryGraphNodeId::Chat(_)))
            .collect();
        let index_of = |id: &MemoryGraphNodeId| graph.nodes.iter().position(|node| &node.id == id);
        let visible_links: Vec<(usize, usize, usize)> = graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| shown_kind(&edge.kind))
            .filter_map(|(index, edge)| Some((index, index_of(&edge.from)?, index_of(&edge.to)?)))
            .filter(|(_, from, to)| node_shown[*from] && node_shown[*to])
            .collect();
        let mut degree = vec![0_usize; graph.nodes.len()];
        for (_, from, to) in &visible_links {
            degree[*from] += 1;
            degree[*to] += 1;
        }
        let focus = self.graph.hovered.as_ref().or(selected).and_then(index_of);
        let hovered_link = self
            .graph
            .hovered_link
            .filter(|link| visible_links.iter().any(|(index, ..)| index == link));
        let mut lit: HashSet<usize> = HashSet::new();
        if let Some(focus) = focus {
            lit.insert(focus);
            for (_, from, to) in &visible_links {
                if *from == focus {
                    lit.insert(*to);
                } else if *to == focus {
                    lit.insert(*from);
                }
            }
        }
        if let Some(link) = hovered_link
            && let Some((_, from, to)) = visible_links.iter().find(|(index, ..)| *index == link)
        {
            lit.insert(*from);
            lit.insert(*to);
        }
        let dimming = !lit.is_empty();
        let busiest = degree.iter().copied().max().unwrap_or(0).max(1);
        let mut ranked: Vec<usize> = (0..graph.nodes.len()).filter(|i| degree[*i] >= 2).collect();
        ranked.sort_by_key(|i| std::cmp::Reverse(degree[*i]));
        ranked.truncate(HUB_LABELS);
        let selected_index = selected.and_then(index_of);
        let hovered_index = self.graph.hovered.as_ref().and_then(index_of);
        let small = ink.node.to_pixels(rem);
        let large = ink.node_max.to_pixels(rem);
        let nodes: Vec<SceneNode> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let chat = matches!(node.id, MemoryGraphNodeId::Chat(_));
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "link counts are bounded by the picture's few dozen links"
                )]
                let weight = (degree[index] as f32 / busiest as f32).sqrt();
                SceneNode {
                    shown: node_shown[index],
                    chat,
                    diameter: if chat {
                        ink.chat_node.to_pixels(rem)
                    } else {
                        small + (large - small) * weight
                    },
                    ink: if chat { ink.chat } else { ink.detail },
                    alpha: if !dimming || lit.contains(&index) {
                        1.0
                    } else {
                        ink.faded
                    },
                    emphasis: if selected_index == Some(index) {
                        Emphasis::Selected
                    } else if hovered_index == Some(index) {
                        Emphasis::Hovered
                    } else {
                        Emphasis::Plain
                    },
                }
            })
            .collect();
        let links: Vec<SceneLink> = visible_links
            .iter()
            .map(|(index, from, to)| {
                let kind = LinkKind::of(&graph.edges[*index].kind);
                let touches = focus.is_some_and(|focus| *from == focus || *to == focus)
                    || hovered_link == Some(*index);
                SceneLink {
                    from: *from,
                    to: *to,
                    ink: kind.ink(&ink),
                    width: if touches { ink.line_focus } else { ink.line },
                    alpha: if !dimming || touches {
                        0.9
                    } else {
                        ink.faded * 0.7
                    },
                    bend: kind.bend(),
                }
            })
            .collect();
        let scene = Scene {
            spots: spots.clone(),
            nodes: nodes.clone(),
            links: links.clone(),
            view: self.graph.view,
            pad,
            grid: ink.hit.to_pixels(rem),
            ink,
        };
        let entity = cx.weak_entity();
        let drawing = canvas(
            move |bounds, _, cx| {
                let frame = Frame { bounds, pad };
                entity
                    .update(cx, |view, cx| {
                        if view.graph.frame != Some(frame) {
                            view.graph.frame = Some(frame);
                            cx.notify();
                        }
                    })
                    .ok();
            },
            move |bounds, (), window, _| paint_map(bounds, &scene, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut area = div()
            .id("memory-graph-canvas")
            .test_support()
            .aria_label("Picture of saved links")
            .relative()
            .w_full()
            .min_w_0()
            .h(height)
            .overflow_hidden()
            .rounded(t.radius.card)
            .border_1()
            .border_color(cx.theme().border)
            .bg(ink.backdrop)
            .cursor_grab()
            .child(drawing)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, cx| {
                    view.graph.drag = Some(Drag::Pan {
                        last: event.position,
                        moved: false,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    view.drag_map(event.position, event.pressed_button.is_some(), cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.end_drag(cx)),
            )
            .on_scroll_wheel(
                cx.listener(|view, event: &gpui_kit::ScrollWheelEvent, window, cx| {
                    if !(event.modifiers.control || event.modifiers.platform) {
                        return;
                    }
                    // Trackpads report pixels; treat one interface line of travel as one notch.
                    let lines = match event.delta {
                        ScrollDelta::Pixels(delta) => delta.y / window.line_height(),
                        ScrollDelta::Lines(delta) => delta.y,
                    };
                    view.zoom_at(
                        event.position,
                        design::tokens(cx).graph.zoom_step.powf(lines),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
            .on_pinch(cx.listener(|view, event: &gpui_kit::PinchEvent, _, cx| {
                view.zoom_at(event.position, 1. + event.delta, cx);
                cx.stop_propagation();
            }));

        let Some(frame) = self.graph.frame else {
            return area;
        };
        let origin = frame.bounds.origin;
        let screen: Vec<Point<Pixels>> = spots
            .iter()
            .map(|&(x, y)| frame.to_screen(self.graph.view, x, y))
            .collect();
        // Never let two pointer targets overlap, so a click reaches the detail drawn there.
        let mut closest = ink.hit.to_pixels(rem);
        for a in 0..screen.len() {
            for b in (a + 1)..screen.len() {
                if node_shown[a] && node_shown[b] {
                    let gap = (screen[a].x - screen[b].x)
                        .abs()
                        .max((screen[a].y - screen[b].y).abs());
                    closest = closest.min(gap * 0.95);
                }
            }
        }
        let hit = closest.max(small);
        let label_width = if self.graph.focused {
            ink.focused_label_width
        } else {
            ink.label_width
        }
        .to_pixels(rem);
        let marker = ink.link_marker.to_pixels(rem);

        // Link markers sit under the nodes so a node always wins the pointer.
        for (index, from, to) in &visible_links {
            let kind = LinkKind::of(&graph.edges[*index].kind);
            let mid = curve_middle(screen[*from], screen[*to], kind.bend());
            let label = graph.edges[*index].kind.label();
            let link = *index;
            let active = hovered_link == Some(link);
            let size_now = if active { marker * 1.6 } else { marker };
            area = area.child(
                div()
                    .id(("memory-graph-link", *index))
                    .test_support()
                    .aria_label(label)
                    .absolute()
                    .left(mid.x - origin.x - size_now / 2.)
                    .top(mid.y - origin.y - size_now / 2.)
                    .size(size_now)
                    .rounded(t.radius.pill)
                    .bg(kind.ink(&ink))
                    .border_1()
                    .border_color(ink.backdrop)
                    .opacity(
                        if !dimming || active || links_touch(&links, *from, *to, &lit) {
                            1.0
                        } else {
                            ink.faded
                        },
                    )
                    .cursor_pointer()
                    .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                        let next = if *hovered { Some(link) } else { None };
                        if *hovered || view.graph.hovered_link == Some(link) {
                            view.graph.hovered_link = next;
                            cx.notify();
                        }
                    })),
            );
        }

        // Words go to the focus first, then its neighbours, chats and the best-linked
        // details. A label that would cover one already placed is left for zoom or hover.
        let rank = |index: usize| {
            if selected_index == Some(index) || hovered_index == Some(index) {
                0
            } else if lit.contains(&index) {
                1
            } else if nodes[index].chat {
                2
            } else if ranked.contains(&index) {
                3
            } else {
                4
            }
        };
        let mut order: Vec<usize> = (0..graph.nodes.len())
            .filter(|index| node_shown[*index])
            .collect();
        order.sort_by_key(|index| rank(*index));
        let base_line = t.layout.compact_control_height.to_pixels(rem);
        let mut placed: Vec<Bounds<Pixels>> = Vec::new();
        for index in order {
            let node = &graph.nodes[index];
            let spot = screen[index];
            if !frame.bounds.contains(&spot) {
                continue;
            }
            #[expect(
                clippy::cast_precision_loss,
                reason = "labels are clipped to a few dozen characters"
            )]
            let chars = node.label.chars().count() as f32;
            let estimate = (rem * 0.55 * chars + t.space.snug.to_pixels(rem) * 2.)
                .min(label_width)
                .min(frame.bounds.size.width);
            let line = if self.graph.focused {
                base_line * (rem * 0.5 * chars / estimate).ceil().clamp(1.0, 3.0)
            } else {
                base_line
            };
            let gap = nodes[index].diameter / 2. + t.space.compact.to_pixels(rem);
            let candidates = [
                point(spot.x - estimate / 2., spot.y + gap),
                point(spot.x - estimate / 2., spot.y - gap - line),
                point(spot.x + gap, spot.y - line / 2.),
                point(spot.x - gap - estimate, spot.y - line / 2.),
            ];
            let focus_label = rank(index) == 0;
            let rect = candidates
                .into_iter()
                .map(|position| {
                    let x = position.x.clamp(
                        frame.bounds.left(),
                        (frame.bounds.right() - estimate).max(frame.bounds.left()),
                    );
                    let y = position.y.clamp(
                        frame.bounds.top(),
                        (frame.bounds.bottom() - line).max(frame.bounds.top()),
                    );
                    Bounds::new(point(x, y), size(estimate, line))
                })
                .find(|rect| {
                    !placed.iter().any(|other| other.intersects(rect))
                        && !screen.iter().enumerate().any(|(other, centre)| {
                            other != index
                                && node_shown[other]
                                && rect.intersects(&Bounds::new(
                                    point(centre.x - hit / 2., centre.y - hit / 2.),
                                    size(hit, hit),
                                ))
                        })
                });
            let Some(rect) = rect else { continue };
            placed.push(rect);
            let target = node.id.clone();
            area = area.child(
                design::button(
                    ("memory-graph-label", index),
                    if self.graph.focused {
                        String::new()
                    } else {
                        node.label.clone()
                    },
                    cx,
                )
                .small()
                .ghost()
                .bg(ink.label)
                .border_1()
                .border_color(t.material_border)
                .accessibility_label(node.label.clone())
                .absolute()
                .left(rect.left() - origin.x)
                .top(rect.top() - origin.y)
                .w(rect.size.width)
                .h(line)
                .min_h(t.layout.compact_control_height)
                .px(t.space.snug)
                .rounded(t.radius.small)
                .selected(focus_label)
                .tooltip(node.label.clone())
                .sovra_text(TextRole::Caption, cx)
                .when(self.graph.focused, |button| {
                    button.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .sovra_text(TextRole::Body, cx)
                            .child(node.label.clone()),
                    )
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.graph_selection = Some(target.clone());
                    view.graph.hovered = None;
                    cx.notify();
                })),
            );
        }

        for (index, node) in graph.nodes.iter().enumerate() {
            if !node_shown[index] {
                continue;
            }
            let spot = screen[index];
            let id = node.id.clone();
            let hover_id = node.id.clone();
            area = area.child(
                div()
                    .id(("memory-graph-node", index))
                    .test_support()
                    .aria_label(node.label.clone())
                    .absolute()
                    .left(spot.x - origin.x - hit / 2.)
                    .top(spot.y - origin.y - hit / 2.)
                    .size(hit)
                    .rounded(t.radius.pill)
                    .cursor_pointer()
                    .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                        if *hovered {
                            view.graph.hovered = Some(hover_id.clone());
                            cx.notify();
                        } else if view.graph.hovered.as_ref() == Some(&hover_id) {
                            view.graph.hovered = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.graph_selection = Some(id.clone());
                            view.graph.drag = Some(Drag::Node { id: id.clone() });
                            cx.notify();
                        }),
                    ),
            );
        }

        if let Some(link) = hovered_link
            && let Some((_, from, to)) = visible_links.iter().find(|(index, ..)| *index == link)
        {
            let kind = LinkKind::of(&graph.edges[link].kind);
            let mid = curve_middle(screen[*from], screen[*to], kind.bend());
            area = area.child(
                div()
                    .id("memory-graph-link-tip")
                    .test_support()
                    .absolute()
                    .left(mid.x - origin.x - label_width / 2.)
                    .top(mid.y - origin.y + marker)
                    .w(label_width)
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .max_w_full()
                            .whitespace_normal()
                            .px(t.space.small)
                            .py(t.space.compact)
                            .rounded(t.radius.small)
                            .bg(t.elevated)
                            .border_1()
                            .border_color(kind.ink(&ink))
                            .shadow(vec![t.elevated_shadow])
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(cx.theme().foreground)
                            .child(graph.edges[link].kind.label()),
                    ),
            );
        }
        area
    }

    fn drag_map(&mut self, at: Point<Pixels>, pressed: bool, cx: &mut Context<Self>) {
        if !pressed {
            if self.graph.drag.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some(frame) = self.graph.frame else {
            return;
        };
        match &mut self.graph.drag {
            Some(Drag::Pan { last, moved }) => {
                let delta = point(at.x - last.x, at.y - last.y);
                if delta.x == Pixels::ZERO && delta.y == Pixels::ZERO {
                    return;
                }
                *last = at;
                *moved = true;
                self.graph.view.pan = point(
                    self.graph.view.pan.x + delta.x,
                    self.graph.view.pan.y + delta.y,
                );
                cx.notify();
            }
            Some(Drag::Node { id }) => {
                let spot = frame.to_layout(self.graph.view, at);
                self.graph.pinned.insert(id.clone(), spot);
                cx.notify();
            }
            None => {}
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        // A click on empty space, rather than a pan, puts the selection down.
        if let Some(Drag::Pan { moved: false, .. }) = self.graph.drag {
            self.graph_selection = None;
            self.graph.focused = false;
        }
        if self.graph.drag.take().is_some() {
            cx.notify();
        }
    }

    fn zoom_at(&mut self, anchor: Point<Pixels>, factor: f32, cx: &mut Context<Self>) {
        let Some(frame) = self.graph.frame else {
            return;
        };
        if !factor.is_finite() || factor <= 0. {
            return;
        }
        self.graph.view = frame.zoom_about(self.graph.view, anchor, factor, zoom_limits(cx));
        cx.notify();
    }

    /// The words for the focus, its links as a walkable list, and a way into the detail.
    fn graph_inspector(
        &self,
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let words = reading_words(selected, &self.state.page.facts, &self.state.page.chats);
        let heading = match selected {
            Some(MemoryGraphNodeId::Detail(_)) => "Saved detail",
            Some(MemoryGraphNodeId::Chat(_)) => "Chat",
            None => "Start exploring",
        };
        let mut panel = v_flex()
            .id("memory-graph-inspector")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(div().sovra_text(TextRole::Caption, cx).child(heading))
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
            );
        let Some(selected) = selected else {
            let chats = graph
                .nodes
                .iter()
                .filter(|node| matches!(node.id, MemoryGraphNodeId::Chat(_)))
                .count();
            let summary = format!(
                "{} details · {} links · {} chats",
                graph.nodes.len() - chats,
                graph.edges.len(),
                chats
            );
            return panel
                .child(
                    div()
                        .id("memory-graph-summary")
                        .test_support()
                        .aria_label(summary.clone())
                        .sovra_text(TextRole::Caption, cx)
                        .child(summary),
                )
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child("Point at a detail to see what it links to. Click to read it here and follow its links. Drag to move around; pinch or hold Ctrl and scroll to zoom."),
                );
        };
        if !self.graph.focused {
            panel = panel.child(
                design::button("memory-graph-focus", "Focus on these connections", cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        view.graph.focused = true;
                        view.graph.view = Viewport::default();
                        view.graph.pinned.clear();
                        view.graph.hovered = None;
                        view.graph.hovered_link = None;
                        cx.notify();
                    }),
                ),
            );
        }
        let Some(focus) = graph.nodes.iter().position(|node| &node.id == selected) else {
            return panel;
        };
        if let MemoryGraphNodeId::Detail(id) = selected
            && let Some(fact) = self.state.page.facts.iter().find(|fact| &fact.id == id)
        {
            panel = panel.child(self.subject_buttons(fact, "memory-graph-subject", cx));
        }
        // One row per neighbour, listing every reason it is linked.
        let mut neighbours: Vec<(usize, Vec<LinkKind>, Vec<String>)> = Vec::new();
        for edge in &graph.edges {
            let other = if edge.from == *selected {
                &edge.to
            } else if edge.to == *selected {
                &edge.from
            } else {
                continue;
            };
            let kind = LinkKind::of(&edge.kind);
            if self.graph.hidden[kind.index()] {
                continue;
            }
            let Some(node) = graph.nodes.iter().position(|node| &node.id == other) else {
                continue;
            };
            match neighbours.iter_mut().find(|(index, ..)| *index == node) {
                Some((_, kinds, reasons)) => {
                    kinds.push(kind);
                    reasons.push(edge.kind.label());
                }
                None => neighbours.push((node, vec![kind], vec![edge.kind.label()])),
            }
        }
        let mut walk = v_flex().w_full().min_w_0().gap(t.space.hair);
        for (row, (node, kinds, reasons)) in neighbours.iter().enumerate() {
            let target = graph.nodes[*node].id.clone();
            let reasons = reasons.join(" · ");
            let mut swatches = h_flex().flex_shrink_0().gap(t.space.hair);
            for kind in kinds {
                swatches = swatches.child(
                    div()
                        .size(t.graph.legend_swatch)
                        .bg(kind.ink(&t.graph))
                        .when_chat(*kind == LinkKind::Chat, t.radius.small, t.radius.pill),
                );
            }
            walk = walk.child(
                design::button(("memory-graph-neighbor", row), "", cx)
                    .ghost()
                    .w_full()
                    .h_auto()
                    .py(t.space.snug)
                    .px(t.space.small)
                    .accessibility_label(format!("{}: {}", graph.nodes[*node].label, reasons))
                    .tooltip(reasons.clone())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.graph_selection = Some(target.clone());
                        view.graph.hovered = None;
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap(t.space.small)
                            .child(swatches)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_start()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .sovra_text(TextRole::Body, cx)
                                            .child(graph.nodes[*node].label.clone()),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .sovra_text(TextRole::Caption, cx)
                                            .text_color(t.text_secondary)
                                            .child(reasons),
                                    ),
                            ),
                    ),
            );
        }
        let found = neighbours.len();
        panel = panel.child(div().sovra_text(TextRole::Caption, cx).child(match found {
            0 => "Not linked to anything else yet.".to_owned(),
            1 => "Linked to 1 thing".to_owned(),
            count => format!("Linked to {count} things"),
        }));
        if found > 0 {
            panel = panel.child(walk);
        }
        if let MemoryGraphNodeId::Detail(id) = &graph.nodes[focus].id {
            let id = id.clone();
            panel = panel.child(
                design::primary_button("memory-graph-open", "Explore this detail", cx)
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(move |view, _, _, cx| view.inspect(id.clone(), cx))),
            );
        }
        panel
    }

    /// The whole map as words: every detail is reachable by keyboard and screen reader.
    fn graph_list(
        graph: &MemoryGraph,
        selected: Option<&MemoryGraphNodeId>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut nodes = v_flex().w_full().min_w_0().gap(t.space.compact);
        for (index, node) in graph.nodes.iter().enumerate() {
            let id = node.id.clone();
            nodes = nodes.child(
                design::button(("memory-graph-list-node", index), node.label.clone(), cx)
                    .w_full()
                    .max_w_full()
                    .small()
                    .selected(selected == Some(&node.id))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.graph_selection = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        let mut lines = v_flex().w_full().min_w_0().gap(t.space.hair);
        for (index, edge) in graph.edges.iter().enumerate() {
            let name = |id: &MemoryGraphNodeId| {
                graph
                    .nodes
                    .iter()
                    .find(|node| &node.id == id)
                    .map_or_else(String::new, |node| node.label.clone())
            };
            let words = format!(
                "{}: {} — {}",
                edge.kind.label(),
                name(&edge.from),
                name(&edge.to)
            );
            lines = lines.child(
                div()
                    .id(("memory-graph-list-link", index))
                    .test_support()
                    .aria_label(words.clone())
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child(words),
            );
        }
        v_flex()
            .id("memory-graph-list")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Everything in the map"),
            )
            .child(nodes)
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("How they link"),
            )
            .child(lines)
    }
}

/// Square swatches mark chats so the legend does not rely on colour alone.
trait ChatShape: Styled + Sized {
    fn when_chat(self, chat: bool, square: Pixels, round: Pixels) -> Self {
        self.rounded(if chat { square } else { round })
    }
}

impl<T: Styled> ChatShape for T {}

fn zoom_limits(cx: &Context<MemoryView>) -> (f32, f32) {
    let graph = design::tokens(cx).graph;
    (graph.zoom_min, graph.zoom_max)
}

fn links_touch(links: &[SceneLink], from: usize, to: usize, lit: &HashSet<usize>) -> bool {
    lit.contains(&from)
        && lit.contains(&to)
        && links.iter().any(|link| link.from == from && link.to == to)
}

/// The point halfway along a link's bow, where its marker sits.
fn curve_middle(from: Point<Pixels>, to: Point<Pixels>, bend: f32) -> Point<Pixels> {
    let control = control_point(from, to, bend);
    point(
        (from.x + to.x) * 0.25 + control.x * 0.5,
        (from.y + to.y) * 0.25 + control.y * 0.5,
    )
}

fn control_point(from: Point<Pixels>, to: Point<Pixels>, bend: f32) -> Point<Pixels> {
    let mid = point((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
    // Perpendicular to the link, scaled by its length.
    point(
        mid.x - (to.y - from.y) * bend,
        mid.y + (to.x - from.x) * bend,
    )
}

fn graph_records(facts: &[Fact], chats: &[MemoryFactChat]) -> Vec<MemoryGraphRecord> {
    facts
        .iter()
        .map(|fact| {
            let chat = chats.iter().find(|chat| chat.fact == fact.id);
            MemoryGraphRecord {
                fact: fact.clone(),
                session: chat
                    .map(|chat| chat.session.trim())
                    .filter(|session| !session.is_empty())
                    .map(str::to_owned),
                title: chat
                    .map(|chat| chat.title.trim())
                    .filter(|title| !title.is_empty())
                    .map(str::to_owned),
            }
        })
        .collect()
}

fn reading_words(
    selected: Option<&MemoryGraphNodeId>,
    facts: &[Fact],
    chats: &[MemoryFactChat],
) -> String {
    match selected {
        Some(MemoryGraphNodeId::Detail(id)) => {
            facts.iter().find(|fact| &fact.id == id).map_or_else(
                || "Choose any detail to see its story and connections.".into(),
                |fact| fact.content.clone(),
            )
        }
        Some(MemoryGraphNodeId::Chat(session)) => {
            let title = chats
                .iter()
                .find(|chat| chat.session.trim() == session.trim())
                .map(|chat| chat.title.trim())
                .filter(|title| !title.is_empty());
            match title {
                Some(title) => format!("These details were saved from the chat {title}."),
                None => "These details were saved from this chat.".into(),
            }
        }
        None => "Choose any detail to see its story and connections.".into(),
    }
}

fn paint_map(bounds: Bounds<Pixels>, scene: &Scene, window: &mut Window) {
    let frame = Frame {
        bounds,
        pad: scene.pad,
    };
    let ink = &scene.ink;
    paint_grid(bounds, scene, window);
    if scene.spots.len() != scene.nodes.len() {
        return;
    }
    let screen: Vec<Point<Pixels>> = scene
        .spots
        .iter()
        .map(|&(x, y)| frame.to_screen(scene.view, x, y))
        .collect();
    for link in &scene.links {
        let (from, to) = (screen[link.from], screen[link.to]);
        let mut path = PathBuilder::stroke(link.width);
        path.move_to(from);
        path.curve_to(to, control_point(from, to, link.bend));
        if let Ok(path) = path.build() {
            window.paint_path(path, link.ink.alpha(link.alpha));
        }
    }
    for (node, spot) in scene.nodes.iter().zip(&screen) {
        if !node.shown {
            continue;
        }
        let radius = |side: Pixels| if node.chat { side * 0.3 } else { side * 0.5 };
        let square = |side: Pixels| {
            Bounds::new(
                point(spot.x - side * 0.5, spot.y - side * 0.5),
                size(side, side),
            )
        };
        if node.emphasis != Emphasis::Plain {
            let halo = node.diameter * 2.2;
            window.paint_quad(fill(square(halo), ink.halo).corner_radii(radius(halo)));
        }
        let body = node.diameter;
        let mut quad = fill(square(body), node.ink.alpha(node.alpha)).corner_radii(radius(body));
        if node.emphasis == Emphasis::Selected {
            quad = quad.border_widths(ink.ring * 1.5).border_color(ink.focus);
        } else {
            quad = quad
                .border_widths(ink.ring * 0.5)
                .border_color(ink.backdrop.alpha(node.alpha));
        }
        window.paint_quad(quad);
    }
}

/// A quiet dot grid that moves with the map, so panning and zooming read as motion.
fn paint_grid(bounds: Bounds<Pixels>, scene: &Scene, window: &mut Window) {
    let step = scene.grid * scene.view.zoom.max(0.5);
    if step <= Pixels::ZERO {
        return;
    }
    let dot = scene.ink.ring;
    let shift = |pan: Pixels| {
        let rem = pan % step;
        if rem < Pixels::ZERO { rem + step } else { rem }
    };
    let mut y = bounds.origin.y + shift(scene.view.pan.y);
    while y < bounds.bottom() {
        let mut x = bounds.origin.x + shift(scene.view.pan.x);
        while x < bounds.right() {
            window.paint_quad(
                fill(Bounds::new(point(x, y), size(dot, dot)), scene.ink.grid)
                    .corner_radii(dot * 0.5),
            );
            x += step;
        }
        y += step;
    }
}

fn thousandth(value: i32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "layout coordinates stay inside 0..=1000"
    )]
    let value = value as f32;
    value
}
