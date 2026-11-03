//! Live conversation memories, retained sources, and explicit user corrections.

use gpui_kit::component::{
    Disableable as _, Selectable as _, h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command, FactId, MemoryAction, MemoryCorrection, MemoryListQuery, MemoryReviewEvent,
    MemoryReviewResult, MemoryReviewState, MemorySemanticStatus,
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    models_view::ModelsView,
};

mod directory;
mod exploration;
mod graph;
mod profile;
mod source_browser;

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view flags; a state enum would be artificial"
)]
#[derive(Default)]
struct CaptureView {
    dirty: bool,
    expanded: bool,
    new_details: bool,
    result: Option<String>,
}

#[derive(Clone, Copy)]
enum SourceNavigation {
    Conversation,
    Memories,
    Sources,
}

#[derive(Clone)]
struct SupportingDetail {
    id: FactId,
    label: String,
    source_only: bool,
    navigation: SourceNavigation,
    trail: Vec<FactId>,
}

pub(crate) struct MemoryView {
    state: MemoryReviewState,
    navigation_request: Option<u64>,
    models: Entity<ModelsView>,
    embedding_search: Entity<InputState>,
    choosing: bool,
    _embedding_search: gpui_kit::Subscription,
    query: MemoryListQuery,
    subject_query: Option<sovra_core::MemorySubjectQuery>,
    subject_search: Entity<InputState>,
    source_query: Option<sovra_core::MemorySourceQuery>,
    supporting_detail: Option<SupportingDetail>,
    source_search: Entity<InputState>,
    _search_events: Vec<gpui_kit::Subscription>,
    search: Entity<InputState>,
    correction: Entity<TextareaState>,
    editing: Option<FactId>,
    stopping: Option<FactId>,
    source_stop: Option<sovra_core::EpisodeId>,
    source_only: bool,
    source_navigation: SourceNavigation,
    trail: Vec<FactId>,
    expanded_source: Option<sovra_core::EpisodeId>,
    capture: CaptureView,
    adding: bool,
    note: Entity<TextareaState>,
    notice: Option<String>,
    width_class: design::WidthClass,
    /// Detail or chat chosen in the picture. Dropped when that detail leaves the page.
    graph_selection: Option<sovra_core::MemoryGraphNodeId>,
    /// Local repeat check. Empty until the person asks, and cleared when the page reloads.
    duplicates: Option<sovra_core::MemoryDuplicateReport>,
}

