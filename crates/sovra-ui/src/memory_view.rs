//! Live conversation memories, retained sources, and explicit user corrections.

use gpui_kit::component::{
    Disableable as _, Selectable as _, h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
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

pub(crate) struct MemoryView {
    state: MemoryReviewState,
    models: Entity<ModelsView>,
    embedding_search: Entity<InputState>,
    choosing: bool,
    _embedding_search: gpui_kit::Subscription,
    query: MemoryListQuery,
    search: Entity<InputState>,
    correction: Entity<TextareaState>,
    editing: Option<FactId>,
    stopping: Option<FactId>,
    source_only: bool,
    new_memories: bool,
    capture_result: Option<String>,
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
        Self {
            models,
            embedding_search,
            choosing: false,
            _embedding_search: subscription,
            state: MemoryReviewState::default(),
            query: MemoryListQuery::default(),
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search saved details")),
            correction: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 5)),
            editing: None,
            stopping: None,
            source_only: false,
            new_memories: false,
            capture_result: None,
        }
    }

    fn send(&mut self, action: MemoryAction, cx: &mut Context<Self>) {
        let command = self.state.begin(action);
        let request_id = command.request_id;
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
        self.new_memories = false;
        self.source_only = false;
        self.query.search = self.search.read(cx).value().to_string();
        self.query.before = None;
        self.editing = None;
        self.stopping = None;
        self.send(MemoryAction::List(self.query.clone()), cx);
    }

    pub(crate) fn source_only(&self) -> bool {
        self.source_only
    }

    pub(crate) fn captured(
        &mut self,
        job: &sovra_core::MemoryJob,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(result) = job.result_label() {
            self.capture_result = Some(result);
            cx.notify();
        }
        if job.state == sovra_core::MemoryJobState::Complete && !job.fact_ids.is_empty() {
            self.new_memories = true;
            self.refresh_captured(visible, cx);
            cx.notify();
        }
    }

    pub(crate) fn refresh_captured(&mut self, visible: bool, cx: &mut Context<Self>) {
        // A background save must not discard a correction, move an older page,
        // or apply search text that the user is still typing.
        if self.new_memories
            && visible
            && !self.source_only
            && !self.state.loading
            && self.state.detail.is_none()
            && self.editing.is_none()
            && self.stopping.is_none()
            && self.query.before.is_none()
            && self.search.read(cx).value().as_ref() == self.query.search
        {
            self.load(cx);
        }
    }

    pub(crate) fn open_source(&mut self, id: FactId, cx: &mut Context<Self>) {
        self.source_only = true;
        self.state.detail = None;
        self.state.page = sovra_core::MemoryPage::default();
        self.editing = None;
        self.stopping = None;
        self.send(MemoryAction::Inspect(id), cx);
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
                    .child("Find memories by meaning"),
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

    pub(crate) fn apply(&mut self, event: &MemoryReviewEvent, cx: &mut Context<Self>) {
        if self.state.apply(event) {
            if matches!(event.result, Ok(MemoryReviewResult::Changed(_))) {
                self.load(cx);
            }
            cx.notify();
        }
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
            .sovra_surface(Surface::Matte, cx)
            .p(t.space.medium)
            .gap(t.space.small);
        if !self.source_only {
            column = column
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("What I remember"))
            .child(div().sovra_text(TextRole::Caption, cx).child("Corrections keep previous versions. Stop using excludes that version from current and historical recall; its record and original conversation remain."))
            .child(Input::new(&self.search).aria_label("Search saved memories"))
            .child(h_flex().w_full().flex_wrap().gap(t.space.small)
                .child(design::button("memory-search", "Search / refresh", cx).disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| view.load(cx))))
                .child(design::button("memory-history", "Include history", cx).selected(self.query.include_history).disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.query.include_history = !view.query.include_history;
                        view.load(cx);
                    }))));
            if let Some(result) = &self.capture_result {
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
            if self.new_memories {
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
            && self.state.page.facts.is_empty()
            && !busy
            && self.state.error.is_none()
        {
            let message = if self.query.before.is_some() {
                "No more saved details on this page. Use Back to newest to return to recent memories."
            } else if self.query.search.is_empty() {
                "No saved details yet. Start a conversation and share something you want Sovra to remember, such as ‘I prefer tea without sugar.’ Keep remembering switched on above."
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
        for (index, fact) in self
            .state
            .page
            .facts
            .iter()
            .enumerate()
            .filter(|_| self.state.detail.is_none() && !self.source_only)
        {
            let id = fact.id.clone();
            column =
                column.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(t.space.small)
                        .p(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child(fact.content.clone()),
                        )
                        .child(div().sovra_text(TextRole::Caption, cx).child(
                            if fact.is_current() {
                                if fact.extraction.is_some() {
                                    "Interpretation of something you said"
                                } else {
                                    "Saved detail"
                                }
                            } else {
                                "Retained history · not used in current recall"
                            },
                        ))
                        .child(
                            design::button(("memory-open", index), "Source and changes", cx)
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.editing = None;
                                    view.stopping = None;
                                    view.send(MemoryAction::Inspect(id.clone()), cx);
                                })),
                        ),
                );
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
                        .child("Source and changes"),
                )
                .child(div().child(detail.fact.content.clone()))
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
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.state.detail = None;
                            view.editing = None;
                            view.stopping = None;
                            view.load(cx);
                        })),
                );
            if let Some(source) = &detail.source {
                let source_label = match source.kind {
                    sovra_core::EpisodeKind::UserMessage => "Your original message",
                    sovra_core::EpisodeKind::Note => "Your saved note or correction",
                    sovra_core::EpisodeKind::AgentMessage => {
                        "An earlier reply (not a user statement)"
                    }
                    sovra_core::EpisodeKind::ToolCall => "An earlier tool request",
                    sovra_core::EpisodeKind::ToolResult => "An earlier tool result",
                };
                panel = panel
                    .child(div().sovra_text(TextRole::Caption, cx).child(source_label))
                    .child(
                        div()
                            .id("memory-source")
                            .test_support()
                            .aria_label(source.content.clone())
                            .child(source.content.clone()),
                    );
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
            if detail.fact.is_current() {
                let text = detail.fact.content.clone();
                let edit_id = id.clone();
                let stop_id = id.clone();
                panel = panel.child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::button("memory-edit", "Correct", cx)
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
                        .child(
                            design::button("memory-stop", "Stop using", cx)
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
                panel = panel.child(div().child("Stop using this saved version in current and historical answers? Its record stays in history. Other versions remain available, and the original conversation keeps its own context."))
                    .child(h_flex().flex_wrap().gap(t.space.small)
                        .child(design::button("memory-confirm-stop", "Confirm stop using", cx).disabled(busy)
                            .on_click(cx.listener(move |view, _, _, cx| view.send(MemoryAction::StopUsing(id.clone()), cx))))
                        .child(design::button("memory-cancel-stop", "Cancel", cx).disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| { view.stopping = None; cx.notify(); }))));
            }
            column = column.child(panel);
        }
        if !self.source_only {
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
                let mut shell = crate::Shell::new(window, cx);
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
    }

    #[gpui_kit::test]
    fn capture_results_distinguish_empty_success_and_failure(cx: &mut TestAppContext) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = crate::Shell::new(window, cx);
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
                    .contains("Start a conversation")
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
                let view = cx.new(|cx| crate::Shell::new(window, cx));
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
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                click(window, ("memory-open", 0usize), height, cx);
            })
            .unwrap();
            let inspect = request(&commands);
            respond(
                &events,
                inspect.request_id,
                MemoryReviewResult::Detail(Box::new(detail())),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
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
                MemoryAction::StopUsing(FactId("fact:tea".into()))
            );
            cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
        }
    }
}
