//! One local map shared by every Knowledge collection.
mod drawing;
mod scene;
#[cfg(test)]
mod tests;
use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    memory_view::{Frame, Viewport},
    shell::Shell,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Point, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
    relative,
};
use scene::Scene;
use sovra_core::{
    Command, Event,
    knowledge_map::{Branch, Layer, MapPage, MapRequest},
};
use std::{
    collections::{BTreeSet, VecDeque},
    rc::Rc,
};

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum OptionKey {
    Labels,
    Membership,
    Relationships,
    Focus,
    History,
    List,
    LinkedLayout,
}

struct Hover {
    node: usize,
    neighbours: BTreeSet<usize>,
}
impl Hover {
    fn includes(&self, i: usize) -> bool {
        i == self.node || self.neighbours.contains(&i)
    }
}

pub(crate) struct MapView {
    width: design::WidthClass,
    filters_open: bool,
    options: BTreeSet<OptionKey>,
    visible_nodes: usize,
    visible_edges: usize,
    queue: VecDeque<(String, Branch)>,
    pub(crate) open: bool,
    data: MapPage,
    scene: Rc<Scene>,
    shown: Rc<Vec<bool>>,
    query: Entity<InputState>,
    _enter: gpui_kit::Subscription,
    sequence: u64,
    pending: Option<(u64, Option<String>)>,
    status: String,
    selected: Option<String>,
    hover: Option<Rc<Hover>>,
    hidden: BTreeSet<Layer>,
    page: usize,
    neighbours: usize,
    frame: Option<Frame>,
    camera: Viewport,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    roots: BTreeSet<String>,
}
impl MapView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find wording in files, personal records and saved work…")
        });
        let enter = cx.subscribe_in(&query, window, |shell, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                shell.search_knowledge_map(cx);
            }
        });
        Self {
            width: design::WidthClass::Compact,
            filters_open: false,
            options: BTreeSet::from([
                OptionKey::Labels,
                OptionKey::Membership,
                OptionKey::Relationships,
            ]),
            visible_nodes: 0,
            visible_edges: 0,
            queue: VecDeque::new(),
            open: false,
            data: MapPage::default(),
            scene: Rc::new(Scene::default()),
            shown: Rc::new(Vec::new()),
            query,
            _enter: enter,
            sequence: 0,
            pending: None,
            status: String::new(),
            selected: None,
            hover: None,
            hidden: BTreeSet::new(),
            page: 0,
            neighbours: 0,
            frame: None,
            camera: Viewport::default(),
            drag: None,
            roots: BTreeSet::new(),
        }
    }
    fn selected_index(&self) -> Option<usize> {
        self.selected
            .as_ref()
            .and_then(|id| self.scene.by_id.get(id).copied())
    }
    fn select_index(&mut self, index: usize) {
        if let Some(n) = self.scene.nodes.get(index) {
            self.selected = Some(n.id.clone());
            self.status.clear();
            self.neighbours = 0;
            self.filter();
        }
    }
    fn on(&self, option: OptionKey) -> bool {
        self.options.contains(&option)
    }
    fn set_hover(&mut self, node: Option<usize>) -> bool {
        if self.hover.as_ref().map(|h| h.node) == node {
            return false;
        }
        self.hover = node.map(|i| {
            Rc::new(Hover {
                node: i,
                neighbours: self
                    .scene
                    .neighbours(
                        i,
                        self.on(OptionKey::Membership),
                        self.on(OptionKey::Relationships),
                    )
                    .filter(|&j| self.shown[j] && j != i)
                    .collect(),
            })
        });
        true
    }
    fn filter(&mut self) {
        self.hover = None;
        self.shown = Rc::new(self.scene.visible(
            &self.hidden,
            self.selected_index().filter(|_| self.on(OptionKey::Focus)),
            self.on(OptionKey::Membership),
            self.on(OptionKey::Relationships),
        ));
        self.visible_nodes = self.shown.iter().filter(|v| **v).count();
        self.visible_edges = self
            .scene
            .edges
            .iter()
            .filter(|e| {
                self.shown[e.from]
                    && self.shown[e.to]
                    && if e.membership {
                        self.on(OptionKey::Membership)
                    } else {
                        self.on(OptionKey::Relationships)
                    }
            })
            .count();
    }
    fn rebuild(&mut self) {
        self.scene = Rc::new(Scene::build(&self.data, self.on(OptionKey::LinkedLayout)));
        self.filter();
    }
    pub(crate) fn close(&mut self) {
        if self.open {
            self.open = false;
            self.clear();
        }
    }
    fn clear(&mut self) {
        self.pending = None;
        self.queue.clear();
        self.data = MapPage::default();
        self.selected = None;
        self.options.remove(&OptionKey::Focus);
        self.page = 0;
        self.camera = Viewport::default();
        self.neighbours = 0;
        self.rebuild();
    }
}
impl Shell {
    pub(crate) fn open_knowledge_map(&mut self, cx: &mut Context<Self>) {
        let map = &mut self.knowledge_map;
        map.open = true;
        map.clear();
        map.roots = self
            .file_index
            .state
            .roots
            .iter()
            .map(|r| r.id.clone())
            .collect();
        self.load_map(Branch::Overview, None, cx);
    }
    fn find_on_map(&mut self, cx: &mut Context<Self>) {
        let s = &mut self.knowledge_map;
        let query = s.query.read(cx).value().trim().to_lowercase();
        if query.is_empty() {
            s.status = "Enter a name or words to find.".into();
            cx.notify();
            return;
        }
        let len = s.scene.nodes.len();
        let start = s.selected_index().map_or(0, |i| i + 1);
        if let Some(i) = (0..len).map(|offset| (start + offset) % len).find(|i| {
            let n = &s.scene.nodes[*i];
            !s.hidden.contains(&n.layer)
                && s.data.nodes.get(&n.id).is_some_and(|source| {
                    source.label.to_lowercase().contains(&query)
                        || source.detail.to_lowercase().contains(&query)
                })
        }) {
            s.select_index(i);
            if let Some(frame) = s.frame {
                let dot = &s.scene.nodes[i];
                let spot = frame.to_screen(s.camera, dot.x, dot.y);
                s.camera.pan += frame.bounds.center() - spot;
            }
            s.status = format!(
                "Selected {}. Use Find on map again for the next match.",
                s.scene.nodes[i].label
            );
        } else {
            s.status="No drawn record in the enabled collections matches. Add search results to look in connected sources.".into();
        }
        cx.notify();
    }
    fn search_knowledge_map(&mut self, cx: &mut Context<Self>) {
        let query = self.knowledge_map.query.read(cx).value().trim().to_owned();
        if query.is_empty() {
            self.knowledge_map.status = "Enter a name or words to find.".into();
            cx.notify();
            return;
        }
        self.load_map(Branch::Search(query), None, cx);
    }
    fn draw_more_collections(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.knowledge_map;
        state.queue = state
            .data
            .branches
            .iter()
            .filter_map(|(key, branch)| {
                let branch = branch.as_ref()?;
                if !matches!(
                    branch,
                    Branch::Files { .. }
                        | Branch::Findings(..)
                        | Branch::Relations(None, _)
                        | Branch::Quotes(_)
                        | Branch::Summaries(_)
                        | Branch::Facts(_)
                        | Branch::Sources(_)
                        | Branch::Artifacts(_)
                        | Branch::GroupRecords(..)
                ) {
                    return None;
                }
                let index = *state.scene.by_id.get(key)?;
                state.shown[index].then(|| (key.clone(), branch.clone()))
            })
            .collect();
        if let Some((owner, branch)) = state.queue.pop_front() {
            self.load_map(branch, Some(owner), cx);
        } else {
            state.status="All top-level pages for the visible collections are drawn. Select a point to follow its evidence.".into();
            cx.notify();
        }
    }
    fn load_map(&mut self, branch: Branch, owner: Option<String>, cx: &mut Context<Self>) {
        let state = &mut self.knowledge_map;
        state.sequence = state.sequence.wrapping_add(1);
        let id = state.sequence;
        let cmd = Command::KnowledgeMap(MapRequest {
            id,
            branch,
            history: state.on(OptionKey::History),
        });
        if cx.global::<RuntimeClient>().0.send(cmd).is_ok() {
            state.pending = Some((id, owner));
            state.status = "Reading local knowledge…".into();
        } else {
            state.pending = None;
            state.status = "The knowledge service is unavailable.".into();
        }
        cx.notify();
    }
    pub(crate) fn observe_knowledge_map(&mut self, event: &Event, cx: &mut Context<Self>) {
        use sovra_core::{
            MemoryEvent, MemoryReviewResult, artifacts::ArtifactEvent,
            connections::ConnectionEvent, file_index::IndexEvent, group::GroupEvent,
        };
        let s = &mut self.knowledge_map;
        if !s.open {
            return;
        }
        if let Event::KnowledgeMapped { request_id, result } = event {
            if s.pending.as_ref().map(|v| v.0) != Some(*request_id) {
                return;
            }
            let owner = s.pending.take().and_then(|v| v.1);
            match result {
                Ok(page) => {
                    let mut incoming = (**page).clone();
                    if let Some(key) = owner {
                        // A leaf read is exhausted unless the adapter supplied a continuation.
                        incoming.branches.entry(key.clone()).or_insert(None);
                        s.data.branches.remove(&key);
                    }
                    // Rediscovering an entity from a different branch must preserve its cursor.
                    incoming
                        .branches
                        .retain(|id, _| !s.data.branches.contains_key(id));
                    s.status = incoming.notices.join("  ");
                    incoming.notices.clear();
                    if s.data.nodes.is_empty() {
                        incoming.people(
                            &self.state.connections.people,
                            &self.state.connections.groups,
                        );
                    }
                    s.data.merge(incoming);
                    s.rebuild();
                }
                Err(e) => s.status.clone_from(e),
            }
            if let Some((owner, branch)) = s.queue.pop_front() {
                self.load_map(branch, Some(owner), cx);
            }
            return;
        }
        let invalidate = match event {
            Event::FileIndex(IndexEvent::Roots(roots)) => {
                let ids = roots.iter().map(|r| r.id.clone()).collect();
                let removed = !s.roots.is_subset(&ids);
                s.roots = ids;
                removed
            }
            Event::Memory(MemoryEvent::Review(r)) => matches!(
                r.result,
                Ok(MemoryReviewResult::Changed(_) | MemoryReviewResult::SourceStopped(_))
            ),
            Event::Artifacts(ArtifactEvent::Deleted { .. } | ArtifactEvent::Saved(_))
            | Event::Connections(
                ConnectionEvent::PeopleChanged(_)
                | ConnectionEvent::GroupsChanged(_)
                | ConnectionEvent::Loaded(_),
            ) => true,
            Event::Group(g) => matches!(
                g.as_ref(),
                GroupEvent::Knowledge(_)
                    | GroupEvent::Artifacts(_)
                    | GroupEvent::Templates(_)
                    | GroupEvent::LeftGroup
                    | GroupEvent::Changed(_)
                    | GroupEvent::Memberships(_)
            ),
            _ => false,
        };
        if invalidate {
            s.clear();
            self.load_map(Branch::Overview, None, cx);
        }
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let s = &shell.knowledge_map;
    let t = design::tokens(cx);
    let compact = s.width == design::WidthClass::Compact;
    let counts = format!(
        "{} points drawn · {} connections drawn",
        s.visible_nodes, s.visible_edges
    );
    let count = div()
        .id("atlas-count")
        .test_support()
        .aria_label(counts.clone())
        .sovra_text(TextRole::Caption, cx)
        .text_color(t.text_secondary)
        .child(counts);
    let mut heading = h_flex().min_w_0().flex_1().gap(t.space.small).child(
        div()
            .truncate()
            .sovra_text(TextRole::SectionTitle, cx)
            .child("Knowledge map"),
    );
    let mut filters = v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .gap(t.space.small);
    if compact {
        filters = filters.child(count);
    } else {
        heading = heading.child(count);
    }
    let header = h_flex()
        .w_full()
        .flex_shrink_0()
        .gap(t.space.compact)
        .child(heading)
        .child(
            design::button(
                "atlas-filters",
                if compact {
                    "Filters"
                } else {
                    "Search and filters"
                },
                cx,
            )
            .accessibility_label("Search and filters")
            .small()
            .selected(s.filters_open)
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.knowledge_map.filters_open = !shell.knowledge_map.filters_open;
                cx.notify();
            })),
        )
        .child(
            design::button(
                "atlas-close",
                if compact { "Back" } else { "Back to Knowledge" },
                cx,
            )
            .accessibility_label("Back to Knowledge")
            .small()
            .ghost()
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.knowledge_map.close();
                cx.notify();
            })),
        );
    let mut page = v_flex()
        .id("all-knowledge-map")
        .test_support()
        .relative()
        .size_full()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .p(t.space.small)
        .gap(t.space.compact)
        .child(design::width_probe(cx, |s| &mut s.knowledge_map.width))
        .child(header);
    if s.filters_open {
        let mut layers = h_flex().flex_wrap().gap(t.space.compact);
        let ink = t.graph;
        let colours = [
            ink.detail,
            ink.version,
            ink.subject,
            ink.chat,
            ink.message,
            ink.saved_from,
            ink.focus,
        ];
        for layer in Layer::ALL {
            let enabled = !s.hidden.contains(&layer);
            layers = layers.child(
                design::button(("atlas-layer", layer as usize), layer.label(), cx)
                    .small()
                    .selected(enabled)
                    .child(
                        div()
                            .size(ink.legend_swatch)
                            .rounded(t.radius.pill)
                            .bg(colours[layer as usize]),
                    )
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        let s = &mut shell.knowledge_map;
                        if !s.hidden.remove(&layer) {
                            s.hidden.insert(layer);
                        }
                        if s.selected_index()
                            .is_some_and(|i| s.hidden.contains(&s.scene.nodes[i].layer))
                        {
                            s.selected = None;
                            s.status.clear();
                            s.options.remove(&OptionKey::Focus);
                        }
                        s.page = 0;
                        s.neighbours = 0;
                        s.filter();
                        cx.notify();
                    })),
            );
        }
        let mut controls = h_flex().flex_wrap().gap(t.space.compact);
        for (id, label, option) in [
            ("atlas-labels", "Labels", OptionKey::Labels),
            (
                "atlas-membership",
                "Collection links",
                OptionKey::Membership,
            ),
            (
                "atlas-relations",
                "Evidence links",
                OptionKey::Relationships,
            ),
            (
                "atlas-layout",
                "Group by connections",
                OptionKey::LinkedLayout,
            ),
            ("atlas-focus", "Neighbours only", OptionKey::Focus),
            ("atlas-history", "Include history", OptionKey::History),
            ("atlas-list", "Read as a list", OptionKey::List),
        ] {
            controls = controls.child(
                design::button(id, label, cx)
                    .small()
                    .ghost()
                    .selected(s.on(option))
                    .disabled(option == OptionKey::Focus && s.selected.is_none())
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        let state = &mut shell.knowledge_map;
                        if !state.options.remove(&option) {
                            state.options.insert(option);
                        }
                        state.page = 0;
                        state.neighbours = 0;
                        match option {
                            OptionKey::History => shell.open_knowledge_map(cx),
                            OptionKey::LinkedLayout => {
                                state.rebuild();
                                cx.notify();
                            }
                            _ => {
                                state.filter();
                                cx.notify();
                            }
                        }
                    })),
            );
        }
        if s.queue.is_empty() {
            controls = controls.child(
                design::button("atlas-draw-more", "Draw more across collections", cx)
                    .small()
                    .disabled(s.pending.is_some())
                    .on_click(cx.listener(|shell, _, _, cx| shell.draw_more_collections(cx))),
            );
        } else {
            controls = controls.child(
                design::button("atlas-stop", "Stop adding pages", cx)
                    .small()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.knowledge_map.queue.clear();
                        cx.notify();
                    })),
            );
        }
        filters = filters
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        div().min_w_0().flex_1().child(
                            Input::new(&s.query)
                                .id("atlas-search")
                                .aria_label("Find sources to add to this map"),
                        ),
                    )
                    .child(
                        design::button("atlas-find", "Find on map", cx)
                            .on_click(cx.listener(|shell, _, _, cx| shell.find_on_map(cx))),
                    )
                    .child(
                        design::button("atlas-add-search", "Add search results", cx)
                            .disabled(s.pending.is_some())
                            .on_click(
                                cx.listener(|shell, _, _, cx| shell.search_knowledge_map(cx)),
                            ),
                    ),
            )
            .child(layers)
            .child(
                controls.child(
                    design::button("atlas-refresh", "Refresh", cx)
                        .small()
                        .icon(IconName::RefreshCw)
                        .on_click(cx.listener(|shell, _, _, cx| shell.open_knowledge_map(cx))),
                ),
            );
        page = page.child(
            div()
                .id("atlas-filter-scroll")
                .test_support()
                .w_full()
                .min_h_0()
                .max_h(relative(0.40))
                .flex_shrink_0()
                .overflow_y_scroll()
                .child(filters),
        );
    }
    if !s.status.is_empty() {
        page = page.child(
            div()
                .id("atlas-status")
                .test_support()
                .aria_label(s.status.clone())
                .sovra_text(TextRole::Caption, cx)
                .child(s.status.clone()),
        );
    }
    if s.visible_nodes == 0 && s.pending.is_none() {
        page = page.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child(if s.hidden.len() == 7 {
                    "Turn on a collection to draw its records."
                } else {
                    "Refresh to draw the available knowledge."
                }),
        );
    }
    let mut view = v_flex().flex_1().min_w_0().min_h_0().overflow_hidden();
    if s.on(OptionKey::List) {
        view = view.child(
            div()
                .id("atlas-list-scroll")
                .test_support()
                .size_full()
                .overflow_y_scroll()
                .child(record_list(s, cx)),
        );
    } else {
        view = view.child(drawing::canvas_view(shell, cx));
    }
    let mut content = div()
        .flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .w_full()
        .gap(t.space.small);
    let wide = s.width.fits_detail_pane();
    content = if wide {
        content.flex_row()
    } else {
        content.flex_col()
    };
    content = content.child(view);
    if let Some(i) = s.selected_index() {
        let inspector_pane = div()
            .id("atlas-inspector-scroll")
            .test_support()
            .min_h_0()
            .min_w_0()
            .flex_shrink_0()
            .overflow_y_scroll();
        let inspector_pane = if wide {
            inspector_pane.w(t.layout.inspector_width).h_full()
        } else {
            inspector_pane.w_full().max_h(relative(0.45))
        };
        content = content.child(inspector_pane.child(inspector(s, i, cx)));
    }
    page = page.child(content);
    if !s.on(OptionKey::List) {
        let level = format!("{:.0}%", s.camera.zoom * 100.);
        let mut toolbar = h_flex()
            .w_full()
            .flex_shrink_0()
            .gap(t.space.compact)
            .justify_end();
        if !compact {
            toolbar = toolbar.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child("Hover to highlight connections · Scroll to zoom · Drag to pan"),
            );
        }
        page =
            page.child(
                toolbar
                    .child(
                        design::button("atlas-out", "−", cx)
                            .accessibility_label("Zoom out")
                            .tooltip("Zoom out")
                            .small()
                            .on_click(
                                cx.listener(|shell, _, _, cx| drawing::zoom(shell, 0.8, None, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("atlas-zoom-level")
                            .test_support()
                            .aria_label(level.clone())
                            .sovra_text(TextRole::Body, cx)
                            .child(level),
                    )
                    .child(
                        design::button("atlas-in", "+", cx)
                            .accessibility_label("Zoom in")
                            .tooltip("Zoom in")
                            .small()
                            .on_click(
                                cx.listener(|shell, _, _, cx| drawing::zoom(shell, 1.25, None, cx)),
                            ),
                    )
                    .child(design::button("atlas-fit", "Fit map", cx).small().on_click(
                        cx.listener(|shell, _, _, cx| {
                            shell.knowledge_map.camera = Viewport::default();
                            cx.notify();
                        }),
                    )),
            );
    }
    page
}
fn node_button(
    s: &MapView,
    i: usize,
    label: String,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    design::button(("atlas-record", i), label, cx)
        .small()
        .selected(s.selected_index() == Some(i))
        .max_w_full()
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.knowledge_map.select_index(i);
            cx.notify();
        }))
}
fn record_list(s: &MapView, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut list = v_flex()
        .w_full()
        .min_w_0()
        .items_start()
        .gap(t.space.compact);
    let filtered = s
        .scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| s.shown[*i]);
    let total = filtered.clone().count();
    for (i, n) in filtered.skip(s.page * 24).take(24) {
        list = list.child(node_button(
            s,
            i,
            format!("{} · {}", n.layer.label(), n.label),
            cx,
        ));
    }
    let mut nav = h_flex().gap(t.space.small).flex_wrap();
    for (id, label, next, enabled) in [
        (
            "atlas-prev",
            "Previous points",
            s.page.saturating_sub(1),
            s.page > 0,
        ),
        (
            "atlas-next",
            "More points",
            s.page + 1,
            (s.page + 1) * 24 < total,
        ),
    ] {
        nav = nav.child(
            design::button(id, label, cx)
                .small()
                .disabled(!enabled)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.knowledge_map.page = next;
                    cx.notify();
                })),
        );
    }
    list.child(nav)
}
fn inspector(s: &MapView, index: usize, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let dot = &s.scene.nodes[index];
    let node = &s.data.nodes[&dot.id];
    let mut panel = v_flex()
        .w_full()
        .min_w_0()
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .justify_between()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(node.layer.label()),
                )
                .child(
                    design::button("atlas-clear", "Close details", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            let state = &mut shell.knowledge_map;
                            state.selected = None;
                            state.status.clear();
                            state.options.remove(&OptionKey::Focus);
                            state.filter();
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("atlas-selection")
                .test_support()
                .aria_label(node.label.clone())
                .sovra_text(TextRole::SectionTitle, cx)
                .child(node.label.clone()),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .min_w_0()
                .child(node.detail.clone()),
        );
    if let Some(Some(branch)) = s.data.branches.get(&dot.id) {
        let branch = branch.clone();
        let owner = dot.id.clone();
        panel = panel.child(
            design::primary_button("atlas-expand", "Draw connected records", cx)
                .self_start()
                .disabled(s.pending.is_some())
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.load_map(branch.clone(), Some(owner.clone()), cx);
                })),
        );
    }
    let mut links = h_flex().flex_wrap().gap(t.space.compact);
    let mut count = 0usize;
    for j in &s.scene.adjacent[index] {
        let e = &s.scene.edges[*j];
        let outgoing = e.from == index;
        let other = if outgoing { e.to } else { e.from };
        if !s.shown[other]
            || (e.membership && !s.on(OptionKey::Membership))
            || (!e.membership && !s.on(OptionKey::Relationships))
        {
            continue;
        }
        let label = if outgoing {
            format!("{} · {}", e.label, s.scene.nodes[other].label)
        } else {
            format!("{} · {}", s.scene.nodes[other].label, e.label)
        };
        count += 1;
        if count > (s.neighbours + 1) * 16 || count <= s.neighbours * 16 {
            continue;
        }
        links = links.child(
            design::button(("atlas-link", *j), label, cx)
                .small()
                .max_w_full()
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.knowledge_map.select_index(other);
                    cx.notify();
                })),
        );
    }
    panel = panel.child(links);
    if count > 16 {
        let pages = count.div_ceil(16);
        panel = panel.child(
            design::button(
                "atlas-more-links",
                format!("More connections · {} / {pages}", s.neighbours + 1),
                cx,
            )
            .small()
            .self_start()
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.knowledge_map.neighbours = (shell.knowledge_map.neighbours + 1) % pages;
                cx.notify();
            })),
        );
    }
    panel
}