impl MemoryView {
    pub(crate) fn new(
        models: Entity<ModelsView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let embedding_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a search helper…"));
        let subscription = cx.subscribe_in(
            &embedding_search,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search saved details"));
        let subject_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a person, place or project"));
        let source_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search original wording"));
        let search_events = vec![
            cx.subscribe_in(
                &source_search,
                window,
                |view, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. })
                        && !view.state.loading
                        && view.source_query.is_some()
                        && !view.source_only
                    {
                        view.search_sources(cx);
                    }
                },
            ),
            cx.subscribe_in(&search, window, |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && !view.state.loading
                    && view.subject_query.is_none()
                    && view.source_query.is_none()
                    && !view.source_only
                {
                    view.load(cx);
                }
            }),
            cx.subscribe_in(
                &subject_search,
                window,
                |view, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. })
                        && !view.state.loading
                        && view.subject_query.is_some()
                    {
                        view.search_subjects(cx);
                    }
                },
            ),
        ];
        Self {
            models,
            embedding_search,
            choosing: false,
            _embedding_search: subscription,
            state: MemoryReviewState::default(),
            navigation_request: None,
            query: MemoryListQuery::default(),
            search,
            subject_search,
            subject_query: None,
            source_query: None,
            supporting_detail: None,
            source_search,
            _search_events: search_events,
            correction: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 5)),
            editing: None,
            stopping: None,
            source_stop: None,
            source_only: false,
            source_navigation: SourceNavigation::Conversation,
            trail: Vec::new(),
            expanded_source: None,
            capture: CaptureView::default(),
            adding: false,
            note: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 5)),
            notice: None,
            width_class: design::WidthClass::Compact,
            graph_selection: None,
            duplicates: None,
        }
    }

    fn send(&mut self, action: MemoryAction, cx: &mut Context<Self>) {
        let navigates = matches!(
            action,
            MemoryAction::List(_)
                | MemoryAction::ListSubjects(_)
                | MemoryAction::ListSources(_)
                | MemoryAction::Inspect(_)
                | MemoryAction::InspectSource(_)
        );
        let command = self.state.begin(action);
        let request_id = command.request_id;
        self.navigation_request = navigates.then_some(request_id);
        if let Err(error) = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Memory(command))
        {
            self.state.apply(&MemoryReviewEvent {
                request_id,
                result: Err(error.to_string()),
            });
        }
        cx.notify();
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        self.supporting_detail = None;
        self.subject_query = None;
        self.source_query = None;
        self.trail.clear();
        self.expanded_source = None;
        self.capture.new_details = false;
        self.capture.dirty = false;
        self.source_only = false;
        self.query.search = self.search.read(cx).value().to_string();
        self.query.before = None;
        self.editing = None;
        self.stopping = None;
        self.source_stop = None;
        self.send(MemoryAction::List(self.query.clone()), cx);
    }

    pub(crate) fn source_only(&self) -> bool {
        self.source_only && matches!(self.source_navigation, SourceNavigation::Conversation)
    }

    pub(crate) fn reading_source(&self) -> bool {
        self.source_only || self.supporting_detail.is_some()
    }

    pub(crate) fn captured(
        &mut self,
        job: &sovra_core::MemoryJob,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(result) = job.result_label() {
            self.capture.result = Some(result);
            cx.notify();
        }
        if job.state == sovra_core::MemoryJobState::Complete && !job.fact_ids.is_empty() {
            self.capture.new_details = true;
        }
        self.capture_activity(visible, cx);
    }

    pub(crate) fn capture_activity(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.capture.dirty = true;
        self.refresh_captured(visible, cx);
        cx.notify();
    }

    pub(crate) fn refresh_captured(&mut self, visible: bool, cx: &mut Context<Self>) {
        // A background save must not discard a correction, move an older page,
        // or apply search text that the user is still typing.
        if (self.capture.new_details || self.capture.dirty)
            && visible
            && !self.source_only
            && self.subject_query.is_none()
            && self.source_query.is_none()
            && !self.state.loading
            && self.state.detail.is_none()
            && self.editing.is_none()
            && self.stopping.is_none()
            && !self.adding
            && self.query.before.is_none()
            && self.search.read(cx).value().as_ref() == self.query.search
        {
            self.load(cx);
            // Automatic refresh keeps the reader's position on the current page.
            self.navigation_request = None;
        }
    }

    pub(crate) fn open_source(&mut self, action: MemoryAction, cx: &mut Context<Self>) {
        self.supporting_detail = None;
        self.subject_query = None;
        self.source_query = None;
        self.trail.clear();
        self.expanded_source = None;
        self.source_only = true;
        self.source_navigation = SourceNavigation::Conversation;
        self.state.detail = None;
        self.state.page = sovra_core::MemoryPage::default();
        self.editing = None;
        self.stopping = None;
        self.source_stop = None;
        self.send(action, cx);
    }

    pub(crate) fn apply_semantic(&mut self, status: &MemorySemanticStatus, cx: &mut Context<Self>) {
        self.state.semantic.apply(status);
        cx.notify();
    }

    fn semantic_controls(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex()
            .id("memory-semantic")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Find related memories in chat"),
            )
            .child(
                div()
                    .id("memory-semantic-status")
                    .test_support()
                    .child(self.state.semantic.phase.label().to_owned()),
            )
            .child(
                design::button("memory-choose-semantic", "Choose a search helper", cx)
                    .disabled(self.state.loading)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.choosing = true;
                        cx.notify();
                    })),
            );
        if let Some(setting) = &self.state.semantic.state.setting {
            panel = panel.child(crate::widgets::model_destination::choice(
                &setting.target,
                "memory-current-destination".into(),
                design::button("memory-disable-semantic", "Turn off search by meaning", cx)
                    .disabled(self.state.loading)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.send(MemoryAction::ConfigureSemantic(None), cx);
                    })),
                cx,
            ));
        }
        if self.choosing {
            let candidates = sovra_core::file_index::semantic::embedding_targets(
                &self.models.read(cx).state,
                self.embedding_search.read(cx).value().as_ref(),
            );
            let mut chooser = v_flex().id("memory-semantic-chooser").test_support().w_full().min_w_0().gap(t.space.small)
                .child(div().sovra_text(TextRole::Caption, cx).child("Enabling sends saved details, including retained versions, and future questions to the named service, including details saved later. The service may charge for usage. No model is downloaded automatically. Use a model designed for plain-text embeddings; models requiring task prefixes are not supported yet."))
                .child(Input::new(&self.embedding_search).aria_label("Filter memory search helpers"));
            if candidates.is_empty() {
                chooser = chooser.child(div().child("No matching models. In Models, refresh OpenRouter, inspect an installed Ollama embedding model or add an embedding model ID to an OpenAI-compatible connection."));
            }
            for (index, target) in candidates.iter().take(20).enumerate() {
                let selection = target.clone();
                chooser = chooser.child(crate::widgets::model_destination::choice(
                    target,
                    gpui_kit::SharedString::from(format!("memory-semantic-destination-{index}"))
                        .into(),
                    design::button(
                        ("memory-enable-semantic", index),
                        "Use for saved memories",
                        cx,
                    )
                    .disabled(self.state.loading)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.choosing = false;
                        view.send(MemoryAction::ConfigureSemantic(Some(selection.clone())), cx);
                    })),
                    cx,
                ));
            }
            if candidates.len() > 20 {
                chooser = chooser.child(div().child(
                    "Showing 20 choices. Filter by model or connection name to find another.",
                ));
            }
            panel = panel.child(chooser.child(
                design::button("memory-cancel-semantic", "Cancel", cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.choosing = false;
                        cx.notify();
                    },
                )),
            ));
        }
        panel
    }

    /// A successful explicit navigation starts at the top; stale responses never scroll.
    pub(crate) fn apply(&mut self, event: &MemoryReviewEvent, cx: &mut Context<Self>) -> bool {
        if self.state.apply(event) {
            let reset_scroll =
                self.navigation_request.take() == Some(event.request_id) && event.result.is_ok();
            match &event.result {
                Ok(MemoryReviewResult::Changed(_)) => {
                    if self.adding {
                        self.adding = false;
                        self.notice = Some(
                            "Saved exactly as you wrote it. You can review or correct it below."
                                .into(),
                        );
                    } else if self.stopping.is_some() {
                        self.notice = Some("Removed this detail and its corrected versions from memory. Stopped history remains available for review.".into());
                    } else if self.editing.is_some() {
                        self.notice = Some("Saved your correction as a new version.".into());
                    }
                    self.load(cx);
                }
                Ok(MemoryReviewResult::SourceStopped(_)) => {
                    self.source_stop = None;
                    self.notice = Some("Stopped using this original record. It stays available with Include stopped records.".into());
                    if matches!(self.source_navigation, SourceNavigation::Sources) {
                        self.load_sources(self.source_query.clone().unwrap_or_default(), cx);
                    } else {
                        self.load(cx);
                    }
                }
                Ok(MemoryReviewResult::Retried(count)) => {
                    self.notice = Some(if *count == 0 {
                        "No failed messages need retrying. Pending messages will be checked automatically.".into()
                    } else {
                        format!(
                            "{count} {} queued to try again. Progress appears in Recent activity.",
                            if *count == 1 { "message" } else { "messages" }
                        )
                    });
                    self.load(cx);
                }
                Ok(MemoryReviewResult::Page(_)) => {
                    self.duplicates = None;
                    if let Some(sovra_core::MemoryGraphNodeId::Detail(id)) = &self.graph_selection
                        && !self.state.page.facts.iter().any(|fact| &fact.id == id)
                    {
                        self.graph_selection = None;
                    }
                }
                Ok(
                    MemoryReviewResult::Semantics(_)
                    | MemoryReviewResult::Subjects(_)
                    | MemoryReviewResult::Sources(_)
                    | MemoryReviewResult::Detail(_)
                    | MemoryReviewResult::Source(_),
                )
                | Err(_) => {}
            }
            cx.notify();
            return reset_scroll;
        }
        false
    }
}

