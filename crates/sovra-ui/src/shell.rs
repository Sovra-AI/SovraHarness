//! The main window's root view: title bar, Login / 3-column Main Chat workspace, and status bar.

use gpui_kit::component::{
    ActiveTheme as _, TitleBar, h_flex,
    input::{InputEvent, InputState, TextareaState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, TestSupportExt as _, Window, div, img,
};
use sovra_core::{
    ChatHistoryFilter, Command, DecisionBackend, ModelTier, NavTab, RuntimeStatus, ScreenRoute,
    SelectionMode, WorkspaceState,
};

use crate::RuntimeClient;
use crate::chat_history_view::render_chat_history_rail;
use crate::chat_view::{ChatPresentation, render_chat_column};
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::inspector::render_inspector;
use crate::login::render_login_screen;
use crate::my_ais_view::render_my_ais_view;
use crate::schedules_view::render_schedules_view;
use crate::sidebar::render_sidebar;

/// How long the UI waits to coalesce streamed tokens into one update.
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent view flags; a state enum would be artificial"
)]
pub struct Shell {
    pub(crate) state: WorkspaceState,
    pub(crate) chat: ChatPresentation,
    focus_handle: FocusHandle,
    history_compact: bool,
    history_overlay_open: bool,
    composer_input: Entity<TextareaState>,
    search_input: Entity<InputState>,
    history_search_input: Entity<InputState>,
    rename_thread_input: Entity<InputState>,
    schedule_name_input: Entity<InputState>,
    schedule_cadence_input: Entity<InputState>,
    schedule_prompt_input: Entity<InputState>,
    schedule_form_open: bool,
    login_email_input: Entity<InputState>,
    login_password_input: Entity<InputState>,
    sim_route_input: Entity<InputState>,
    my_ais_advanced_open: bool,
    models: Entity<crate::ModelsView>,
    /// Redraws once a second while a turn runs, for the elapsed timer.
    turn_ticker: Option<Task<()>>,
    _events: Task<()>,
    _composer_events: Subscription,
    _history_search_events: Subscription,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("status", &self.state.runtime_status)
            .field("route", &self.state.route)
            .field("active_tab", &self.state.active_tab)
            .finish_non_exhaustive()
    }
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Root's native Tab bindings need an initial focus path before any input
        // has been clicked. The shell itself stays outside the tab order.
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let composer_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message Sovra…")
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });
        let composer_events = cx.subscribe_in(
            &composer_input,
            window,
            |shell, _, event, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => shell.submit_composer(window, cx),
                InputEvent::Change => cx.notify(),
                InputEvent::Focus | InputEvent::Blur => {
                    shell.chat.composer_focused = matches!(event, InputEvent::Focus);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {}
            },
        );
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search anything..."));
        let history_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let history_search_events = cx.subscribe_in(
            &history_search_input,
            window,
            |shell, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    shell.state.set_history_search(&query);
                    cx.notify();
                }
            },
        );
        let rename_thread_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Conversation title"));
        let schedule_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Morning Market Check"));
        let schedule_cadence_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Daily · 08:00 GST"));
        let schedule_prompt_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Summarize overnight market moves and portfolio alerts.")
        });
        let login_email_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("ahmed@sovra.ai"));
        let login_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Enter your password")
                .masked(true)
        });
        let sim_route_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type any prompt to test Laya / OpenJev routing tree...")
        });

        let events = cx.global::<RuntimeClient>().0.events();
        let models = cx.new(|cx| crate::ModelsView::new(window, cx));
        let task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                // Streamed tokens arrive far faster than frames. Wait out one frame
                // (~8ms, imperceptible) and fold everything that queued meanwhile into
                // a single update, so each frame costs at most one state change, one
                // Markdown append and one background parse. Other events apply at once.
                if matches!(
                    event,
                    sovra_core::Event::AgentChunk { .. }
                        | sovra_core::Event::AgentThinkingChunk { .. }
                ) {
                    cx.background_executor().timer(FRAME_BUDGET).await;
                }
                let mut batch = vec![event];
                for _ in 1..1024 {
                    let Ok(event) = events.try_recv() else {
                        break;
                    };
                    match (&event, batch.last_mut()) {
                        (
                            sovra_core::Event::AgentChunk { session_id, text },
                            Some(sovra_core::Event::AgentChunk {
                                session_id: previous_session,
                                text: previous_text,
                            }),
                        ) if previous_session == session_id => previous_text.push_str(text),
                        (
                            sovra_core::Event::AgentThinkingChunk {
                                session_id,
                                text,
                                elapsed_ms,
                            },
                            Some(sovra_core::Event::AgentThinkingChunk {
                                session_id: previous_session,
                                text: previous_text,
                                elapsed_ms: previous_elapsed,
                            }),
                        ) if previous_session == session_id => {
                            previous_text.push_str(text);
                            *previous_elapsed = *elapsed_ms;
                        }
                        _ => batch.push(event),
                    }
                }
                let applied = this.update(cx, |shell, cx| {
                    let mut should_persist_thread = false;
                    for event in batch {
                        if let sovra_core::Event::Models(event) = &event {
                            shell
                                .models
                                .update(cx, |models, cx| models.apply(event, cx));
                        }
                        if matches!(
                            event,
                            sovra_core::Event::AgentCompleted { .. }
                                | sovra_core::Event::AgentError { .. }
                                | sovra_core::Event::AgUiSurface { .. }
                        ) {
                            should_persist_thread = true;
                        }
                        if let Some(next_prompt) = shell.state.apply_event(&event) {
                            shell.dispatch_runtime_prompt(next_prompt, cx);
                            should_persist_thread = true;
                        }
                    }
                    if should_persist_thread {
                        shell.persist_active_thread(cx);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break; // view dropped
                }
            }
        });
        Self {
            state: WorkspaceState::default(),
            chat: ChatPresentation::new(cx),
            _composer_events: composer_events,
            _history_search_events: history_search_events,
            focus_handle,
            history_compact: window.viewport_size().width
                < design::tokens(cx).layout.history_breakpoint,
            history_overlay_open: false,
            composer_input,
            search_input,
            history_search_input,
            rename_thread_input,
            schedule_name_input,
            schedule_cadence_input,
            schedule_prompt_input,
            schedule_form_open: false,
            login_email_input,
            login_password_input,
            sim_route_input,
            my_ais_advanced_open: false,
            models,
            turn_ticker: None,
            _events: task,
        }
    }

    pub fn status(&self) -> &RuntimeStatus {
        &self.state.runtime_status
    }

    pub fn workspace(&self) -> &WorkspaceState {
        &self.state
    }

    pub fn select_tab(&mut self, tab: NavTab, cx: &mut Context<Self>) {
        self.state.select_tab(tab);
        self.models.update(cx, |models, cx| {
            models.settings_open = tab == NavTab::Settings;
            cx.notify();
        });
        if matches!(tab, NavTab::Models | NavTab::Settings) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::Models(sovra_core::models::ModelCommand::Load));
        }
        cx.notify();
    }

    pub fn models(&self) -> Entity<crate::ModelsView> {
        self.models.clone()
    }

    pub fn set_routing_mode(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        self.state.set_routing_mode(mode);
        self.sync_routing_to_runtime(cx);
        cx.notify();
    }

    pub fn set_decision_backend(&mut self, backend: DecisionBackend, cx: &mut Context<Self>) {
        self.state.set_decision_backend(backend);
        self.sync_routing_to_runtime(cx);
        cx.notify();
    }

    pub fn cycle_specialist_tier(
        &mut self,
        specialist_id: &str,
        next_tier: ModelTier,
        cx: &mut Context<Self>,
    ) {
        let model_override = self.state.routing.global_model.clone();
        self.state
            .set_specialist_model(specialist_id, model_override.clone(), next_tier);
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::SetSpecialistModel {
                specialist_id: specialist_id.to_owned(),
                model_override,
                preferred_tier: next_tier,
            });
        cx.notify();
    }

    pub fn toggle_my_ais_advanced(&mut self, cx: &mut Context<Self>) {
        self.my_ais_advanced_open = !self.my_ais_advanced_open;
        cx.notify();
    }

    pub fn simulate_route_from_input(&mut self, cx: &mut Context<Self>) {
        let raw = self.sim_route_input.read(cx).value().to_string();
        let prompt = if raw.trim().is_empty() {
            "Debug and refactor this Rust trait implementation".to_owned()
        } else {
            raw
        };
        let _ = self.state.simulate_route_offline(&prompt);
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::SimulateRoute { prompt });
        cx.notify();
    }

    fn sync_routing_to_runtime(&self, cx: &mut Context<Self>) {
        let _ = cx.global::<RuntimeClient>().0.send(Command::UpdateRouting {
            mode: self.state.routing.mode,
            backend: self.state.routing.backend,
            global_model: self.state.routing.global_model.clone(),
        });
    }

    pub fn submit_login(&mut self, cx: &mut Context<Self>) {
        let email = self.login_email_input.read(cx).value().to_string();
        self.state.login(&email);
        cx.notify();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        self.state.logout();
        cx.notify();
    }

    pub fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.composer_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_prompt_text(&text, window, cx);
    }

    pub fn fill_composer_draft(
        &mut self,
        draft: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owned = draft.to_owned();
        self.composer_input.update(cx, |input, cx| {
            input.set_value(owned, window, cx);
        });
        cx.notify();
    }

    pub fn send_prompt_text(&mut self, prompt: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if !prompt.trim().is_empty() {
            self.chat.follow_latest();
        }
        if let Some(dispatched) = self.state.submit_prompt(prompt) {
            self.dispatch_runtime_prompt(dispatched, cx);
        }
        self.persist_active_thread(cx);
        cx.notify();
    }

    pub fn start_new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_overlay_open = false;
        self.persist_active_thread(cx);
        let _ = self.state.start_new_chat();
        self.composer_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    pub fn select_thread(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.persist_active_thread(cx);
        if self.state.select_thread(session_id) {
            self.history_overlay_open = false;
            self.chat.follow_latest();
        }
        cx.notify();
    }

    pub fn set_history_filter(&mut self, filter: ChatHistoryFilter, cx: &mut Context<Self>) {
        self.state.set_history_filter(filter);
        cx.notify();
    }

    pub fn toggle_history_rail(&mut self, cx: &mut Context<Self>) {
        if self.history_compact {
            self.history_overlay_open = !self.history_overlay_open;
        } else {
            self.state.toggle_history_collapsed();
        }
        cx.notify();
    }

    pub fn toggle_thread_pin(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some(updated) = self.state.toggle_thread_pin(session_id) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::SaveChatThread {
                    thread: Box::new(updated),
                });
        }
        cx.notify();
    }

    pub fn begin_rename_thread(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(current_title) = self.state.begin_rename_thread(session_id) {
            self.rename_thread_input.update(cx, |input, cx| {
                input.set_value(current_title, window, cx);
            });
        }
        cx.notify();
    }

    pub fn commit_rename_thread(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let draft = self.rename_thread_input.read(cx).value().to_string();
        if let Some(updated) = self.state.rename_thread(session_id, &draft) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::SaveChatThread {
                    thread: Box::new(updated),
                });
        }
        cx.notify();
    }

    pub fn delete_thread(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let owned = session_id.to_owned();
        if self.state.delete_thread(&owned) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::DeleteChatThread { session_id: owned });
        }
        cx.notify();
    }

    pub fn toggle_schedule_form(&mut self, cx: &mut Context<Self>) {
        self.schedule_form_open = !self.schedule_form_open;
        cx.notify();
    }

    pub fn open_schedule_form(&mut self, cx: &mut Context<Self>) {
        self.schedule_form_open = true;
        cx.notify();
    }

    pub fn submit_new_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.schedule_name_input.read(cx).value().to_string();
        let cadence = self.schedule_cadence_input.read(cx).value().to_string();
        let prompt = self.schedule_prompt_input.read(cx).value().to_string();
        if self
            .state
            .create_schedule(&name, &cadence, &prompt)
            .is_some()
        {
            self.schedule_name_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            self.schedule_cadence_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            self.schedule_prompt_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            self.schedule_form_open = false;
        }
        cx.notify();
    }

    pub fn delete_schedule(&mut self, schedule_id: &str, cx: &mut Context<Self>) {
        self.state.delete_schedule(schedule_id);
        cx.notify();
    }

    pub(crate) fn persist_active_thread(&self, cx: &mut Context<Self>) {
        if let Some(thread) = self.state.active_thread()
            && !thread.messages.is_empty()
        {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::SaveChatThread {
                    thread: Box::new(thread.clone()),
                });
        }
    }

    pub fn interrupt_active_session(&mut self, cx: &mut Context<Self>) {
        let session_id = self.state.session_id.clone();
        self.state.interrupt_generation();
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::InterruptSession { session_id });
        cx.notify();
    }

    pub fn preview_personal_ai(&mut self, cx: &mut Context<Self>) {
        self.state.activate_preview_ai();
        cx.notify();
    }

    pub fn handle_config_card_click(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "model" => self.select_tab(NavTab::MyAis, cx),
            "deployment" => self.state.activate_preview_ai(),
            "knowledge" => self.state.select_tab(NavTab::Knowledge),
            _ => {}
        }
        cx.notify();
    }

    pub fn toggle_schedule(&mut self, schedule_id: &str, cx: &mut Context<Self>) {
        self.state.toggle_schedule(schedule_id);
        cx.notify();
    }

    pub fn run_schedule_in_chat(
        &mut self,
        schedule_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dispatched) = self.state.trigger_schedule(schedule_id) {
            self.dispatch_runtime_prompt(dispatched, cx);
            self.persist_active_thread(cx);
        }
        cx.notify();
    }

    fn dispatch_runtime_prompt(&self, prompt: String, cx: &mut Context<Self>) {
        let session_id = self.state.session_id.clone();
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::RunPrompt { session_id, prompt });
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < design::tokens(cx).layout.history_breakpoint;
        self.chat.viewport_width = window.viewport_size().width;
        if self.history_compact != compact {
            self.history_compact = compact;
            self.history_overlay_open = false;
        }
        let theme = cx.theme();
        let border = theme.border;
        let muted_foreground = theme.muted_foreground;
        let t = design::tokens(cx);
        v_flex()
            .id("sovra-shell")
            .track_focus(&self.focus_handle)
            .tab_stop(false)
            .size_full()
            .sovra_surface(Surface::Canvas, cx)
            .sovra_text(TextRole::Body, cx)
            .child(TitleBar::new())
            .child(self.render_body(cx))
            .child(
                h_flex()
                    .h(t.layout.status_height)
                    .flex_shrink_0()
                    .px(t.space.small)
                    .justify_between()
                    .items_center()
                    .sovra_surface(Surface::Chrome, cx)
                    .border_t_1()
                    .border_color(border)
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(muted_foreground)
                    .child(match &self.state.runtime_status {
                        RuntimeStatus::Starting => "Connecting…",
                        RuntimeStatus::Ready { .. } => "Ready",
                        RuntimeStatus::Stopped => "Disconnected",
                    })
                    .child(
                        self.state
                            .routing
                            .global_model
                            .as_ref()
                            .map_or("Sovra".to_owned(), |model| model.model_id.clone()),
                    ),
            )
    }
}

