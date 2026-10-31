//! The main window's root view: title bar (when the app draws one), Login / 3-column Main Chat workspace, and status bar.

use gpui_kit::component::{
    ActiveTheme as _, TitleBar, h_flex,
    input::{InputEvent, InputState, TextareaState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, TestSupportExt as _, Window, div, img,
};
use sovra_core::{
    ChatHistoryFilter, Command, DecisionBackend, ModelTier, NavTab, RuntimeStatus, ScreenRoute,
    SelectionMode, WorkspaceState,
};

use crate::RuntimeClient;
use crate::chat_view::{ChatPresentation, render_chat_column};
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::inspector::render_inspector;
use crate::knowledge_view::render_knowledge_view;
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
    inspector_docked: bool,
    pub(crate) composer_input: Entity<TextareaState>,
    pub(crate) skills: sovra_core::skills::SkillCatalog,
    pub(crate) skill_menu: crate::skill_menu::MenuState,
    pub(crate) skill_library: crate::skill_library::SkillLibrary,
    pub(crate) file_index: crate::file_index_view::FileIndexView,
    pub(crate) memory_view: Entity<crate::memory_view::MemoryView>,
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
    pub(crate) my_ais_advanced_open: bool,
    pub(crate) knowledge_times_open: bool,
    pub(crate) knowledge_section: crate::knowledge_view::KnowledgeSection,
    pub(crate) knowledge_scroll: gpui_kit::ScrollHandle,
    models: Entity<crate::ModelsView>,
    team_view: Entity<crate::team_view::TeamView>,
    artifacts_view: Entity<crate::artifacts_view::ArtifactsView>,
    /// Shown in Settings → Connections; reads Google state from this shell.
    google_card: Entity<crate::google_card::GoogleCard>,
    /// The conversation, rendered as a cached view: composer cursor blinks, sidebar
    /// and timer updates reuse its last frame instead of laying out every message.
    pub(crate) transcript: Entity<crate::chat_view::Transcript>,
    /// Redraws once a second while a turn runs, for the elapsed timer.
    turn_ticker: Option<Task<()>>,
    /// Last title handed to the OS, so it is only sent when the page changes.
    window_title: String,
    file_picker: Option<Task<()>>,
    next_attachment_id: u64,
    _events: Task<()>,
    _composer_events: Subscription,
    _history_search_events: Subscription,
    _models_events: Subscription,
    _models_navigation: Subscription,
    _models_tour: Subscription,
    _team_ask: Subscription,
    _try_asking: Subscription,
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
                InputEvent::Change => {
                    shell.skill_menu = crate::skill_menu::MenuState::default();
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => {
                    shell.chat.composer_focused = matches!(event, InputEvent::Focus);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {}
            },
        );
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search anything..."));
        let skill_library = crate::skill_library::SkillLibrary::new(window, cx);
        let file_index = crate::file_index_view::FileIndexView::new(window, cx);
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
            cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com"));
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
        let memory_view =
            cx.new(|cx| crate::memory_view::MemoryView::new(models.clone(), window, cx));
        let models_navigation = cx.subscribe_in(
            &models,
            window,
            |shell, _, _: &crate::models_view::GoToChat, window, cx| {
                shell.select_tab(NavTab::Chat, cx);
                shell
                    .composer_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            },
        );
        let models_tour = cx.subscribe(
            &models,
            |shell, _, _: &crate::models_view::StartSystemTour, cx| {
                shell.start_system_tour(cx);
            },
        );
        let models_events = cx.observe(&models, |shell, models, cx| {
            let settings_open = models.read(cx).settings_open;
            if settings_open && shell.state.active_tab == NavTab::Models {
                shell.state.select_tab(NavTab::Settings);
                cx.notify();
            } else if !settings_open && shell.state.active_tab == NavTab::Settings {
                shell.state.select_tab(NavTab::Models);
                cx.notify();
            }
        });
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
                        let memory_visible = shell.state.active_tab == NavTab::Knowledge
                            && shell.knowledge_section
                                == crate::knowledge_view::KnowledgeSection::Memories;
                        if let sovra_core::Event::Memory(sovra_core::MemoryEvent::Processed(job)) =
                            &event
                        {
                            shell
                                .memory_view
                                .update(cx, |view, cx| view.captured(job, memory_visible, cx));
                        }
                        if let sovra_core::Event::Memory(sovra_core::MemoryEvent::Semantic(
                            status,
                        )) = &event
                        {
                            shell
                                .memory_view
                                .update(cx, |view, cx| view.apply_semantic(status, cx));
                        }
                        if let sovra_core::Event::Memory(sovra_core::MemoryEvent::Review(event)) =
                            &event
                        {
                            shell.memory_view.update(cx, |view, cx| {
                                view.apply(event, cx);
                                view.refresh_captured(memory_visible, cx);
                            });
                        }
                        if let sovra_core::Event::FileIndex(event) = &event {
                            shell.file_index.apply(event);
                        }
                        if let sovra_core::Event::Skills(event) = &event {
                            shell.skill_library.notice = match shell.skills.apply(event) {
                                Ok(issues) => issues.join("\n"),
                                Err(error) => error.to_string(),
                            };
                        }
                        if let sovra_core::Event::FilePrepared(prepared) = &event {
                            let applied = shell
                                .state
                                .file_drafts
                                .get_mut(&prepared.session_id)
                                .is_some_and(|draft| draft.apply(prepared));
                            if !applied {
                                let _ = cx.global::<RuntimeClient>().0.send(Command::Files(
                                    sovra_core::files::FileCommand::Discard {
                                        id: prepared.id.clone(),
                                    },
                                ));
                            }
                        }
                        if let sovra_core::Event::Models(event) = &event {
                            shell
                                .models
                                .update(cx, |models, cx| models.apply(event, cx));
                        }
                        if let sovra_core::Event::Connections(event) = &event {
                            shell.models.update(cx, |models, cx| {
                                models.connections.apply(event);
                                cx.notify();
                            });
                        }
                        if matches!(
                            event,
                            sovra_core::Event::AgentCompleted { .. }
                                | sovra_core::Event::AgentError { .. }
                                | sovra_core::Event::AgUiSurface { .. }
                                | sovra_core::Event::ChatTitleGenerated { .. }
                        ) {
                            should_persist_thread = true;
                        }
                        let completed_session = match &event {
                            sovra_core::Event::AgentCompleted { session_id } => {
                                Some(session_id.clone())
                            }
                            _ => None,
                        };
                        if let sovra_core::Event::Artifacts(
                            sovra_core::artifacts::ArtifactEvent::ReadyToOpen { path, .. },
                        ) = &event
                        {
                            cx.open_url(&format!("file://{path}"));
                        }
                        // Google's consent page opens in the person's own browser.
                        if let sovra_core::Event::Google(
                            sovra_core::google::GoogleEvent::OpenSignIn { url },
                        ) = &event
                        {
                            cx.open_url(url);
                        }
                        if matches!(event, sovra_core::Event::Ready { .. }) {
                            // The roster and library feed chat (@mentions, Saved).
                            let client = &cx.global::<RuntimeClient>().0;
                            let _ =
                                client.send(Command::Teams(sovra_core::teams::TeamCommand::Load));
                            let _ = client.send(Command::Artifacts(
                                sovra_core::artifacts::ArtifactCommand::Load,
                            ));
                            let _ = client.send(Command::Connections(
                                sovra_core::connections::ConnectionCommand::Load,
                            ));
                            // Restores a saved session or gates the app behind Login.
                            let _ = client.send(Command::Auth(sovra_core::AuthCommand::Load));
                        }
                        if let sovra_core::Event::Auth(
                            sovra_core::AuthEvent::GoogleSignInStarted { auth_url },
                        ) = &event
                        {
                            cx.open_url(auth_url);
                        }
                        if let Some(next_prompt) = shell.state.apply_event(&event) {
                            shell.dispatch_runtime_prompt(&next_prompt, cx);
                            should_persist_thread = true;
                        }
                        if let Some(session_id) = completed_session
                            && let Some((sid, prompt, reply)) =
                                shell.state.take_pending_title_request(&session_id)
                        {
                            let _ =
                                cx.global::<RuntimeClient>()
                                    .0
                                    .send(Command::GenerateChatTitle {
                                        session_id: sid,
                                        prompt,
                                        reply,
                                    });
                        }
                    }
                    if should_persist_thread {
                        shell.persist_active_thread(cx);
                    }
                    shell.refresh(cx);
                });
                if applied.is_err() {
                    break; // view dropped
                }
            }
        });
        let shell_handle = cx.entity().downgrade();
        let team_view = {
            let shell = shell_handle.clone();
            cx.new(|cx| crate::team_view::TeamView::new(shell, window, cx))
        };
        let artifacts_view = {
            let shell = shell_handle.clone();
            cx.new(|cx| crate::artifacts_view::ArtifactsView::new(shell, window, cx))
        };
        let google_card = {
            let shell = shell_handle.clone();
            cx.new(|_| crate::google_card::GoogleCard::new(shell))
        };
        models.update(cx, |models, _| {
            models.google_card = Some(google_card.clone());
        });
        let try_asking = cx.subscribe_in(
            &google_card,
            window,
            |shell, _, ask: &crate::google_card::TryAsking, window, cx| {
                shell.select_tab(NavTab::Chat, cx);
                shell.fill_composer_draft(&ask.prompt, window, cx);
            },
        );
        let team_ask = cx.subscribe_in(
            &team_view,
            window,
            |shell, _, ask: &crate::team_view::AskTeam, window, cx| {
                shell.ask_team(ask.member.as_deref(), window, cx);
            },
        );
        Self {
            state: WorkspaceState::default(),
            file_picker: None,
            next_attachment_id: 0,
            chat: ChatPresentation::new(cx),
            _composer_events: composer_events,
            _history_search_events: history_search_events,
            _models_events: models_events,
            _models_navigation: models_navigation,
            _models_tour: models_tour,
            focus_handle,
            history_compact: Self::sidebar_rail(window),
            history_overlay_open: false,
            inspector_docked: design::WidthClass::of_window(window).fits_side_pane(),
            composer_input,
            skills: sovra_core::skills::SkillCatalog::default(),
            skill_menu: crate::skill_menu::MenuState::default(),
            skill_library,
            file_index,
            memory_view,
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
            knowledge_times_open: false,
            knowledge_section: crate::knowledge_view::KnowledgeSection::default(),
            knowledge_scroll: gpui_kit::ScrollHandle::new(),
            models,
            team_view,
            artifacts_view,
            _team_ask: team_ask,
            _try_asking: try_asking,
            google_card,
            turn_ticker: None,
            window_title: String::new(),
            transcript: cx.new(|_| crate::chat_view::Transcript::new(shell_handle)),
            _events: task,
        }
    }

    /// State changed: redraw the shell and the cached conversation view.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.state.file_setup = self
            .state
            .file_drafts
            .get(&self.state.session_id)
            .and_then(sovra_core::files::AttachmentDraft::ready)
            .filter(|files| !files.is_empty())
            .and_then(|files| self.models.read(cx).state.model_for_files(&files).err());
        self.refresh_transcript(cx);
        // Team and Artifacts read the roster and library from this state.
        match self.state.active_tab {
            NavTab::Team => self.team_view.update(cx, |_, cx| cx.notify()),
            NavTab::Artifacts => self.artifacts_view.update(cx, |_, cx| cx.notify()),
            NavTab::Knowledge => self.memory_view.update(cx, |_, cx| cx.notify()),
            NavTab::Settings => self.google_card.update(cx, |_, cx| cx.notify()),
            NavTab::Chat | NavTab::MyAis | NavTab::Models | NavTab::Schedules | NavTab::Skills => {}
        }
        cx.notify();
    }

    /// Only the conversation changed (a parse landed, a timer ticked, it scrolled).
    pub(crate) fn refresh_transcript(&self, cx: &mut Context<Self>) {
        self.transcript.update(cx, |_, cx| cx.notify());
    }

    pub fn status(&self) -> &RuntimeStatus {
        &self.state.runtime_status
    }

    pub fn workspace(&self) -> &WorkspaceState {
        &self.state
    }

    /// Opens one artifact in the Artifacts page (from a chat card).
    pub fn open_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        self.select_tab(NavTab::Artifacts, cx);
        self.artifacts_view.update(cx, |view, cx| view.show(id, cx));
    }

    /// "Ask my team" goes to Chat in team mode; "Ask Priya" pre-fills `@priya`.
    pub(crate) fn ask_team(
        &mut self,
        member: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(NavTab::Chat, cx);
        match member {
            Some(id) => self.fill_composer_draft(&format!("@{id} "), window, cx),
            None => self.state.team_mode = true,
        }
        self.composer_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        self.refresh(cx);
    }

    pub(crate) fn open_memory_source(&mut self, id: sovra_core::FactId, cx: &mut Context<Self>) {
        self.select_tab(NavTab::Knowledge, cx);
        self.memory_view
            .update(cx, |view, cx| view.open_source(id, cx));
    }

    pub fn select_tab(&mut self, tab: NavTab, cx: &mut Context<Self>) {
        self.state.select_tab(tab);
        if tab == NavTab::Knowledge {
            self.memory_view
                .update(cx, crate::memory_view::MemoryView::load);
        }
        self.models.update(cx, |models, cx| {
            models.settings_open = tab == NavTab::Settings;
            models.setup_flow = false;
            if tab == NavTab::Models {
                models.open_model_home(cx);
            }
            cx.notify();
        });
        if matches!(tab, NavTab::Models | NavTab::Settings) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::Models(sovra_core::models::ModelCommand::Load));
        }
        self.refresh(cx);
    }

    pub fn models(&self) -> Entity<crate::ModelsView> {
        self.models.clone()
    }

    pub fn set_routing_mode(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        self.state.set_routing_mode(mode);
        self.sync_routing_to_runtime(cx);
        self.refresh(cx);
    }

    pub fn set_decision_backend(&mut self, backend: DecisionBackend, cx: &mut Context<Self>) {
        self.state.set_decision_backend(backend);
        self.sync_routing_to_runtime(cx);
        self.refresh(cx);
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
        self.refresh(cx);
    }

    pub fn toggle_my_ais_advanced(&mut self, cx: &mut Context<Self>) {
        self.my_ais_advanced_open = !self.my_ais_advanced_open;
        self.refresh(cx);
    }

    pub fn toggle_knowledge_times(&mut self, cx: &mut Context<Self>) {
        self.knowledge_times_open = !self.knowledge_times_open;
        self.refresh(cx);
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
        self.refresh(cx);
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
        let password = self.login_password_input.read(cx).value().to_string();
        // On a fresh install the first submit creates the account; afterwards it
        // signs in. The runtime validates and answers with friendly errors.
        let command = if self.state.auth.has_accounts
            || matches!(self.state.auth.phase, sovra_core::AuthPhase::Unknown)
        {
            sovra_core::AuthCommand::SignIn { email, password }
        } else {
            sovra_core::AuthCommand::CreateAccount {
                email,
                password,
                full_name: String::new(),
            }
        };
        self.state.apply_auth(&sovra_core::AuthEvent::Submitting);
        if let Err(error) = cx.global::<RuntimeClient>().0.send(Command::Auth(command)) {
            self.state.apply_auth(&sovra_core::AuthEvent::Failed {
                field: sovra_core::auth::AuthField::General,
                message: error.to_string(),
            });
        }
        self.refresh(cx);
    }

    /// Opens Google Sign-In: the runtime builds the consent URL and the
    /// loopback listener; the browser opens when it reports back.
    pub fn start_google_signin(&mut self, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Auth(sovra_core::AuthCommand::StartGoogleSignIn));
        cx.notify();
    }

    pub fn cancel_google_signin(&mut self, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Auth(sovra_core::AuthCommand::CancelGoogleSignIn));
        self.state
            .apply_auth(&sovra_core::AuthEvent::GoogleSignInCancelled);
        self.refresh(cx);
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Auth(sovra_core::AuthCommand::SignOut));
        self.state.logout();
        self.refresh(cx);
    }

    pub fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_skill_suggestion(window, cx) {
            return;
        }
        let text = self.composer_input.read(cx).value().to_string();
        match self.skills.resolve(&text) {
            Ok(Some(invocation)) => {
                if let sovra_core::skills::SkillKind::Action(action) = invocation.skill.kind {
                    self.run_skill_action(action, invocation.arguments, window, cx);
                    return;
                }
                // An explicit procedure uses the skill-aware orchestrator even
                // when the preceding conversation was in team mode.
                self.state.team_mode = false;
            }
            Ok(None) => {}
            Err(error) => {
                self.skill_menu.notice = Some(error.to_string());
                cx.notify();
                return;
            }
        }
        if self
            .state
            .file_drafts
            .get(&self.state.session_id)
            .is_some_and(|draft| !draft.files.is_empty())
        {
            if self.state.is_generating {
                return;
            }
            let Some(files) = self
                .state
                .file_drafts
                .get(&self.state.session_id)
                .and_then(sovra_core::files::AttachmentDraft::ready)
            else {
                return;
            };
            if let Err(setup) = self.models.read(cx).state.model_for_files(&files) {
                self.open_file_model_setup(setup.purpose, window, cx);
                return;
            }
            let prompt = if text.trim().is_empty() {
                "Please help me understand these files.".to_owned()
            } else {
                text
            };
            let ids = files.iter().map(|file| file.id.clone()).collect();
            if self.state.submit_file_prompt(&prompt, files) {
                self.state.file_drafts.remove(&self.state.session_id);
                self.composer_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.chat.follow_latest();
                let _ = cx.global::<RuntimeClient>().0.send(Command::RunWithFiles {
                    context: Some(sovra_core::ConversationContext::before_current_turn(
                        &self.state.messages,
                    )),
                    session_id: self.state.session_id.clone(),
                    prompt,
                    attachment_ids: ids,
                });
                self.persist_active_thread(cx);
                self.refresh(cx);
            }
            return;
        }

        if text.trim().is_empty() {
            return;
        }
        self.composer_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_prompt_text(&text, window, cx);
    }

    pub fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session_id = self.state.session_id.clone();
        let original = self.composer_input.read(cx).value().to_string();
        let remaining = self
            .skills
            .resolve(&original)
            .ok()
            .flatten()
            .filter(|invocation| {
                invocation.skill.kind
                    == sovra_core::skills::SkillKind::Action(
                        sovra_core::skills::SkillAction::AddFiles,
                    )
            })
            .map(|invocation| invocation.arguments.to_owned());
        let selection = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add to chat".into()),
        });
        self.file_picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |shell, window, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if !paths.is_empty()
                        && shell.state.session_id == session_id
                        && shell.composer_input.read(cx).value().as_ref() == original
                        && let Some(remaining) = remaining
                    {
                        shell.fill_composer_draft(&remaining, window, cx);
                    }
                    shell.add_files_to_session(&session_id, paths, cx);
                }
                Ok(Ok(None)) => {}
                _ => {
                    shell
                        .state
                        .file_drafts
                        .entry(session_id)
                        .or_default()
                        .notice =
                        Some("The file picker couldn't open. Please try Add files again.".into());
                    shell.refresh(cx);
                }
            });
        }));
    }

    /// Native picker and drop targets share the same command path; no file IO occurs here.
    pub fn add_files(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        self.add_files_to_session(&self.state.session_id.clone(), paths, cx);
    }

    fn add_files_to_session(
        &mut self,
        session_id: &str,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            self.next_attachment_id += 1;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos());
            let id = format!("file-{stamp}-{}", self.next_attachment_id);
            let name = path
                .file_name()
                .map_or_else(|| "File".into(), |name| name.to_string_lossy().into_owned());
            let draft = self.state.file_drafts.entry(session_id.into()).or_default();
            if !draft.begin(id.clone(), name) {
                break;
            }
            if cx
                .global::<RuntimeClient>()
                .0
                .send(Command::Files(sovra_core::files::FileCommand::Prepare {
                    session_id: session_id.into(),
                    id: id.clone(),
                    path,
                }))
                .is_err()
            {
                draft.apply(&sovra_core::files::FilePrepared {
                    session_id: session_id.into(),
                    id,
                    result: Err(sovra_core::files::FileError(
                        "The file reader is unavailable. Please try again.".into(),
                    )),
                });
            }
        }
        self.refresh(cx);
    }

    pub fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(draft) = self.state.file_drafts.get_mut(&self.state.session_id) {
            draft.remove(id);
        }
        let _ = cx.global::<RuntimeClient>().0.send(Command::Files(
            sovra_core::files::FileCommand::Discard { id: id.into() },
        ));
        self.refresh(cx);
    }

    pub fn open_file_model_setup(
        &mut self,
        purpose: sovra_core::files::ModelPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(NavTab::Models, cx);
        self.models.update(cx, |models, cx| {
            models.open_for_purpose(purpose, window, cx);
        });
    }

    pub fn fill_composer_draft(
        &mut self,
        draft: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owned = draft.to_owned();
        let end = owned.len();
        self.skill_menu = crate::skill_menu::MenuState::default();
        self.composer_input.update(cx, |input, cx| {
            input.replace_all(owned, window, cx);
            input.set_selected_range(end..end, cx);
        });
        self.refresh(cx);
    }

    pub fn send_prompt_text(&mut self, prompt: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if !prompt.trim().is_empty() {
            self.chat.follow_latest();
        }
        if let Some(dispatched) = self.state.submit_prompt(prompt) {
            self.dispatch_runtime_prompt(&dispatched, cx);
        }
        self.persist_active_thread(cx);
        self.refresh(cx);
    }

    pub fn start_new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_overlay_open = false;
        self.persist_active_thread(cx);
        let _ = self.state.start_new_chat();
        self.composer_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.refresh(cx);
    }

    pub fn select_thread(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.persist_active_thread(cx);
        if self.state.select_thread(session_id) {
            self.history_overlay_open = false;
            self.chat.follow_latest();
        }
        self.refresh(cx);
    }

    pub fn set_history_filter(&mut self, filter: ChatHistoryFilter, cx: &mut Context<Self>) {
        self.state.set_history_filter(filter);
        self.refresh(cx);
    }

    pub fn toggle_history_rail(&mut self, cx: &mut Context<Self>) {
        if self.history_compact {
            self.history_overlay_open = !self.history_overlay_open;
        } else {
            self.state.toggle_history_collapsed();
        }
        self.refresh(cx);
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
        self.refresh(cx);
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
        self.refresh(cx);
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
        self.refresh(cx);
    }

    pub fn delete_thread(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let owned = session_id.to_owned();
        if self.state.delete_thread(&owned) {
            let _ = cx
                .global::<RuntimeClient>()
                .0
                .send(Command::DeleteChatThread { session_id: owned });
        }
        self.refresh(cx);
    }

    pub fn regenerate_thread_title(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(thread) = self
            .state
            .threads
            .iter()
            .find(|t| t.id == session_id && t.is_active())
        else {
            return;
        };
        let prompt = thread
            .messages
            .iter()
            .find(|m| m.role == sovra_core::MessageRole::User && !m.text.trim().is_empty())
            .map_or_else(|| thread.title.clone(), |m| m.text.clone());
        let reply = thread
            .messages
            .iter()
            .find(|m| m.role == sovra_core::MessageRole::Assistant && !m.text.trim().is_empty())
            .map_or_else(String::new, |m| m.text.clone());
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::GenerateChatTitle {
                session_id: session_id.to_owned(),
                prompt,
                reply,
            });
        self.refresh(cx);
    }

    pub fn toggle_schedule_form(&mut self, cx: &mut Context<Self>) {
        self.schedule_form_open = !self.schedule_form_open;
        self.refresh(cx);
    }

    pub fn open_schedule_form(&mut self, cx: &mut Context<Self>) {
        self.schedule_form_open = true;
        self.refresh(cx);
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
        self.refresh(cx);
    }

    pub fn delete_schedule(&mut self, schedule_id: &str, cx: &mut Context<Self>) {
        self.state.delete_schedule(schedule_id);
        self.refresh(cx);
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
        self.refresh(cx);
    }

    pub fn preview_personal_ai(&mut self, cx: &mut Context<Self>) {
        self.state.activate_preview_ai();
        self.refresh(cx);
    }

    pub fn handle_config_card_click(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "model" => self.select_tab(NavTab::MyAis, cx),
            "deployment" => self.state.activate_preview_ai(),
            "knowledge" => self.state.select_tab(NavTab::Knowledge),
            "memory" | "behaviour" | "guardrails" => self.select_tab(NavTab::Settings, cx),
            _ => {}
        }
        self.refresh(cx);
    }

    pub fn handle_llm_setup_action(
        &mut self,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            "open-openrouter-web" => {
                cx.open_url("https://openrouter.ai/keys");
            }
            "openrouter-key" => {
                self.select_tab(NavTab::Settings, cx);
                self.models.update(cx, |models, cx| {
                    models.open_openrouter_key_editor(window, cx);
                    models.setup_flow = true;
                });
            }
            "selected-key" => {
                self.select_tab(NavTab::Settings, cx);
                self.models.update(cx, |models, cx| {
                    models.open_selected_connection(window, cx);
                    models.setup_flow = true;
                });
            }
            "recommended-models" => {
                self.select_tab(NavTab::Models, cx);
                self.models.update(cx, |models, cx| {
                    models.open_model_home(cx);
                });
            }
            "start-tour" => {
                self.start_system_tour(cx);
                return;
            }
            "ollama" => {
                self.select_tab(NavTab::Models, cx);
                self.models.update(cx, |models, cx| {
                    models.open_ollama_setup(window, cx);
                });
            }
            "openai-url" => {
                self.select_tab(NavTab::Settings, cx);
                self.models.update(cx, |models, cx| {
                    models.open_openai_compatible_editor(window, cx);
                    models.setup_flow = true;
                });
            }
            "choose-model" => {
                self.select_tab(NavTab::Models, cx);
                self.models.update(cx, |models, cx| {
                    models.open_model_catalog(cx);
                });
            }
            _ => {
                self.select_tab(NavTab::Settings, cx);
            }
        }
        self.refresh(cx);
    }

    pub fn start_system_tour(&mut self, cx: &mut Context<Self>) {
        let step = self.state.start_tour();
        self.select_tab(step.target_tab(), cx);
    }

    pub fn next_system_tour_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.state.next_tour_step() {
            self.select_tab(next.target_tab(), cx);
        } else {
            let _ = cx.global::<RuntimeClient>().0.send(Command::Models(
                sovra_core::models::ModelCommand::SetOnboardingCompleted { completed: true },
            ));
            self.select_tab(NavTab::Chat, cx);
            self.composer_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
    }

    pub fn prev_system_tour_step(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.state.prev_tour_step() {
            self.select_tab(prev.target_tab(), cx);
        }
    }

    pub fn end_system_tour(&mut self, cx: &mut Context<Self>) {
        self.state.end_tour();
        let _ = cx.global::<RuntimeClient>().0.send(Command::Models(
            sovra_core::models::ModelCommand::SetOnboardingCompleted { completed: true },
        ));
        self.refresh(cx);
    }

    pub fn toggle_schedule(&mut self, schedule_id: &str, cx: &mut Context<Self>) {
        self.state.toggle_schedule(schedule_id);
        self.refresh(cx);
    }

    pub fn run_schedule_in_chat(
        &mut self,
        schedule_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dispatched) = self.state.trigger_schedule(schedule_id) {
            self.dispatch_runtime_prompt(&dispatched, cx);
            self.persist_active_thread(cx);
        }
        self.refresh(cx);
    }

    fn dispatch_runtime_prompt(&self, prompt: &str, cx: &mut Context<Self>) {
        let session_id = self.state.session_id.clone();
        if let Some(prompt) = self.state.team_prompt(prompt) {
            let _ = cx.global::<RuntimeClient>().0.send(Command::Teams(
                sovra_core::teams::TeamCommand::Run {
                    session_id,
                    prompt: prompt.to_owned(),
                    history: self.state.team_history(),
                },
            ));
            return;
        }
        let command = self
            .state
            .messages
            .iter()
            .rev()
            .find(|message| !message.attachments.is_empty())
            .map_or_else(
                || Command::RunPrompt {
                    context: Some(sovra_core::ConversationContext::before_current_turn(
                        &self.state.messages,
                    )),
                    session_id: session_id.clone(),
                    prompt: prompt.to_owned(),
                },
                |message| Command::RunWithFiles {
                    context: Some(sovra_core::ConversationContext::before_current_turn(
                        &self.state.messages,
                    )),
                    session_id: session_id.clone(),
                    prompt: prompt.to_owned(),
                    attachment_ids: message
                        .attachments
                        .iter()
                        .map(|file| file.id.clone())
                        .collect(),
                },
            );
        let _ = cx.global::<RuntimeClient>().0.send(command);
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = Self::sidebar_rail(window);
        self.inspector_docked = design::WidthClass::of_window(window).fits_side_pane();
        if self.history_compact != compact {
            self.history_compact = compact;
            self.history_overlay_open = false;
        }
        let theme = cx.theme();
        let border = cx.theme().border;
        let muted_foreground = theme.muted_foreground;
        let t = design::tokens(cx);
        self.sync_window_title(window);
        let body_view = self.render_body(cx);
        let title_bar = Self::app_owns_titlebar(window).then(|| self.render_title_bar(cx));
        v_flex()
            .id("sovra-shell")
            .track_focus(&self.focus_handle)
            .tab_stop(false)
            .size_full()
            .sovra_surface(Surface::Canvas, cx)
            .sovra_text(TextRole::Body, cx)
            .children(title_bar)
            .child(body_view)
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
    /// Whether the app must draw the title bar itself. macOS and Windows hand the
    /// titlebar to the app; on Linux the window manager frames the window unless it
    /// granted client-side decorations, and a second, empty bar under the system one
    /// only wastes height.
    fn app_owns_titlebar(window: &Window) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(
                window.window_decorations(),
                gpui_kit::Decorations::Client { .. }
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = window;
            true
        }
    }

    fn sync_window_title(&mut self, window: &mut Window) {
        let title =
            sovra_core::window_title(self.state.route, self.state.active_tab, crate::WINDOW_BRAND);
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    /// A drag strip in the chrome colour that names the open page, centred so it
    /// clears the macOS traffic lights and the Windows/Linux window controls alike.
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let page = match self.state.route {
            ScreenRoute::Login => crate::WINDOW_BRAND,
            ScreenRoute::MainWorkspace => self.state.active_tab.label(),
        };
        let border = cx.theme().border;
        let muted_foreground = cx.theme().muted_foreground;
        TitleBar::new()
            .sovra_surface(Surface::Chrome, cx)
            .border_color(border)
            .child(
                div()
                    .id("title-bar-page")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_center()
                    .truncate()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(muted_foreground)
                    .child(page),
            )
    }

    /// The shell is the window's root, so it alone sizes top-level panes from the
    /// window; nested views measure their own container with `design::width_probe`.
    /// Order of yielding: inspector undocks first (below Large), then the sidebar
    /// becomes an icon rail with an overlay (below Expanded).
    fn sidebar_rail(window: &Window) -> bool {
        design::WidthClass::of_window(window) < design::WidthClass::Expanded
    }

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
                        shell.refresh_transcript(cx);
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
            ScreenRoute::Login => render_login_screen(
                &self.login_email_input,
                &self.login_password_input,
                &self.state.auth.clone(),
                cx,
            )
            .into_any_element(),
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
                    NavTab::Chat => h_flex()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .h_full()
                        .overflow_hidden()
                        .child(render_chat_column(self, cx))
                        .into_any_element(),
                    NavTab::Knowledge => render_knowledge_view(self, cx).into_any_element(),
                    NavTab::Skills => crate::skill_library::render(self, cx).into_any_element(),
                    NavTab::Team => div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.team_view.clone())
                        .into_any_element(),
                    NavTab::Artifacts => div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.artifacts_view.clone())
                        .into_any_element(),
                };

                let sidebar_collapsed = self.history_compact || self.state.history_collapsed;
                let mut body = h_flex()
                    .id("sovra-main-workspace")
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(render_sidebar(
                        &self.state,
                        &self.history_search_input,
                        &self.rename_thread_input,
                        sidebar_collapsed,
                        cx,
                    ))
                    .child(center)
                    .children(
                        (self.history_compact && self.history_overlay_open).then(|| {
                            div().absolute().top_0().left_0().h_full().occlude().child(
                                render_sidebar(
                                    &self.state,
                                    &self.history_search_input,
                                    &self.rename_thread_input,
                                    false,
                                    cx,
                                ),
                            )
                        }),
                    );
                if self.state.active_tab == NavTab::Knowledge && self.inspector_docked {
                    body = body.child(render_inspector(&self.state, &self.search_input, cx));
                }
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .children(crate::onboarding_bar::render_onboarding_bar(
                        &self.state,
                        self.history_compact,
                        cx,
                    ))
                    .child(body)
                    .into_any_element()
            }
        }
    }
}

pub(crate) fn logo(id: &'static str, size: gpui_kit::Rems) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Image)
        .aria_label("Sovra logo")
        .size(size)
        .flex_shrink_0()
        .child(img(crate::assets::SOVRA_LOGO).size_full())
}