impl Render for MemoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let busy = self.state.loading;
        let mut column = v_flex()
            .id("memory-browser")
            .test_support()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .relative()
            .child(design::width_probe(cx, |view| &mut view.width_class))
            .sovra_surface(Surface::Matte, cx)
            .p(t.space.medium)
            .gap(t.space.small);
        if self.subject_query.is_some() {
            return column.child(self.subject_directory(cx));
        }
        if self.source_query.is_some() && !self.source_only {
            return column.child(self.source_library(cx));
        }
        if !self.source_only && self.state.detail.is_none() {
            column = column
            .child(self.profile(cx))
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("Saved details"))
            .child(div().sovra_text(TextRole::Caption, cx).child("Read what is saved, select a person or project, or open a detail to explore its sources and connections."))
            .child(div().id("memory-search-input").test_support().w_full().min_w_0().child(Input::new(&self.search).aria_label("Search saved memories")))
            .child(h_flex().w_full().flex_wrap().gap(t.space.small)
                .child(design::button("memory-search", "Search / refresh", cx).disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| view.load(cx))))
                .child(design::button("memory-history", "Include history", cx).selected(self.query.include_history).disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.query.include_history = !view.query.include_history;
                        view.load(cx);
                    }))));
            if let Some(source) = &self.query.source {
                let id = source.clone();
                column = column.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            div()
                                .id("memory-source-filter")
                                .test_support()
                                .child("Details linked to one original record"),
                        )
                        .child(
                            design::button("memory-filter-source-open", "Read original record", cx)
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_source(MemoryAction::InspectSource(id.clone()), cx);
                                    view.source_navigation = SourceNavigation::Memories;
                                })),
                        )
                        .child(
                            design::button("memory-clear-source", "All saved details", cx)
                                .disabled(busy)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.query.source = None;
                                    view.load(cx);
                                })),
                        ),
                );
            }
            if let Some(subject) = &self.query.subject {
                column = column.child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            div()
                                .id("memory-subject-filter")
                                .test_support()
                                .child(format!("About: {subject}")),
                        )
                        .child(
                            design::button("memory-clear-subject", "All subjects", cx)
                                .disabled(busy)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.query.subject = None;
                                    view.load(cx);
                                })),
                        ),
                );
            }
        }
        if !self.source_only {
            if let Some(result) = &self.capture.result {
                column = column.child(
                    div()
                        .id("memory-capture-result")
                        .test_support()
                        .role(gpui_kit::Role::Status)
                        .aria_label(result.clone())
                        .min_w_0()
                        .sovra_text(TextRole::Body, cx)
                        .child(result.clone()),
                );
            }
            if self.capture.new_details {
                column = column.child(div().id("memory-new-details").test_support()
                    .min_w_0().sovra_text(TextRole::Body, cx)
                    .child("New details were saved. Finish reviewing or editing, then use Search / refresh to update this list."));
            }
        }
        if busy {
            column = column.child(
                div()
                    .id("memory-loading")
                    .test_support()
                    .child("Loading saved details…"),
            );
        }
        if let Some(error) = &self.state.error {
            column = column.child(div().id("memory-error").test_support().child(error.clone()));
        }
        if !self.source_only
            && self.state.detail.is_none()
            && self.state.page.facts.is_empty()
            && !busy
            && self.state.error.is_none()
        {
            let message = if self.query.before.is_some() {
                "No more saved details on this page. Use Back to newest to return to recent memories."
            } else if self.query.source.is_some() {
                "No saved details match this source and search. Include history to see earlier or stopped details, or choose All saved details."
            } else if self.query.subject.is_some() {
                "No saved details match this subject and search. Choose All subjects or clear your search to broaden the results."
            } else if self.query.search.is_empty() {
                "No saved details yet. Add a detail above, or tell Sovra about your preferences, important people, work or plans in chat. Keep remembering switched on for automatic learning."
            } else {
                "No saved details match your search. Try fewer words, or clear the search to see everything."
            };
            column = column.child(
                div()
                    .id("memory-empty")
                    .test_support()
                    .aria_label(message)
                    .child(message),
            );
        }
        if !self.source_only && self.state.detail.is_none() && !self.state.page.facts.is_empty() {
            let count = self.state.page.facts.len();
            let label = if count == 1 {
                "1 saved detail shown".to_owned()
            } else {
                format!("{count} saved details shown")
            };
            column = column.child(
                div()
                    .id("memory-count")
                    .test_support()
                    .aria_label(label.clone())
                    .sovra_text(TextRole::Caption, cx)
                    .child(label),
            );
        }
        if !self.source_only && self.state.detail.is_none() && !busy && self.state.error.is_none()
        {
            column = column.child(self.memory_picture(cx));
        }
        for (index, fact) in self
            .state
            .page
            .facts
            .iter()
            .enumerate()
            .filter(|_| self.state.detail.is_none() && !self.source_only)
        {
            column = column.child(self.saved_card(fact, index, cx));
        }
        if self.state.detail.is_none()
            && let Some(cursor) = self.state.page.next.clone()
        {
            column = column.child(
                design::button("memory-next", "Older memories", cx)
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.query.before = Some(cursor.clone());
                        view.send(MemoryAction::List(view.query.clone()), cx);
                    })),
            );
        }
        if self.state.detail.is_none() && self.query.before.is_some() {
            column = column.child(
                design::button("memory-newest", "Back to newest", cx)
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| view.load(cx))),
            );
        }
        if let Some(detail) = &self.state.detail {
            let id = detail.fact.id.clone();
            let supporting = SupportingDetail {
                id: id.clone(),
                label: detail.fact.content.clone(),
                source_only: self.source_only,
                navigation: self.source_navigation,
                trail: self.trail.clone(),
            };
            let mut panel = v_flex()
                .id("memory-detail")
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .p(t.space.medium)
                .sovra_surface(Surface::Subtle, cx)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Explore this detail"),
                )
                .child(
                    div()
                        .id("memory-detail-text")
                        .test_support()
                        .aria_label(detail.fact.content.clone())
                        .sovra_text(TextRole::Reading, cx)
                        .child(detail.fact.content.clone()),
                )
                .child(self.subject_buttons(&detail.fact, "memory-detail-subject", cx))
                .child(
                    div()
                        .id("memory-learned-date")
                        .test_support()
                        .aria_label(format!(
                            "Saved {} · dates shown in UTC",
                            exploration::date(detail.fact.created_at)
                        ))
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!(
                            "Saved {} · dates shown in UTC",
                            exploration::date(detail.fact.created_at)
                        )),
                )
                .child(
                    div()
                        .id("memory-standing")
                        .test_support()
                        .sovra_text(TextRole::Caption, cx)
                        .child(if detail.fact.stopped_at.is_some() {
                            "Stopped · kept for review, excluded from recall"
                        } else if !detail.fact.is_current() {
                            "Previous version · retained history"
                        } else {
                            "Current saved detail"
                        }),
                )
                .child(
                    design::button("memory-close", "Back to memories", cx)
                        .self_start()
                        .max_w_full()
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.state.detail = None;
                            view.editing = None;
                            view.stopping = None;
                            view.load(cx);
                        })),
                );
            if !self.trail.is_empty() {
                panel = panel.child(
                    design::button("memory-trail-back", "Back to previous detail", cx)
                        .self_start()
                        .max_w_full()
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            if let Some(id) = view.trail.pop() {
                                view.editing = None;
                                view.stopping = None;
                                view.send(MemoryAction::Inspect(id), cx);
                            }
                        })),
                );
            }
            panel = panel.child(
                design::button("memory-supporting-records", "Browse supporting records", cx)
                    .self_start()
                    .max_w_full()
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.source_search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.supporting_detail = Some(supporting.clone());
                        view.load_sources(
                            sovra_core::MemorySourceQuery {
                                fact: Some(supporting.id.clone()),
                                ..Default::default()
                            },
                            cx,
                        );
                    })),
            );
            if let Some(source) = &detail.source {
                let source_label = match source.kind {
                    sovra_core::EpisodeKind::UserMessage => "One supporting chat message",
                    sovra_core::EpisodeKind::Note => "One supporting note or correction",
                    sovra_core::EpisodeKind::AgentMessage => {
                        "An earlier reply (not a user statement)"
                    }
                    sovra_core::EpisodeKind::ToolCall => "An earlier tool request",
                    sovra_core::EpisodeKind::ToolResult => "An earlier tool result",
                };
                let (excerpt, shortened) = exploration::source_excerpt(&source.content);
                let expanded = self.expanded_source.as_ref() == Some(&source.id);
                let shown = if expanded {
                    source.content.clone()
                } else {
                    excerpt
                };
                panel = panel
                    .child(div().sovra_text(TextRole::Caption, cx).child(source_label))
                    .child(
                        div()
                            .id("memory-source")
                            .test_support()
                            .aria_label(shown.clone())
                            .sovra_text(TextRole::Reading, cx)
                            .min_w_0()
                            .child(shown),
                    );
                if shortened {
                    let source_id = source.id.clone();
                    panel = panel.child(
                        design::button(
                            "memory-expand-source",
                            if expanded {
                                "Show shorter excerpt"
                            } else {
                                "Read full original message"
                            },
                            cx,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.expanded_source = if expanded {
                                None
                            } else {
                                Some(source_id.clone())
                            };
                            cx.notify();
                        })),
                    );
                }
            } else {
                panel = panel.child(div().child("This older saved detail has no source attached."));
            }
            if let Some(extraction) = &detail.fact.extraction {
                panel = panel.child(div().sovra_text(TextRole::Caption, cx).child(format!(
                    "Interpreted by {}. This may need correction.",
                    extraction.model
                )));
            }
            if let Some(previous) = &detail.fact.replaces {
                let previous = previous.clone();
                panel = panel.child(
                    design::button("memory-previous", "Previous version", cx)
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.editing = None;
                            view.stopping = None;
                            view.send(MemoryAction::Inspect(previous.clone()), cx);
                        })),
                );
            }
            if detail.fact.stopped_at.is_none() {
                let text = detail.fact.content.clone();
                let edit_id = id.clone();
                let stop_id = id.clone();
                panel = panel.child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap(t.space.small)
                        .when(detail.fact.is_current(), |row| {
                            row.child(
                                design::button("memory-edit", "Edit detail", cx)
                                    .disabled(busy)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.correction.update(cx, |input, cx| {
                                            input.set_value(text.clone(), window, cx);
                                        });
                                        view.editing = Some(edit_id.clone());
                                        view.stopping = None;
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            design::button("memory-stop", "Remove from memory", cx)
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.stopping = Some(stop_id.clone());
                                    view.editing = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
            if self.editing.as_ref() == Some(&id) {
                let target = id.clone();
                panel = panel
                    .child(Textarea::new(&self.correction).aria_label("Corrected memory"))
                    .child(
                        design::button("memory-save", "Save correction", cx)
                            .disabled(busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let correction = MemoryCorrection {
                                    target: target.clone(),
                                    text: view.correction.read(cx).value().to_string(),
                                };
                                if let Err(error) = correction.validate() {
                                    view.state.error = Some(error.to_string());
                                    cx.notify();
                                } else {
                                    view.send(MemoryAction::Correct(correction), cx);
                                }
                            })),
                    );
            }
            if self.stopping.as_ref() == Some(&id) {
                panel = panel.child(div().child("Remove this detail and all its corrected versions from memory? Their supporting records will also stop appearing in memory searches. Stopped history stays available for your review. Other details and the original chats are kept."))
                    .child(h_flex().flex_wrap().gap(t.space.small)
                        .child(design::button("memory-confirm-stop", "Confirm removal", cx).disabled(busy)
                            .on_click(cx.listener(move |view, _, _, cx| view.send(MemoryAction::Remove(id.clone()), cx))))
                        .child(design::button("memory-cancel-stop", "Cancel", cx).disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| { view.stopping = None; cx.notify(); }))));
            }
            panel = panel.child(self.connected_details(detail, cx));
            column = column.child(panel);
        }
        if let Some(source) = &self.state.source {
            column = column.child(self.source_reader(source, cx));
        }
        if !self.source_only && self.state.detail.is_none() {
            column = column.child(self.semantic_controls(cx));
        }
        column
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{App, ElementId, ScrollDelta, TestAppContext, point, px, size};
    use sovra_core::{
        Episode, EpisodeId, EpisodeKind, Event, Fact, MemoryCommand, MemoryDetail, MemoryEvent,
        MemoryPage, NavTab, Timestamp, models::InterfaceScale,
    };
    use sovra_runtime::Client;

    fn settle(cx: &mut TestAppContext) {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
    }

    fn request(commands: &async_channel::Receiver<Command>) -> MemoryCommand {
        std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| {
                if let Command::Memory(command) = command {
                    Some(command)
                } else {
                    None
                }
            })
            .expect("memory command")
    }

    fn respond(events: &async_channel::Sender<Event>, request_id: u64, result: MemoryReviewResult) {
        events
            .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                request_id,
                result: Ok(result),
            })))
            .unwrap();
    }

    fn click(window: &mut Window, id: impl Into<ElementId>, height: f32, cx: &mut App) {
        let id = id.into();
        for _ in 0..100 {
            window.render_frame(cx);
            let item = window.find(id.clone());
            let bounds = item.bounds();
            let viewport = window.find("sovra-knowledge-view").bounds();
            assert!(viewport.bottom() <= px(height));
            if item.visible()
                && bounds.top() >= viewport.top()
                && bounds.bottom() <= viewport.bottom()
            {
                window.click(id, cx);
                return;
            }
            let delta = if bounds.top() < viewport.top() {
                viewport.top() - bounds.top() + px(1.)
            } else {
                viewport.bottom() - bounds.bottom() - px(1.)
            };
            window.scroll(
                "sovra-knowledge-view",
                ScrollDelta::Pixels(point(px(0.), delta)),
                cx,
            );
        }
        panic!(
            "memory control {id:?} must be reachable by scrolling: {:?}, viewport {:?}",
            window.find(id.clone()).bounds(),
            window.find("sovra-knowledge-view").bounds()
        );
    }

    fn detail() -> MemoryDetail {
        MemoryDetail {
            neighborhood: sovra_core::MemoryNeighborhood::default(),
            fact: Fact {
                id: FactId("fact:tea".into()),
                content: "I prefer tea".into(),
                entities: vec![],
                source: Some(EpisodeId("episode:tea".into())),
                replaces: None,
                extraction: None,
                created_at: Timestamp(1),
                valid_at: Some(Timestamp(1)),
                invalid_at: None,
                expired_at: None,
                stopped_at: None,
            },
            source: Some(Episode {
                id: EpisodeId("episode:tea".into()),
                session_id: "old-chat".into(),
                kind: EpisodeKind::UserMessage,
                content: "I prefer tea".into(),
                occurred_at: Timestamp(1),
            }),
        }
    }

    fn captured() -> Event {
        Event::Memory(MemoryEvent::Processed(sovra_core::MemoryJob {
            episode_id: EpisodeId("episode:tea".into()),
            state: sovra_core::MemoryJobState::Complete,
            attempts: 1,
            error: None,
            fact_ids: vec![FactId("fact:tea".into())],
            candidates: vec![],
            extraction_model: Some("fixture".into()),
        }))
    }

    #[gpui_kit::test]
    fn newly_captured_memories_refresh_the_open_list(cx: &mut TestAppContext) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = crate::Shell::new(0, window, cx);
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(
                    crate::knowledge_view::KnowledgeSection::Memories,
                    cx,
                );
                shell
            });
            Root::new(shell, window, cx)
        });
        let load = request(&commands);
        respond(
            &events,
            load.request_id,
            MemoryReviewResult::Page(MemoryPage::default()),
        );
        settle(cx);
        events.send_blocking(captured()).unwrap();
        settle(cx);
        let refresh = request(&commands);
        assert!(matches!(refresh.action, MemoryAction::List(_)));
        events.send_blocking(captured()).unwrap();
        settle(cx);
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::Memory(_)))
        );
        respond(
            &events,
            refresh.request_id,
            MemoryReviewResult::Page(MemoryPage::default()),
        );
        settle(cx);
        let refresh = request(&commands);
        respond(
            &events,
            refresh.request_id,
            MemoryReviewResult::Page(MemoryPage {
                facts: vec![detail().fact],
                next: None,
                overview: None,
                chats: Vec::new(),
            }),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("memory-empty").is_none());
            assert!(window.try_find(("memory-open", 0usize)).is_some());
            assert_eq!(
                window.find("memory-capture-result").label(),
                Some("Last check finished. 1 saved detail is ready to review.")
            );
            assert_eq!(
                window.find("memory-count").label(),
                Some("1 saved detail shown")
            );
            assert!(
                window.find(("memory-open", 0usize)).bounds().top()
                    < window.find("memory-semantic").bounds().top()
            );
        })
        .unwrap();
        let before = cx
            .update_window(window.into(), |_, window, cx| {
                window.scroll(
                    "sovra-knowledge-view",
                    ScrollDelta::Pixels(point(px(0.), px(-80.))),
                    cx,
                );
                window.render_frame(cx);
                let top = window.find("conversation-memory-status").bounds().top();
                assert!(top < window.find("sovra-knowledge-view").bounds().top());
                top
            })
            .unwrap();
        events.send_blocking(captured()).unwrap();
        settle(cx);
        let refresh = request(&commands);
        respond(
            &events,
            refresh.request_id,
            MemoryReviewResult::Page(MemoryPage {
                facts: vec![detail().fact],
                next: None,
                overview: None,
                chats: Vec::new(),
            }),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("conversation-memory-status").bounds().top(),
                before,
                "a background save must not jump to the top of the reader"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn capture_results_distinguish_empty_success_and_failure(cx: &mut TestAppContext) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = crate::Shell::new(0, window, cx);
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(
                    crate::knowledge_view::KnowledgeSection::Memories,
                    cx,
                );
                shell
            });
            Root::new(shell, window, cx)
        });
        let load = request(&commands);
        respond(
            &events,
            load.request_id,
            MemoryReviewResult::Page(MemoryPage::default()),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("memory-capture-result").is_none());
            assert!(
                window
                    .find("memory-empty")
                    .label()
                    .unwrap()
                    .contains("Add a detail")
            );
        })
        .unwrap();
        let Event::Memory(MemoryEvent::Processed(mut job)) = captured() else {
            unreachable!()
        };
        job.fact_ids.clear();
        events
            .send_blocking(Event::Memory(MemoryEvent::Processed(job.clone())))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("memory-capture-result").label(),
                Some("Last check finished. No new detail was saved.")
            );
            assert!(window.try_find("memory-count").is_none());
        })
        .unwrap();
        job.state = sovra_core::MemoryJobState::Failed;
        job.error = Some("The selected AI could not be reached.".into());
        events
            .send_blocking(Event::Memory(MemoryEvent::Processed(job)))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("memory-capture-result").label(),
                Some("A message could not be remembered. The selected AI could not be reached.")
            );
            assert!(window.try_find("memory-count").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn saved_memory_controls_work_through_the_shell_at_every_size(cx: &mut TestAppContext) {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| crate::init(client, cx));
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| crate::Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            events
                .send_blocking(Event::Models(sovra_core::models::ModelEvent::Loaded {
                    connections: vec![],
                    preferences: sovra_core::models::ModelPreferences::default(),
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                shell.update(cx, |shell, cx| {
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Memories,
                        cx,
                    );
                });
                window.render_frame(cx);
            })
            .unwrap();
            let load = request(&commands);
            assert!(matches!(load.action, MemoryAction::List(_)));
            respond(
                &events,
                load.request_id,
                MemoryReviewResult::Page(MemoryPage {
                    facts: vec![detail().fact],
                    next: None,
                    overview: None,
                    chats: Vec::new(),
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                for theme in [ThemeMode::Light, ThemeMode::Dark] {
                    Theme::change(theme, Some(window), cx);
                    window.render_frame(cx);
                    let bounds = window.find("memory-browser").bounds();
                    assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                }
                click(window, "knowledge-memory-toggle", height, cx);
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::Models(sovra_core::models::ModelCommand::SetAgentSettings { settings }) if !settings.memory_enabled
                )));
                click(window, ("memory-open", 0usize), height, cx);
            })
            .unwrap();
            let inspect = request(&commands);
            assert_eq!(
                inspect.action,
                MemoryAction::Inspect(FactId("fact:tea".into()))
            );
            respond(
                &events,
                inspect.request_id,
                MemoryReviewResult::Detail(Box::new(detail())),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                click(window, "memory-edit", height, cx);
                let view = shell.read(cx).memory_view.clone();
                view.update(cx, |view, cx| {
                    view.correction.update(cx, |input, cx| {
                        input.set_value("I prefer jasmine tea", window, cx);
                    });
                });
            })
            .unwrap();
            events.send_blocking(captured()).unwrap();
            settle(cx);
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|command| matches!(command, Command::Memory(_)))
            );
            cx.update_window(window.into(), |_, window, cx| {
                let view = shell.read(cx).memory_view.clone();
                assert_eq!(
                    view.read(cx).correction.read(cx).value().as_ref(),
                    "I prefer jasmine tea"
                );
                for theme in [ThemeMode::Light, ThemeMode::Dark] {
                    Theme::change(theme, Some(window), cx);
                    window.render_frame(cx);
                    for id in ["memory-new-details", "memory-capture-result"] {
                        let bounds = window.find(id).bounds();
                        assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                        assert!(bounds.size.height > px(0.));
                    }
                }
                click(window, "memory-save", height, cx);
            })
            .unwrap();
            let edit = request(&commands);
            assert_eq!(
                edit.action,
                MemoryAction::Correct(MemoryCorrection {
                    target: FactId("fact:tea".into()),
                    text: "I prefer jasmine tea".into()
                })
            );
            respond(
                &events,
                edit.request_id,
                MemoryReviewResult::Changed(FactId("fact:tea".into())),
            );
            settle(cx);
            let reload = request(&commands);
            respond(
                &events,
                reload.request_id,
                MemoryReviewResult::Page(MemoryPage {
                    facts: vec![detail().fact],
                    next: None,
                    overview: None,
                    chats: Vec::new(),
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                click(window, ("memory-open", 0usize), height, cx);
            })
            .unwrap();
            let inspect = request(&commands);
            let mut previous = detail();
            previous.fact.invalid_at = Some(Timestamp(2));
            previous.fact.expired_at = Some(Timestamp(2));
            respond(
                &events,
                inspect.request_id,
                MemoryReviewResult::Detail(Box::new(previous)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                assert_eq!(window.find_all("memory-edit").len(), 0);
                click(window, "memory-stop", height, cx);
                assert!(
                    !std::iter::from_fn(|| commands.try_recv().ok())
                        .any(|command| matches!(command, Command::Memory(_))),
                    "opening confirmation must not change memory"
                );
                click(window, "memory-confirm-stop", height, cx);
            })
            .unwrap();
            assert_eq!(
                request(&commands).action,
                MemoryAction::Remove(FactId("fact:tea".into()))
            );
            cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
        }
    }
}
