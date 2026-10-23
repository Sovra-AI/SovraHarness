//! The main window's root view: title bar, Login / 3-column Main Chat workspace, and status bar.

use gpui_kit::component::{
    ActiveTheme as _, Theme, ThemeMode, TitleBar, h_flex, input::InputState, v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, Role, StatefulInteractiveElement as _, Styled as _, Task, TestSupportExt as _,
    Window, div, img,
};
use sovra_core::{
    Command, DecisionBackend, ModelTier, NavTab, RuntimeStatus, ScreenRoute, SelectionMode,
    WorkspaceState,
};

use crate::RuntimeClient;
use crate::chat_view::render_chat_column;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::inspector::render_inspector;
use crate::login::render_login_screen;
use crate::my_ais_view::render_my_ais_view;
use crate::schedules_view::render_schedules_view;
use crate::sidebar::render_sidebar;

pub struct Shell {
    state: WorkspaceState,
    composer_input: Entity<InputState>,
    search_input: Entity<InputState>,
    login_email_input: Entity<InputState>,
    login_password_input: Entity<InputState>,
    sim_route_input: Entity<InputState>,
    models: Entity<crate::ModelsView>,
    _events: Task<()>,
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
        let composer_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Message your SOVRA..."));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search anything..."));
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
                let applied = this.update(cx, |shell, cx| {
                    if let sovra_core::Event::Models(event) = &event {
                        shell
                            .models
                            .update(cx, |models, cx| models.apply(event, cx));
                    }
                    if let Some(next_prompt) = shell.state.apply_event(&event) {
                        shell.dispatch_runtime_prompt(next_prompt, cx);
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
            composer_input,
            search_input,
            login_email_input,
            login_password_input,
            sim_route_input,
            models,
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
        if tab == NavTab::Models {
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
        if let Some(dispatched) = self.state.submit_prompt(prompt) {
            self.dispatch_runtime_prompt(dispatched, cx);
        }
        cx.notify();
    }

    pub fn interrupt_active_session(&mut self, cx: &mut Context<Self>) {
        let session_id = self.state.session_id.clone();
        self.state.is_generating = false;
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let muted_foreground = theme.muted_foreground;
        let t = design::tokens(cx);
        let next_mode = if theme.is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        let appearance_label = if theme.is_dark() {
            "Use light appearance"
        } else {
            "Use dark appearance"
        };

        v_flex()
            .id("sovra-shell")
            .size_full()
            .sovra_surface(Surface::Canvas, cx)
            .sovra_text(TextRole::Body, cx)
            .child(render_title_bar(appearance_label, next_mode, cx))
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
                    .child(format!("Runtime: {}", self.state.runtime_status.label()))
                    .child(format!(
                        "Routing: {} · {} · AG-UI (adk-ui v2.2)",
                        self.state.routing.mode.label(),
                        self.state.routing.backend.default_model_id()
                    )),
            )
    }
}

impl Shell {
    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        match self.state.route {
            ScreenRoute::Login => {
                render_login_screen(&self.login_email_input, &self.login_password_input, cx)
                    .into_any_element()
            }
            ScreenRoute::MainWorkspace => {
                let center = match self.state.active_tab {
                    NavTab::Models => div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.models.clone())
                        .into_any_element(),
                    NavTab::MyAis => render_my_ais_view(&self.state, &self.sim_route_input, cx)
                        .into_any_element(),
                    NavTab::Schedules => render_schedules_view(&self.state, cx).into_any_element(),
                    NavTab::Chat | NavTab::Knowledge | NavTab::Settings => {
                        render_chat_column(&self.state, &self.composer_input, cx).into_any_element()
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
                if !matches!(self.state.active_tab, NavTab::Models | NavTab::MyAis) {
                    body = body.child(render_inspector(&self.state, &self.search_input, cx));
                }
                body.into_any_element()
            }
        }
    }
}

fn render_title_bar(
    appearance_label: &'static str,
    next_mode: ThemeMode,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    TitleBar::new().child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .pr(t.space.small)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.space.compact)
                    .child(logo("title-logo", t.layout.logo_small))
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(crate::PRODUCT_NAME),
                    ),
            )
            .child(
                design::primary_button("toggle-appearance", appearance_label, cx)
                    .on_click(move |_, _, cx| Theme::change(next_mode, None, cx)),
            ),
    )
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