impl Shell {
    fn ensure_turn_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_generating {
            self.turn_ticker = None;
            return;
        }
        if self.turn_ticker.is_some() {
            return;
        }
        self.turn_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let running = this.update(cx, |shell, cx| {
                    if shell.state.is_generating {
                        cx.notify();
                    }
                    shell.state.is_generating
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.ensure_turn_ticker(cx);
        match self.state.route {
            ScreenRoute::Login => {
                render_login_screen(&self.login_email_input, &self.login_password_input, cx)
                    .into_any_element()
            }
            ScreenRoute::MainWorkspace => {
                let center = match self.state.active_tab {
                    NavTab::Models | NavTab::Settings => div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.models.clone())
                        .into_any_element(),
                    NavTab::MyAis => render_my_ais_view(
                        &self.state,
                        &self.sim_route_input,
                        self.my_ais_advanced_open,
                        cx,
                    )
                    .into_any_element(),
                    NavTab::Schedules => render_schedules_view(
                        &self.state,
                        &self.schedule_name_input,
                        &self.schedule_cadence_input,
                        &self.schedule_prompt_input,
                        self.schedule_form_open,
                        cx,
                    )
                    .into_any_element(),
                    NavTab::Chat => {
                        self.chat.sync(&self.state, cx);
                        h_flex()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .overflow_hidden()
                            .child(render_chat_history_rail(
                                &self.state,
                                &self.history_search_input,
                                &self.rename_thread_input,
                                self.history_compact || self.state.history_collapsed,
                                cx,
                            ))
                            .child(render_chat_column(
                                &self.state,
                                &self.composer_input,
                                &self.chat,
                                cx,
                            ))
                            .children((self.history_compact && self.history_overlay_open).then(
                                || {
                                    div().absolute().top_0().left_0().h_full().occlude().child(
                                        render_chat_history_rail(
                                            &self.state,
                                            &self.history_search_input,
                                            &self.rename_thread_input,
                                            false,
                                            cx,
                                        ),
                                    )
                                },
                            ))
                            .into_any_element()
                    }
                    NavTab::Knowledge => {
                        self.chat.sync(&self.state, cx);
                        render_chat_column(&self.state, &self.composer_input, &self.chat, cx)
                            .into_any_element()
                    }
                };

                let mut body = h_flex()
                    .id("sovra-main-workspace")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(render_sidebar(&self.state, cx))
                    .child(center);
                if self.state.active_tab == NavTab::Knowledge {
                    body = body.child(render_inspector(&self.state, &self.search_input, cx));
                }
                body.into_any_element()
            }
        }
    }
}

pub(crate) fn logo(id: &'static str, size: Pixels) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Image)
        .aria_label("Sovra logo")
        .size(size)
        .flex_shrink_0()
        .child(img(crate::assets::SOVRA_LOGO).size_full())
}
